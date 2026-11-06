//! Usage-error and help end-to-end coverage against the real binaries.

use std::path::PathBuf;
use std::process::{Command, Stdio};

const IT: &str = env!("CARGO_BIN_EXE_iroh-tunnel");
const ITS: &str = env!("CARGO_BIN_EXE_iroh-tunnel-serve");
const ITA: &str = env!("CARGO_BIN_EXE_iroh-tunnel-access");

struct World {
    root: tempfile::TempDir,
}

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Out {
    fn first_paragraph(&self) -> &str {
        self.stderr.split("\n\n").next().unwrap_or("")
    }
    /// Usage error: exit 2, empty stdout, `error:` prefix.
    fn assert_usage(&self) -> &Self {
        assert_eq!(self.code, 2, "stderr: {}", self.stderr);
        assert!(self.stdout.is_empty(), "stdout: {}", self.stdout);
        assert!(self.stderr.starts_with("error:"), "stderr: {}", self.stderr);
        self
    }
}

impl World {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("tempdir");
        for dir in ["home", "state", "machine"] {
            std::fs::create_dir_all(root.path().join(dir)).expect("mkdir");
        }
        Self { root }
    }
    fn dir(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }
    fn run_bin(&self, bin: &str, args: &[&str]) -> Out {
        let output = Command::new(bin)
            .args(args)
            .env_remove("EDITOR")
            .env_remove("VISUAL")
            .env_remove("IROH_TUNNEL_TEST_INTERACTIVE")
            .env("HOME", self.dir("home"))
            .env("XDG_CONFIG_HOME", self.dir("home"))
            .env("IROH_TUNNEL_TEST_CONFIG_DIR", self.dir("home"))
            .env("IROH_TUNNEL_STATE_DIR", self.dir("state"))
            .env("IROH_TUNNEL_MACHINE_LOCK_DIR", self.dir("machine"))
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .output()
            .expect("spawn");
        Out {
            code: output.status.code().expect("exit code"),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }
    fn it(&self, args: &[&str]) -> Out {
        self.run_bin(IT, args)
    }
    fn its(&self, args: &[&str]) -> Out {
        self.run_bin(ITS, args)
    }
    fn ita(&self, args: &[&str]) -> Out {
        self.run_bin(ITA, args)
    }
}

#[test]
fn unknown_option_on_leaf_shows_leaf_help() {
    let o = World::new().it(&["serve", "config", "set", "-n", "web", "--bogus"]);
    o.assert_usage();
    assert!(o.first_paragraph().contains("--bogus"), "{}", o.stderr);
    assert!(o.stderr.contains("Usage: iroh-tunnel serve config set"));
}

#[test]
fn unknown_subcommand_shows_group_help() {
    let o = World::new().it(&["serve", "config", "bogus"]);
    o.assert_usage();
    assert!(o.stderr.contains("Usage: iroh-tunnel serve config"));
    for cmd in ["set", "rm", "check"] {
        assert!(
            o.stderr.lines().any(|l| l.trim_start().starts_with(cmd)),
            "missing {cmd}: {}",
            o.stderr
        );
    }
}

#[test]
fn missing_quick_flag_shows_standalone_help() {
    let o = World::new().its(&["-n", "web"]);
    o.assert_usage();
    assert!(o.first_paragraph().contains("--target"), "{}", o.stderr);
    assert!(o.stderr.contains("Usage: iroh-tunnel-serve"));
}

#[test]
fn invalid_protocol_lists_values() {
    let o = World::new().it(&[
        "serve", "config", "set", "-n", "web", "-t", "3000", "-p", "sctp",
    ]);
    o.assert_usage();
    assert!(
        o.first_paragraph().contains("possible values: tcp, udp"),
        "{}",
        o.stderr
    );
    assert!(o.stderr.contains("Usage: iroh-tunnel serve config set"));
}

#[test]
fn wrong_role_flag_on_set() {
    let w = World::new();
    let o = w.it(&["serve", "config", "set", "-n", "web", "-l", "8080"]);
    o.assert_usage();
    assert!(o.first_paragraph().contains("-l"), "{}", o.stderr);
    assert!(o.stderr.contains("Usage: iroh-tunnel serve config set"));

    let o = w.it(&["access", "config", "set", "-n", "web", "-t", "3000"]);
    o.assert_usage();
    assert!(o.first_paragraph().contains("-t"), "{}", o.stderr);
    assert!(o.stderr.contains("Usage: iroh-tunnel access config set"));
}

#[test]
fn relay_flag_rejected_on_set() {
    let w = World::new();
    for role in ["serve", "access"] {
        let o = w.it(&[role, "config", "set", "-n", "web", "--relay", "x"]);
        o.assert_usage();
        assert!(o.first_paragraph().contains("--relay"), "{}", o.stderr);
        assert!(o
            .stderr
            .contains(&format!("Usage: iroh-tunnel {role} config set")));
    }
}

