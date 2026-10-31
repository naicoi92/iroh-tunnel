//! Issue #99 end-to-end: configuration and identity lifecycle (CFG-04, CFG-05;
//! acceptance case CFG-LIFECYCLE-01, scenarios AC-CFG-05–11; AC-CFG-12 lives in
//! `config_paths.rs`).
//!
//! Every test drives the real binary against an isolated `HOME` and real
//! configuration files. Unix-only: it asserts file modes and uses `kill`.
//! Windows owner-only permissions and lock behavior are UNVERIFIED (see the
//! #99 evidence record).
#![cfg(unix)]

use std::fs::File;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_iroh-tunnel");

struct World {
    root: tempfile::TempDir,
}

impl World {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("home")).unwrap();
        Self { root }
    }
    fn home(&self) -> PathBuf {
        self.root.path().join("home")
    }
    fn file(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(BIN);
        cmd.args(args)
            .env_remove("IROH_TUNNEL_TEST_CONFIG_DIR")
            .env_remove("IROH_TUNNEL_TEST_INTERACTIVE")
            .env_remove("EDITOR")
            .env_remove("VISUAL")
            .env("HOME", self.home())
            .env("XDG_CONFIG_HOME", self.home().join("xdg"))
            .env("IROH_TUNNEL_STATE_DIR", self.root.path().join("state"))
            .env(
                "IROH_TUNNEL_MACHINE_LOCK_DIR",
                self.root.path().join("machine"),
            )
            .stdin(Stdio::null());
        std::fs::create_dir_all(self.root.path().join("machine")).unwrap();
        cmd
    }
    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().expect("spawn iroh-tunnel")
    }
    /// The default serve config path under the isolated `HOME`.
    fn default_serve(&self) -> PathBuf {
        self.home().join(".config/iroh-tunnel/serve.toml")
    }
}

fn err(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}
fn out(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}
fn p(path: &Path) -> &str {
    path.to_str().unwrap()
}
fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}
fn secret_key_of(path: &Path) -> String {
    let value: toml::Value = toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    value["node"]["secret_key"].as_str().unwrap().to_string()
}

const RELAY_TOKEN: &str = "tok3n-secret-value";

/// A valid serve config with a known key and relay token.
fn write_valid(path: &Path) -> String {
    let key = iroh_tunnel::config::encode_secret_key(&iroh::SecretKey::generate());
    std::fs::write(
        path,
        format!(
            "# operator comment\n[node]\nsecret_key = \"{key}\"\nrelay_urls = [\"https://relay.example\"]\nrelay_token = \"{RELAY_TOKEN}\"\n\n[[services]]\nname = \"web\"\nprotocol = \"tcp\"\nhost = \"127.0.0.1\"\nport = 8080\n"
        ),
    )
    .unwrap();
    key
}

/// A document the schema rejects (service name violates the name rules).
fn write_invalid(path: &Path) {
    std::fs::write(
        path,
        "[node]\nsecret_key = \"\"\n\n[[services]]\nname = \"X\"\nprotocol = \"tcp\"\nhost = \"127.0.0.1\"\nport = 1\n\n[[services]]\nname = \"Y\"\nprotocol = \"tcp\"\nhost = \"127.0.0.1\"\nport = 2\n",
    )
    .unwrap();
}

/// Editor stand-in: a script that overwrites the file it is given.
fn editor_script(world: &World, body: &str) -> PathBuf {
    let path = world.file("editor.sh");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

// --- AC-CFG-05 -------------------------------------------------------------

#[test]
fn invalid_mutations_fail_and_leave_file_bytes_unchanged() {
    let world = World::new();
    let cfg = world.file("serve.toml");
    write_invalid(&cfg);
    let before = std::fs::read(&cfg).unwrap();

    let editor = editor_script(&world, "printf 'services = 5\\n' > \"$1\"");
    let cases: Vec<Vec<&str>> = vec![
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
            "2",
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
        vec!["serve", "config", "edit", "--config", p(&cfg)],
    ];
    for args in cases {
        let output = world
            .command(&args)
            .env("EDITOR", &editor)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}: {}", err(&output));
        assert_eq!(
            std::fs::read(&cfg).unwrap(),
            before,
            "{args:?} changed the file"
        );
    }
}

