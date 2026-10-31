//! Issue #99 on Windows: the portable half of CFG-LIFECYCLE-01 against the
//! real binary (AC-CFG-05, 06, 07a, 07b, 08, 11). Mode bits, `kill` and shell
//! editors are Unix-only and live in `config_lifecycle.rs`; owner-only ACLs on
//! Windows are not asserted here and stay UNVERIFIED.
#![cfg(windows)]

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn tunnel(args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_iroh-tunnel"));
    cmd.args(args)
        .env_remove("IROH_TUNNEL_TEST_INTERACTIVE")
        .stdin(Stdio::null());
    cmd
}

fn run(args: &[&str]) -> Output {
    tunnel(args).output().expect("spawn iroh-tunnel")
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn p(path: &Path) -> &str {
    path.to_str().unwrap()
}

const TOKEN: &str = "tok3n-secret-value";

fn write_valid(path: &Path) -> String {
    let key = iroh_tunnel::config::encode_secret_key(&iroh::SecretKey::generate());
    std::fs::write(
        path,
        format!(
            "# operator comment\n[node]\nsecret_key = \"{key}\"\nrelay_urls = [\"https://relay.example\"]\nrelay_token = \"{TOKEN}\"\n\n[[services]]\nname = \"web\"\nprotocol = \"tcp\"\nhost = \"127.0.0.1\"\nport = 8080\n"
        ),
    )
    .unwrap();
    key
}

fn secret_key_of(path: &Path) -> String {
    let value: toml::Value = toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    value["node"]["secret_key"].as_str().unwrap().to_string()
}

#[test]
fn invalid_mutation_fails_and_leaves_file_bytes_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("serve.toml");
    std::fs::write(
        &cfg,
        "[[services]]\nname = \"X\"\nprotocol = \"tcp\"\nport = 1\n\n[[services]]\nname = \"Y\"\nprotocol = \"tcp\"\nport = 2\n",
    )
    .unwrap();
    let before = std::fs::read(&cfg).unwrap();
    for args in [
        vec![
            "serve",
            "config",
            "add",
            "--config",
            p(&cfg),
            "--name",
            "ok",
            "--protocol",
            "tcp",
            "--port",
            "3",
        ],
        vec![
            "serve",
            "config",
            "remove",
            "--config",
            p(&cfg),
            "--name",
            "X",
        ],
        vec!["serve", "config", "keygen", "--config", p(&cfg), "--yes"],
    ] {
        let o = run(&args);
        assert_eq!(o.status.code(), Some(2), "{args:?}: {}", err(&o));
        assert_eq!(std::fs::read(&cfg).unwrap(), before, "{args:?}");
    }
}

#[test]
fn concurrent_adds_serialize_without_losing_updates() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("serve.toml");
    write_valid(&cfg);
    let names: Vec<String> = (0..6).map(|i| format!("svc-{i}")).collect();
    let children: Vec<_> = names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let port = (3000 + i).to_string();
            tunnel(&[
                "serve",
                "config",
                "add",
                "--config",
                p(&cfg),
                "--name",
                name,
                "--protocol",
                "tcp",
                "--port",
                &port,
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
        })
        .collect();
    for child in children {
        let o = child.wait_with_output().unwrap();
        assert!(o.status.success(), "{}", err(&o));
    }
    let saved = std::fs::read_to_string(&cfg).unwrap();
    for name in names.iter().map(String::as_str).chain(["web"]) {
        assert!(saved.contains(&format!("name = \"{name}\"")), "lost {name}");
    }
    toml::from_str::<toml::Value>(&saved).expect("file parses");
}

