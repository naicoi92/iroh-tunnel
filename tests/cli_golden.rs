//! CLI output and exit characterization (refactor-codebase Task 2).
//!
//! These tests pin today's exact stdout/stderr and exit codes of the real
//! binary against an isolated `HOME`, state dir and machine-lock dir, so the
//! workspace refactor can prove nothing observable changed. Goldens live in
//! `tests/fixtures/golden/<test_name>.{stdout,stderr}.txt`.
//!
//! Re-bless from the current code with `GOLDEN_BLESS=1 cargo test --test
//! cli_golden`; without it the output is compared. Only dynamic values are
//! normalized (see [`normalize`]); the static text under test is never touched.
//! The serve test uses an in-process local relay, never a public one.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_iroh-tunnel");

/// Isolated environment: everything lives under one temp dir.
struct World {
    root: tempfile::TempDir,
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
        let mut cmd = Command::new(BIN);
        cmd.args(args)
            .env_remove("EDITOR")
            .env_remove("VISUAL")
            .env_remove("IROH_TUNNEL_TEST_INTERACTIVE")
            .env("HOME", self.dir("home"))
            .env("XDG_CONFIG_HOME", self.dir("home"))
            .env("IROH_TUNNEL_TEST_CONFIG_DIR", self.dir("home"))
            .env("IROH_TUNNEL_STATE_DIR", self.dir("state"))
            .env("IROH_TUNNEL_MACHINE_LOCK_DIR", self.dir("machine"))
            .env("NO_COLOR", "1")
            .stdin(Stdio::null());
        cmd
    }
    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().expect("spawn iroh-tunnel")
    }
    /// Write `body` to `<root>/<name>` and return its path.
    fn file(&self, name: &str, body: &str) -> PathBuf {
        let path = self.dir(name);
        std::fs::write(&path, body).expect("write fixture");
        path
    }
    /// Replace the temp root (raw and canonical spellings) with `<TMP>`.
    fn tmp(&self, text: &str) -> String {
        let raw = self.root.path().to_string_lossy().into_owned();
        let canonical = self
            .root
            .path()
            .canonicalize()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| raw.clone());
        text.replace(&canonical, "<TMP>").replace(&raw, "<TMP>")
    }
    fn normalized(&self, text: &str) -> String {
        normalize(&self.tmp(text).replace('\\', "/"))
    }
}

/// Normalize dynamic values by line prefix. Static text is left intact.
///
/// - `Secret key: …` → `<SECRET>`
/// - `NodeId: …`, `New NodeId: …`, `Node ID: …` → `<NODE_ID>`
/// - `Home relay: …` → `<RELAY>`
///
/// Temp paths and the dynamic target port are replaced by the callers.
fn normalize(text: &str) -> String {
    let mut out = String::new();
    for line in text.split_inclusive('\n') {
        let (body, newline) = match line.strip_suffix('\n') {
            Some(body) => (body, "\n"),
            None => (line, ""),
        };
        let replaced = [
            ("Secret key: ", "<SECRET>"),
            ("NodeId: ", "<NODE_ID>"),
            ("New NodeId: ", "<NODE_ID>"),
            ("Node ID: ", "<NODE_ID>"),
            ("Home relay: ", "<RELAY>"),
        ]
        .iter()
        .find_map(|(prefix, token)| body.starts_with(prefix).then(|| format!("{prefix}{token}")));
        out.push_str(replaced.as_deref().unwrap_or(body));
        out.push_str(newline);
    }
    out
}

fn golden_path(test: &str, stream: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/golden")
        .join(format!("{test}.{stream}.txt"))
}

/// Compare `actual` with the golden file, or write it under `GOLDEN_BLESS=1`.
fn golden(test: &str, stream: &str, actual: &str) {
    let path = golden_path(test, stream);
    if std::env::var_os("GOLDEN_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "missing golden {} ({e}); run with GOLDEN_BLESS=1",
            path.display()
        )
    });
    assert_eq!(actual, expected, "golden mismatch: {}", path.display());
}