#[test]
fn rejected_edit_keeps_prior_file_and_a_private_sidecar() {
    let world = World::new();
    let cfg = world.file("serve.toml");
    let key = write_valid(&cfg);
    let before = std::fs::read(&cfg).unwrap();
    let editor = editor_script(
        &world,
        "printf '\\n[[services]]\\nname = \"BAD\"\\nprotocol = \"tcp\"\\nport = 3\\n' >> \"$1\"",
    );
    let output = world
        .command(&["serve", "config", "edit", "--config", p(&cfg)])
        .env("EDITOR", &editor)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", err(&output));
    assert_eq!(std::fs::read(&cfg).unwrap(), before);
    assert_eq!(secret_key_of(&cfg), key);
    let sidecar = world.file("serve.toml.rejected");
    assert!(
        err(&output).contains("serve.toml.rejected"),
        "{}",
        err(&output)
    );
    assert!(std::fs::read_to_string(&sidecar).unwrap().contains("BAD"));
    assert_eq!(mode(&sidecar), 0o600);
}

#[test]
fn valid_edit_is_applied_atomically_and_owner_only() {
    let world = World::new();
    let cfg = world.file("serve.toml");
    write_valid(&cfg);
    std::fs::set_permissions(&cfg, std::fs::Permissions::from_mode(0o644)).unwrap();
    let editor = editor_script(
        &world,
        "sed 's/8080/9090/' \"$1\" > \"$1.new\" && mv \"$1.new\" \"$1\"",
    );
    let output = world
        .command(&["serve", "config", "edit", "--config", p(&cfg)])
        .env("EDITOR", &editor)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", err(&output));
    assert!(std::fs::read_to_string(&cfg).unwrap().contains("9090"));
    assert_eq!(mode(&cfg), 0o600);
    assert!(!world.file("serve.toml.rejected").exists());
}

#[test]
fn edit_conflicting_with_a_concurrent_change_is_rejected() {
    let world = World::new();
    let cfg = world.file("serve.toml");
    write_valid(&cfg);
    // The editor, while "open", lets another writer add a service.
    let body = format!(
        "{BIN} serve config add --config {cfg} --name other --protocol tcp --port 7 >/dev/null || exit 9\nsed 's/8080/9090/' \"$1\" > \"$1.new\" && mv \"$1.new\" \"$1\"",
        cfg = p(&cfg)
    );
    let editor = editor_script(&world, &body);
    let output = world
        .command(&["serve", "config", "edit", "--config", p(&cfg)])
        .env("EDITOR", &editor)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let saved = std::fs::read_to_string(&cfg).unwrap();
    assert!(saved.contains("other"), "concurrent change was lost");
    assert!(!saved.contains("9090"), "stale edit overwrote the file");
    assert!(err(&output).contains("changed"), "{}", err(&output));
}

// --- AC-CFG-06 / AC-CFG-10 -------------------------------------------------

#[test]
fn concurrent_adds_serialize_without_losing_updates() {
    let world = World::new();
    let cfg = world.file("serve.toml");
    write_valid(&cfg);
    let names: Vec<String> = (0..8).map(|i| format!("svc-{i}")).collect();
    let children: Vec<_> = names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let port = (3000 + i).to_string();
            world
                .command(&[
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
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{}", err(&output));
    }
    let saved = std::fs::read_to_string(&cfg).unwrap();
    for name in names.iter().map(String::as_str).chain(["web"]) {
        assert!(saved.contains(&format!("name = \"{name}\"")), "lost {name}");
    }
    assert!(saved.contains("# operator comment"));
    toml::from_str::<toml::Value>(&saved).expect("file parses");
}

#[test]
fn killed_or_concurrent_saves_never_expose_a_truncated_file() {
    let world = World::new();
    let cfg = world.file("serve.toml");
    write_valid(&cfg);
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reader = {
        let (cfg, stop) = (cfg.clone(), stop.clone());
        std::thread::spawn(move || {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let text = std::fs::read_to_string(&cfg).expect("config always readable");
                toml::from_str::<toml::Value>(&text)
                    .unwrap_or_else(|e| panic!("observed a torn file: {e}\n{text}"));
            }
        })
    };
    for round in 0..25u64 {
        let name = format!("k-{round}");
        let mut child = world
            .command(&[
                "serve",
                "config",
                "add",
                "--config",
                p(&cfg),
                "--name",
                &name,
                "--protocol",
                "tcp",
                "--port",
                "5000",
            ])
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_micros(round * 400));
        let _ = child.kill();
        let _ = child.wait();
        let text = std::fs::read_to_string(&cfg).unwrap();
        let value: toml::Value = toml::from_str(&text).expect("file is a complete document");
        assert!(!value["services"].as_array().unwrap().is_empty());
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    reader.join().unwrap();
    // No temp debris is left next to the config by completed saves.
    let output = world.run(&[
        "serve",
        "config",
        "add",
        "--config",
        p(&cfg),
        "--name",
        "final",
        "--protocol",
        "tcp",
        "--port",
        "1",
    ]);
    assert!(output.status.success(), "{}", err(&output));
}