#[test]
fn rm_without_name_shows_rm_help() {
    let o = World::new().it(&["serve", "config", "rm"]);
    o.assert_usage();
    assert!(o.first_paragraph().contains("--name"), "{}", o.stderr);
    assert!(o.stderr.contains("Usage: iroh-tunnel serve config rm"));
}

#[test]
fn set_without_name_shows_set_help() {
    let o = World::new().it(&["serve", "config", "set", "-t", "3000"]);
    o.assert_usage();
    assert!(o.first_paragraph().contains("--name"), "{}", o.stderr);
    assert!(o.stderr.contains("Usage: iroh-tunnel serve config set"));
}

#[test]
fn bare_invocations_print_help() {
    let w = World::new();
    let cases: Vec<(Out, &str)> = vec![
        (w.it(&[]), "Usage: iroh-tunnel"),
        (w.its(&[]), "Usage: iroh-tunnel-serve"),
        (w.ita(&[]), "Usage: iroh-tunnel-access"),
        (w.it(&["serve"]), "Usage: iroh-tunnel serve"),
        (w.it(&["access"]), "Usage: iroh-tunnel access"),
        (w.ita(&["config"]), "Usage: iroh-tunnel-access config"),
        (
            w.it(&["serve", "service"]),
            "Usage: iroh-tunnel serve service",
        ),
    ];
    for (o, usage) in &cases {
        assert_eq!(o.code, 2, "{usage}: {}", o.stderr);
        assert!(o.stdout.is_empty(), "{usage}: {}", o.stdout);
        assert!(o.stderr.contains(usage), "{usage}: {}", o.stderr);
    }
    let serve = &cases[3].0;
    assert!(!serve.stderr.contains("is required"), "{}", serve.stderr);
}

#[test]
fn malformed_addresses() {
    let w = World::new();
    for addr in [
        ":3000",
        "host:",
        "::1:8080",
        "not-an-address",
        "0",
        "host:70000",
    ] {
        let o = w.it(&["serve", "config", "set", "-n", "web", "-t", addr]);
        o.assert_usage();
        assert!(
            o.first_paragraph().contains("--target"),
            "{addr}: {}",
            o.stderr
        );
        assert!(
            o.stderr.contains("Usage: iroh-tunnel serve config set"),
            "{addr}: {}",
            o.stderr
        );
    }
}

#[test]
fn invalid_color_lists_values() {
    let o = World::new().it(&["--color", "bogus", "serve", "config", "list"]);
    o.assert_usage();
    assert!(
        o.first_paragraph()
            .contains("possible values: auto, always, never"),
        "{}",
        o.stderr
    );
}

#[test]
fn removed_syntax_is_usage_error() {
    let w = World::new();
    for args in [
        &["serve", "config", "add", "-n", "web", "-t", "3000"][..],
        &["serve", "config", "remove", "-n", "web"][..],
        &["serve", "config", "keygen", "random"][..],
    ] {
        let o = w.it(args);
        o.assert_usage();
        assert!(o.stderr.contains("Usage:"), "{args:?}: {}", o.stderr);
    }
}

#[test]
fn keygen_print_with_config_is_usage_error() {
    let w = World::new();
    let cfg = w.dir("home").join("k.toml");
    let cfg_s = cfg.to_str().unwrap();
    let o = w.it(&["serve", "config", "keygen", "--print", "-c", cfg_s]);
    o.assert_usage();
    assert!(o.stderr.contains("Usage: iroh-tunnel serve config keygen"));
    assert!(!cfg.exists());
}

#[test]
fn examples_in_help() {
    let w = World::new();
    for args in [
        &["serve", "config", "set", "--help"][..],
        &["serve", "config", "rm", "--help"][..],
        &["serve", "config", "keygen", "--help"][..],
        &["serve", "--help"][..],
        &["access", "--help"][..],
        &["access", "config", "set", "--help"][..],
    ] {
        let o = w.it(args);
        assert_eq!(o.code, 0, "{args:?}: {}", o.stderr);
        assert!(o.stdout.contains("Examples:"), "{args:?}: {}", o.stdout);
        if args == ["access", "config", "set", "--help"] {
            assert!(o.stdout.contains("-l") && o.stdout.contains("-N"));
        }
    }
}

#[test]
fn protocol_values_in_help() {
    let w = World::new();
    for args in [
        &["serve", "--help"][..],
        &["serve", "config", "set", "--help"][..],
    ] {
        let o = w.it(args);
        assert_eq!(o.code, 0, "{args:?}: {}", o.stderr);
        assert!(
            o.stdout.contains("tcp") && o.stdout.contains("udp"),
            "{args:?}: {}",
            o.stdout
        );
    }
}

#[test]
fn runtime_config_error_has_no_usage() {
    let w = World::new();
    let cfg = w.dir("home").join("broken.toml");
    std::fs::write(&cfg, "[[service]\nname = \n").unwrap();
    let o = w.it(&["serve", "config", "list", "-c", cfg.to_str().unwrap()]);
    assert_eq!(o.code, 2, "{}", o.stderr);
    assert!(o.stderr.contains("line"), "{}", o.stderr);
    assert!(!o.stderr.contains("Usage:"), "{}", o.stderr);
}