/// Check exit code, then stdout and stderr goldens.
fn check(world: &World, test: &str, out: &Output, code: i32) {
    let stdout = world.normalized(&String::from_utf8_lossy(&out.stdout));
    let stderr = world.normalized(&String::from_utf8_lossy(&out.stderr));
    assert_eq!(out.status.code(), Some(code), "{test}: {stdout}{stderr}");
    golden(test, "stdout", &stdout);
    golden(test, "stderr", &stderr);
}

const SERVE_CONFIG: &str = "[node]\nsecret_key = \"\"\n\n\
[[services]]\nname = \"web\"\nprotocol = \"tcp\"\nhost = \"127.0.0.1\"\nport = 8080\n\n\
[[services]]\nname = \"a-much-longer-service-name\"\nprotocol = \"udp\"\nhost = \"10.0.0.7\"\nport = 53\n";

const ACCESS_CONFIG: &str = "[node]\nsecret_key = \"\"\n\n\
[[services]]\nname = \"web\"\nnode_id = \"abcdef0123456789abcdef\"\nprotocol = \"tcp\"\nport = 8080\n\n\
[[services]]\nname = \"dns\"\nnode_id = \"0123456789abcdef\"\nprotocol = \"udp\"\nport = 5353\n";

fn listed(test: &str, role: &str, body: &str) {
    let world = World::new();
    let cfg = world.file("cfg.toml", body);
    let out = world.run(&[role, "config", "list", "--config", cfg.to_str().unwrap()]);
    check(&world, test, &out, 0);
}

#[test]
fn config_list_serve_table_is_unchanged() {
    listed(
        "config_list_serve_table_is_unchanged",
        "serve",
        SERVE_CONFIG,
    );
}

#[test]
fn config_list_access_table_is_unchanged() {
    listed(
        "config_list_access_table_is_unchanged",
        "access",
        ACCESS_CONFIG,
    );
}

#[test]
fn config_list_without_services_array_prints_guidance_line() {
    listed(
        "config_list_without_services_array_prints_guidance_line",
        "serve",
        "[node]\nsecret_key = \"\"\n",
    );
}

#[test]
fn config_list_with_empty_services_prints_header_only() {
    listed(
        "config_list_with_empty_services_prints_header_only",
        "serve",
        // An inline empty array is not an array of tables, so today this prints the
        // guidance line; a header-only table is unreachable from a parsed file.
        "services = []\n\n[node]\nsecret_key = \"\"\n",
    );
}

#[test]
fn config_show_redacted_output_is_unchanged() {
    let world = World::new();
    let cfg = world.file(
        "cfg.toml",
        "# operator comment\n[node]\nsecret_key = \"c2VjcmV0LXNlY3JldC1zZWNyZXQtc2VjcmV0LXNlY3JldA==\"\nrelay_urls = [\"https://relay.example\"]\nrelay_token = \"tok3n-secret-value\"\n\n[[services]]\nname = \"web\"\nprotocol = \"tcp\"\nhost = \"127.0.0.1\"\nport = 8080\n",
    );
    let out = world.run(&["serve", "config", "show", "--config", cfg.to_str().unwrap()]);
    check(&world, "config_show_redacted_output_is_unchanged", &out, 0);
}

#[test]
fn config_path_prints_resolved_path() {
    let world = World::new();
    let out = world.run(&["serve", "config", "path"]);
    check(&world, "config_path_prints_resolved_path", &out, 0);
}

#[test]
fn config_set_created_message() {
    let world = World::new();
    let cfg = world.file("cfg.toml", "[node]\nsecret_key = \"\"\n");
    let cfg = cfg.to_str().unwrap();
    let set = world.run(&[
        "serve", "config", "set", "-c", cfg, "-n", "web", "-t", "8080", "-p", "tcp",
    ]);
    check(&world, "config_set_created_message", &set, 0);
}

