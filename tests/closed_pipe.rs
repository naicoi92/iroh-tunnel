//! #216/#217: CLI output written into a closed or failing stdout never
//! panics. A consumer closing the pipe early is a normal exit for read-only
//! output; any other write error exits non-zero with an `error:` line.
#![cfg(unix)]
#![cfg(test)]

use std::io::Read;
use std::path::Path;
use std::process::{Command, Output, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_iroh-tunnel");

/// A config with enough services that the output cannot fit a pipe buffer.
fn big_config(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("serve.toml");
    let body: String = (0..3000)
        .map(|i| format!(
                "[[services]]\nname = \"w{i}\"\nprotocol = \"tcp\"\nhost = \"127.0.0.1\"\nport = 80\n"
            ))
        .collect();
    std::fs::write(&path, body).unwrap();
    path
}

fn command(dir: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(BIN);
    cmd.args(args)
        .env("HOME", dir)
        .env("XDG_CONFIG_HOME", dir.join("xdg"))
        .env("IROH_TUNNEL_STATE_DIR", dir.join("state"))
        .stdin(Stdio::null())
        .stderr(Stdio::piped());
    cmd
}

/// Run with stdout connected to a pipe whose reader reads one byte and then
/// closes, like `| head -c1`.
fn run_into_closed_pipe(dir: &Path, args: &[&str]) -> Output {
    let mut child = command(dir, args)
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn iroh-tunnel");
    let mut stdout = child.stdout.take().unwrap();
    let mut first = [0u8; 1];
    let _ = stdout.read(&mut first);
    drop(stdout);
    child.wait_with_output().unwrap()
}

fn assert_clean_stop(out: &Output, what: &str) {
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("panicked"), "{what} panicked: {stderr}");
    assert_eq!(out.status.code(), Some(0), "{what}: {stderr}");
}

#[test]
fn read_only_config_output_stops_cleanly_on_a_closed_pipe() {
    let dir = tempfile::tempdir().unwrap();
    let config = big_config(dir.path());
    let config = config.to_str().unwrap();
    for args in [
        ["serve", "config", "list", "--config", config],
        ["serve", "config", "show", "--config", config],
    ] {
        let out = run_into_closed_pipe(dir.path(), &args);
        assert_clean_stop(&out, &args.join(" "));
    }
}

#[test]
fn small_outputs_stop_cleanly_when_the_reader_is_gone() {
    let dir = tempfile::tempdir().unwrap();
    let config = big_config(dir.path());
    let config = config.to_str().unwrap();
    for args in [
        vec!["serve", "config", "path", "--config", config],
        vec!["serve", "config", "check", "--config", config],
        vec!["serve", "config", "keygen", "--print"],
    ] {
        // The reader is closed before the child writes anything.
        let mut child = command(dir.path(), &args)
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn iroh-tunnel");
        drop(child.stdout.take());
        let out = child.wait_with_output().unwrap();
        assert_clean_stop(&out, &args.join(" "));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn a_full_device_is_an_error_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let config = big_config(dir.path());
    let full = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .unwrap();
    let out = command(
        dir.path(),
        &[
            "serve",
            "config",
            "list",
            "--config",
            config.to_str().unwrap(),
        ],
    )
    .stdout(full)
    .output()
    .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("panicked"), "{stderr}");
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.starts_with("error: failed to write to stdout"),
        "{stderr}"
    );
}

/// The ready line is the supervisor's start signal: when it cannot be
/// written the start fails (exit 1, `error:` line), it is never reported as
/// success and never panics.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unwritable_ready_line_fails_the_start() {
    let dir = tempfile::tempdir().unwrap();
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await.unwrap();
    let relay = relay.to_string();
    let mut child = command(
        dir.path(),
        &[
            "serve",
            "--name",
            "echo",
            "--target",
            "127.0.0.1:9",
            "--relay-url",
            &relay,
        ],
    )
    .env("IROH_TUNNEL_TEST_CONFIG_DIR", dir.path())
    .stdout(Stdio::piped())
    .spawn()
    .expect("spawn iroh-tunnel");
    drop(child.stdout.take());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while child.try_wait().unwrap().is_none() {
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("serve kept running after its ready line failed");
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let out = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("panicked"), "{stderr}");
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains("error: ") && stderr.contains("failed to write to stdout"),
        "{stderr}"
    );
}
