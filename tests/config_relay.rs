//! `config node` relay flags end to end against the real binary.
//! Portable: every child gets an isolated config, state and lock dir.
#![cfg(test)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const IT: &str = env!("CARGO_BIN_EXE_iroh-tunnel");

struct World {
    root: tempfile::TempDir,
}

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
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
    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(IT);
        cmd.args(args)
            .env_remove("EDITOR")
            .env_remove("VISUAL")
            .env("HOME", self.dir("home"))
            .env("XDG_CONFIG_HOME", self.dir("home"))
            .env("IROH_TUNNEL_TEST_CONFIG_DIR", self.dir("home"))
            .env("IROH_TUNNEL_STATE_DIR", self.dir("state"))
            .env("IROH_TUNNEL_MACHINE_LOCK_DIR", self.dir("machine"))
            .env("NO_COLOR", "1");
        cmd
    }
    fn finish(output: std::process::Output) -> Out {
        Out {
            code: output.status.code().expect("exit code"),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }
    fn run(&self, args: &[&str]) -> Out {
        let mut cmd = self.command(args);
        Self::finish(cmd.stdin(Stdio::null()).output().expect("spawn"))
    }
    fn run_stdin(&self, args: &[&str], input: &str) -> Out {
        let mut child = self
            .command(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .expect("write stdin");
        Self::finish(child.wait_with_output().expect("wait"))
    }
    fn file(&self, name: &str, body: &str) -> PathBuf {
        let path = self.dir(name);
        std::fs::write(&path, body).expect("write");
        path
    }
}

fn s(p: &Path) -> &str {
    p.to_str().expect("utf8 path")
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).expect("read")
}

fn all(o: &Out) -> String {
    format!("{}{}", o.stdout, o.stderr)
}

fn assert_runtime_error(o: &Out) {
    assert_eq!(o.code, 2, "{}", o.stderr);
    assert!(o.stderr.starts_with("error:"), "{}", o.stderr);
    assert!(!o.stderr.contains("Usage:"), "{}", o.stderr);
}

#[test]
fn config_node_role_isolation() {
    let w = World::new();
    let o = w.run(&["serve", "config", "node", "-r", "https://a.example"]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    let serve = w.dir("home").join("iroh-tunnel").join("serve.toml");
    assert!(read(&serve).contains("https://a.example"));
    let access = w.run(&["access", "config", "path"]);
    assert_eq!(access.code, 0, "{}", access.stderr);
    assert!(!Path::new(access.stdout.trim()).exists());
}

fn token_case(file_body: &str) -> (World, PathBuf, Out) {
    let w = World::new();
    let cfg = w.dir("cfg.toml");
    let tok = w.file("token.txt", file_body);
    let o = w.run(&[
        "serve",
        "config",
        "node",
        "-c",
        s(&cfg),
        "-r",
        "https://a.example",
        "--relay-token-file",
        s(&tok),
    ]);
    (w, cfg, o)
}

#[test]
fn config_node_token_from_file() {
    let (_w, cfg, o) = token_case("secret-token\n");
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(read(&cfg).contains("relay_token = \"secret-token\""));
}

#[test]
fn config_node_token_file_crlf() {
    let (_w, cfg, o) = token_case("secret-token\r\n");
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(read(&cfg).contains("relay_token = \"secret-token\""));
}

#[test]
fn config_node_token_from_stdin() {
    let w = World::new();
    let cfg = w.dir("cfg.toml");
    let o = w.run_stdin(
        &[
            "serve",
            "config",
            "node",
            "-c",
            s(&cfg),
            "-r",
            "https://a.example",
            "--relay-token-file",
            "-",
        ],
        "secret-token",
    );
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(read(&cfg).contains("relay_token = \"secret-token\""));
}

#[test]
fn config_node_stdin_closed_is_empty_error() {
    let w = World::new();
    let cfg = w.dir("cfg.toml");
    let o = w.run(&[
        "serve",
        "config",
        "node",
        "-c",
        s(&cfg),
        "-r",
        "https://a.example",
        "--relay-token-file",
        "-",
    ]);
    assert_runtime_error(&o);
    assert!(o.stderr.contains("empty"), "{}", o.stderr);
    assert!(!cfg.exists());
}

#[test]
fn config_node_unreadable_token_file() {
    let w = World::new();
    let cfg = w.file("cfg.toml", "[node]\nsecret_key = \"\"\n");
    let before = std::fs::read(&cfg).unwrap();
    let missing = w.dir("missing.txt");
    let o = w.run(&[
        "serve",
        "config",
        "node",
        "-c",
        s(&cfg),
        "-r",
        "https://a.example",
        "--relay-token-file",
        s(&missing),
    ]);
    assert_runtime_error(&o);
    assert!(o.stderr.contains("--relay-token-file"), "{}", o.stderr);
    assert!(o.stderr.contains("missing.txt"), "{}", o.stderr);
    assert_eq!(std::fs::read(&cfg).unwrap(), before);
}

#[test]
fn config_node_empty_token_file() {
    let (_w, cfg, o) = token_case("");
    assert_runtime_error(&o);
    assert!(o.stderr.contains("empty"), "{}", o.stderr);
    assert!(!cfg.exists());
}

#[test]
fn config_node_token_with_whitespace() {
    let w = World::new();
    let cfg = w.dir("cfg.toml");
    let o = w.run(&[
        "serve",
        "config",
        "node",
        "-c",
        s(&cfg),
        "-r",
        "https://a.example",
        "--relay-token",
        "has space",
    ]);
    assert_runtime_error(&o);
    assert!(o.stderr.contains("invalid relay_token"), "{}", o.stderr);
    assert!(!cfg.exists());
}

#[test]
fn config_node_token_file_extra_line() {
    let (_w, cfg, o) = token_case("secret-token\n\n");
    assert_eq!(o.code, 2, "{}", o.stderr);
    assert!(!all(&o).contains("secret-token"), "{}", all(&o));
    assert!(!cfg.exists());
}

#[test]
fn config_node_insecure_url() {
    let w = World::new();
    let cfg = w.file("cfg.toml", "[node]\nsecret_key = \"\"\n");
    let before = std::fs::read(&cfg).unwrap();
    let o = w.run(&[
        "serve",
        "config",
        "node",
        "-c",
        s(&cfg),
        "-r",
        "http://relay.example",
    ]);
    assert_runtime_error(&o);
    assert!(
        o.stderr
            .contains("invalid relay_url 'http://relay.example'"),
        "{}",
        o.stderr
    );
    assert_eq!(std::fs::read(&cfg).unwrap(), before);
}

#[test]
fn config_node_never_prints_token() {
    let w = World::new();
    let cfg = w.dir("cfg.toml");
    let first = w.run(&[
        "serve",
        "config",
        "node",
        "-c",
        s(&cfg),
        "-r",
        "https://a.example",
        "--relay-token",
        "t1",
    ]);
    assert_eq!(first.code, 0, "{}", first.stderr);
    let second = w.run(&[
        "serve",
        "config",
        "node",
        "-c",
        s(&cfg),
        "--relay-token",
        "t2",
    ]);
    assert_eq!(second.code, 0, "{}", second.stderr);
    assert!(read(&cfg).contains("t2"));
    for o in [&first, &second] {
        let text = all(o);
        assert!(!text.contains("t1") && !text.contains("t2"), "{text}");
    }
    assert!(second.stdout.contains("<redacted> -> <redacted> (changed)"));
}