#[test]
fn config_set_updated_report() {
    let world = World::new();
    let cfg = world.file("cfg.toml", "[node]\nsecret_key = \"\"\n");
    let cfg = cfg.to_str().unwrap();
    let create = world.run(&[
        "serve", "config", "set", "-c", cfg, "-n", "web", "-t", "8080",
    ]);
    assert_eq!(create.status.code(), Some(0));
    let update = world.run(&[
        "serve", "config", "set", "-c", cfg, "-n", "web", "-t", "8081", "-p", "udp",
    ]);
    check(&world, "config_set_updated_report", &update, 0);
}

#[test]
fn config_rm_message() {
    let world = World::new();
    let cfg = world.file("cfg.toml", "[node]\nsecret_key = \"\"\n");
    let cfg = cfg.to_str().unwrap();
    let add = world.run(&[
        "serve", "config", "set", "-c", cfg, "-n", "web", "-t", "8080",
    ]);
    assert_eq!(add.status.code(), Some(0));
    let rm = world.run(&["serve", "config", "rm", "-c", cfg, "-n", "web"]);
    check(&world, "config_rm_message", &rm, 0);
}

#[test]
fn config_check_ok_message() {
    let world = World::new();
    let cfg = world.file("cfg.toml", "[node]\nsecret_key = \"\"\n");
    let out = world.run(&["serve", "config", "check", "-c", cfg.to_str().unwrap()]);
    check(&world, "config_check_ok_message", &out, 0);
}

#[test]
fn keygen_messages_are_unchanged() {
    let world = World::new();
    let cfg = world.dir("cfg.toml");
    let out = world.run(&[
        "serve",
        "config",
        "keygen",
        "--config",
        cfg.to_str().unwrap(),
    ]);
    check(&world, "keygen_messages_are_unchanged", &out, 0);
}

#[test]
fn keygen_print_prints_key_and_node_id_lines() {
    let world = World::new();
    let out = world.run(&["serve", "config", "keygen", "--print"]);
    check(&world, "keygen_print_prints_key_and_node_id_lines", &out, 0);
}

#[test]
fn status_json_forwards_file_bytes() {
    let world = World::new();
    // Non-canonical key order and whitespace: a re-serialization would differ.
    let body = "{ \"services\" : [ { \"protocol\":\"tcp\",  \"name\":\"web\", \"local_addr\":\"127.0.0.1:8080\", \"active_connections\":0 } ],\n  \"pid\": 999999,\n\"started_at\":  1700000000,\n \"node_id\":\"abc\",   \"home_relay\":null,\n\"connections\":[]  }";
    std::fs::write(world.dir("state").join("serve-status.json"), body).unwrap();
    let out = world.run(&["serve", "status", "--json"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(out.stdout, format!("{body}\n").into_bytes());
}

#[test]
fn status_not_running_message_is_unchanged() {
    let world = World::new();
    let out = world.run(&["serve", "status"]);
    check(&world, "status_not_running_message_is_unchanged", &out, 1);
}

/// Stop a child as a real operator would: SIGTERM on unix, kill elsewhere.
fn stop(child: &mut std::process::Child) {
    #[cfg(unix)]
    // SAFETY: kill(2) on our own live child pid.
    unsafe {
        libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
    }
    #[cfg(not(unix))]
    let _ = child.kill();
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().expect("try_wait").is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.wait();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_startup_lines_are_unchanged() {
    let world = World::new();
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await.unwrap();
    let relay = relay.to_string();
    let target = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = target.local_addr().unwrap().port().to_string();
    let target_addr = format!("127.0.0.1:{port}");
    let mut child = world
        .command(&[
            "serve",
            "-n",
            "echo",
            "-t",
            &target_addr,
            "--relay-url",
            &relay,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let mut lines = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let line = rx
            .recv_timeout(left)
            .expect("serve printed no Serving: line");
        let done = line.starts_with("Serving:");
        lines.push(line);
        if done {
            break;
        }
    }
    stop(&mut child);
    let text = lines.join("\n") + "\n";
    // The only dynamic part of the Serving line is the target port.
    let text = world
        .normalized(&text)
        .replace(&format!("127.0.0.1:{port}"), "127.0.0.1:<PORT>");
    golden("serve_startup_lines_are_unchanged", "stdout", &text);
}