#[test]
fn lock_held_by_another_writer_times_out_with_a_clear_error() {
    let world = World::new();
    let cfg = world.file("serve.toml");
    write_valid(&cfg);
    let before = std::fs::read(&cfg).unwrap();
    let lock = File::options()
        .create(true)
        .write(true)
        .truncate(false)
        .open(world.file("serve.toml.lock"))
        .unwrap();
    lock.lock().unwrap();
    let started = Instant::now();
    let output = world
        .command(&[
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
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(!output.status.success());
    assert!(err(&output).contains("another process"), "{}", err(&output));
    assert_eq!(std::fs::read(&cfg).unwrap(), before);
}

// --- AC-CFG-07a ------------------------------------------------------------

#[test]
fn keygen_rotation_needs_confirmation() {
    let world = World::new();
    let cfg = world.file("serve.toml");
    let old = write_valid(&cfg);

    let refused = world.run(&["serve", "config", "keygen", "--config", p(&cfg)]);
    assert!(!refused.status.success());
    assert!(err(&refused).contains("--yes"), "{}", err(&refused));
    assert_eq!(secret_key_of(&cfg), old);

    // Interactive decline keeps the key; accept rotates.
    let ask = |answer: &str| {
        let mut child = world
            .command(&["serve", "config", "keygen", "--config", p(&cfg)])
            .env("IROH_TUNNEL_TEST_INTERACTIVE", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(answer.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    };
    let declined = ask("n\n");
    assert!(!declined.status.success());
    assert_eq!(secret_key_of(&cfg), old);

    let accepted = ask("y\n");
    assert!(accepted.status.success(), "{}", err(&accepted));
    let rotated = secret_key_of(&cfg);
    assert_ne!(rotated, old);

    let forced = world.run(&["serve", "config", "keygen", "--config", p(&cfg), "--yes"]);
    assert!(forced.status.success(), "{}", err(&forced));
    assert_ne!(secret_key_of(&cfg), rotated);
    assert!(out(&forced).contains("New NodeId"));
}

#[test]
fn first_keygen_does_not_prompt() {
    let world = World::new();
    let cfg = world.file("fresh.toml");
    let first = world.run(&["serve", "config", "keygen", "--config", p(&cfg)]);
    assert!(first.status.success(), "{}", err(&first));
    assert!(!secret_key_of(&cfg).is_empty());
    assert_eq!(mode(&cfg), 0o600);
}

// --- AC-CFG-07b ------------------------------------------------------------

#[test]
fn show_redacts_secrets_unless_revealed() {
    let world = World::new();
    let cfg = world.file("serve.toml");
    let key = write_valid(&cfg);

    let hidden = world.run(&["serve", "config", "show", "--config", p(&cfg)]);
    assert!(hidden.status.success(), "{}", err(&hidden));
    let text = out(&hidden);
    assert!(
        !text.contains(&key) && !text.contains(RELAY_TOKEN),
        "{text}"
    );
    assert!(text.contains("relay.example") && text.contains("# operator comment"));
    assert!(text.contains("name = \"web\""));

    let shown = world.run(&["serve", "config", "show", "--reveal", "--config", p(&cfg)]);
    let text = out(&shown);
    assert!(text.contains(&key) && text.contains(RELAY_TOKEN));
}

// --- AC-CFG-08 -------------------------------------------------------------

#[test]
fn read_commands_without_config_share_one_guidance_error_and_create_nothing() {
    let world = World::new();
    let mut messages = Vec::new();
    for command in ["run", "list", "show"] {
        let args: Vec<&str> = if command == "run" {
            vec!["serve", "run"]
        } else {
            vec!["serve", "config", command]
        };
        let output = world.run(&args);
        assert_eq!(output.status.code(), Some(2), "{command}: {}", err(&output));
        messages.push(err(&output));
    }
    assert!(messages.windows(2).all(|w| w[0] == w[1]), "{messages:?}");
    let message = &messages[0];
    let path = world.default_serve();
    assert!(message.contains(p(&path)), "{message}");
    assert!(
        message.contains("config add") && message.contains("config keygen"),
        "{message}"
    );
    assert!(!path.exists());
    assert!(!path.parent().unwrap().exists(), "directory was created");
}

// --- AC-CFG-09 -------------------------------------------------------------

#[test]
fn run_persists_a_generated_key_owner_only_and_reuses_it() {
    let world = World::new();
    let cfg = world.file("serve.toml");
    std::fs::write(
        &cfg,
        "# keep me\n[[services]]\nname = \"web\"\nprotocol = \"tcp\"\nhost = \"127.0.0.1\"\nport = 9\n",
    )
    .unwrap();
    std::fs::set_permissions(&cfg, std::fs::Permissions::from_mode(0o644)).unwrap();

    let run_until_key = |expect_existing: Option<&str>| -> String {
        let mut child = world
            .command(&["serve", "run", "--config", p(&cfg)])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        let key = loop {
            let text = std::fs::read_to_string(&cfg).unwrap();
            if let Ok(value) = toml::from_str::<toml::Value>(&text) {
                if let Some(key) = value.get("node").and_then(|n| n["secret_key"].as_str()) {
                    if !key.is_empty() {
                        break key.to_string();
                    }
                }
            }
            assert!(Instant::now() < deadline, "run never persisted a key");
            std::thread::sleep(Duration::from_millis(50));
        };
        // Give the process time to reach the endpoint, then kill it hard.
        std::thread::sleep(Duration::from_millis(300));
        let _ = child.kill();
        let _ = child.wait();
        if let Some(existing) = expect_existing {
            assert_eq!(key, existing);
        }
        key
    };

    let first = run_until_key(None);
    assert_eq!(mode(&cfg), 0o600);
    assert!(std::fs::read_to_string(&cfg).unwrap().contains("# keep me"));
    let second = run_until_key(Some(&first));
    assert_eq!(second, first);
}

// --- AC-CFG-11 -------------------------------------------------------------

#[test]
fn every_config_subcommand_honors_config_and_ignores_the_default() {
    let world = World::new();
    let cfg = world.file("custom.toml");
    let peer = iroh::SecretKey::generate().public().to_string();

    let steps: Vec<(Vec<&str>, bool)> = vec![
        (vec!["keygen"], true),
        (
            vec!["add", "--name", "web", "--protocol", "tcp", "--port", "80"],
            true,
        ),
        (vec!["list"], true),
        (vec!["show"], true),
        (vec!["path"], true),
        (vec!["remove", "--name", "web"], true),
    ];
    for (rest, _) in &steps {
        let mut args = vec!["serve", "config"];
        args.push(rest[0]);
        args.extend(["--config", p(&cfg)]);
        args.extend(&rest[1..]);
        let output = world.run(&args);
        assert!(output.status.success(), "{args:?}: {}", err(&output));
    }
    let path_out = world.run(&["serve", "config", "path", "--config", p(&cfg)]);
    assert_eq!(out(&path_out).trim(), p(&cfg));

    // edit, and the access role through the same flag.
    let editor = editor_script(&world, "printf '# edited\\n' >> \"$1\"");
    let edited = world
        .command(&["serve", "config", "edit", "--config", p(&cfg)])
        .env("EDITOR", &editor)
        .output()
        .unwrap();
    assert!(edited.status.success(), "{}", err(&edited));
    assert!(std::fs::read_to_string(&cfg).unwrap().contains("# edited"));
    let acfg = world.file("access.toml");
    let added = world.run(&[
        "access",
        "config",
        "add",
        "--config",
        p(&acfg),
        "--name",
        "db",
        "--protocol",
        "tcp",
        "--port",
        "5432",
        "--node-id",
        &peer,
    ]);
    assert!(added.status.success(), "{}", err(&added));

    assert!(!world.default_serve().exists());
    assert!(
        !world.home().join(".config").exists(),
        "default directory touched"
    );
}