#[test]
fn lock_held_by_another_writer_times_out_with_a_clear_error() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("serve.toml");
    write_valid(&cfg);
    let before = std::fs::read(&cfg).unwrap();
    let lock = File::options()
        .create(true)
        .write(true)
        .truncate(false)
        .open(dir.path().join("serve.toml.lock"))
        .unwrap();
    lock.lock().unwrap();
    let o = tunnel(&[
        "serve",
        "config",
        "add",
        "--config",
        p(&cfg),
        "--name",
        "x",
        "--protocol",
        "tcp",
        "--port",
        "1",
    ])
    .env("IROH_TUNNEL_TEST_CONFIG_LOCK_TIMEOUT_MS", "300")
    .output()
    .unwrap();
    assert_eq!(o.status.code(), Some(2), "{}", err(&o));
    assert!(err(&o).contains("another process"), "{}", err(&o));
    assert_eq!(std::fs::read(&cfg).unwrap(), before);
}

#[test]
fn keygen_rotation_needs_confirmation_and_first_key_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let fresh = dir.path().join("fresh.toml");
    let first = run(&["serve", "config", "keygen", "--config", p(&fresh)]);
    assert!(first.status.success(), "{}", err(&first));

    let cfg = dir.path().join("serve.toml");
    let old = write_valid(&cfg);
    let refused = run(&["serve", "config", "keygen", "--config", p(&cfg)]);
    assert!(!refused.status.success());
    assert!(err(&refused).contains("--yes"), "{}", err(&refused));
    assert_eq!(secret_key_of(&cfg), old);

    let forced = run(&["serve", "config", "keygen", "--config", p(&cfg), "--yes"]);
    assert!(forced.status.success(), "{}", err(&forced));
    assert_ne!(secret_key_of(&cfg), old);
}

#[test]
fn show_redacts_secrets_unless_revealed() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("serve.toml");
    let key = write_valid(&cfg);
    let hidden = out(&run(&["serve", "config", "show", "--config", p(&cfg)]));
    assert!(
        !hidden.contains(&key) && !hidden.contains(TOKEN),
        "{hidden}"
    );
    assert!(hidden.contains("# operator comment"));
    let shown = out(&run(&[
        "serve",
        "config",
        "show",
        "--reveal",
        "--config",
        p(&cfg),
    ]));
    assert!(shown.contains(&key) && shown.contains(TOKEN));
}

#[test]
fn parse_errors_never_echo_secret_lines() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("serve.toml");
    std::fs::write(&cfg, "[node]\nrelay_token = \"leak-me-token\" oops\n").unwrap();
    let o = run(&["serve", "config", "show", "--config", p(&cfg)]);
    assert_eq!(o.status.code(), Some(2));
    assert!(!err(&o).contains("leak-me-token"), "{}", err(&o));
}

#[test]
fn read_commands_without_config_share_one_error_and_create_nothing() {
    let root = tempfile::tempdir().unwrap();
    let base: PathBuf = root.path().canonicalize().unwrap();
    let mut messages = Vec::new();
    for args in [
        vec!["serve", "run"],
        vec!["serve", "config", "list"],
        vec!["serve", "config", "show"],
    ] {
        let o = tunnel(&args)
            .env("IROH_TUNNEL_TEST_CONFIG_DIR", &base)
            .output()
            .unwrap();
        assert_eq!(o.status.code(), Some(2), "{args:?}: {}", err(&o));
        messages.push(err(&o));
    }
    assert!(messages.windows(2).all(|w| w[0] == w[1]), "{messages:?}");
    assert!(messages[0].contains("config add"), "{}", messages[0]);
    assert_eq!(
        std::fs::read_dir(&base).unwrap().count(),
        0,
        "something was created"
    );
}

#[test]
fn every_config_subcommand_honors_config() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("custom.toml");
    for rest in [
        vec!["keygen"],
        vec!["add", "--name", "web", "--protocol", "tcp", "--port", "80"],
        vec!["list"],
        vec!["show"],
        vec!["path"],
        vec!["remove", "--name", "web"],
    ] {
        let mut args = vec!["serve", "config", rest[0], "--config", p(&cfg)];
        args.extend(&rest[1..]);
        let o = run(&args);
        assert!(o.status.success(), "{args:?}: {}", err(&o));
    }
}
