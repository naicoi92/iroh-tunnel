//! #92: run a role in the Windows logged-in session, driven through the real
//! binary and the real Task Scheduler (SM-04, AC-SM-04, story 53).
//!
//! - `system_scope_is_unsupported_on_windows`: `--system` is refused as
//!   unsupported (a Windows Service is deferred) and nothing is registered.
//! - `user_background_task_lifecycle`: install, start, status, stop, restart
//!   and uninstall against the host's Task Scheduler, with the pinned absolute
//!   config and state paths and a working directory unrelated to either.
//!   Opt-in (`IROH_TUNNEL_E2E_SERVICE=1`) because it registers a task; it
//!   uses a unique task-name suffix (the `test-utils` seam) and removes its
//!   task on exit.
//!
//! The `/End` outcome (graceful drain or hard termination) is reported by the
//! test, not asserted: SM-04 records terminate-without-drain as a known
//! limitation, so the measurement is evidence rather than a gate.
//!
//! The `test-utils` feature disables relay TLS verification: only the isolated
//! in-process relay is used, never public relays or real secrets.

#![cfg(windows)]

use anyhow::Result;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_iroh-tunnel");

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn wait_for(what: &str, mut ready: impl FnMut() -> bool) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        if ready() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    anyhow::bail!("timeout waiting for {what}")
}

fn node_id_of(status_file: &Path) -> Option<String> {
    let bytes = std::fs::read(status_file).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    json.get("node_id")?.as_str().map(str::to_owned)
}

struct Fixture {
    root: tempfile::TempDir,
    suffix: String,
}

impl Fixture {
    fn new() -> Result<Self> {
        Ok(Self {
            root: tempfile::tempdir()?,
            suffix: format!("-e2e{}", std::process::id()),
        })
    }
    fn base(&self) -> PathBuf {
        self.root.path().canonicalize().expect("canonical root")
    }
    fn config(&self) -> PathBuf {
        self.base().join("cfg dir").join("serve.toml")
    }
    fn state(&self) -> PathBuf {
        self.base().join("state dir")
    }
    fn write_config(&self, relay: &str) -> Result<()> {
        let key = iroh::SecretKey::generate();
        std::fs::create_dir_all(self.config().parent().unwrap())?;
        std::fs::write(
            self.config(),
            format!(
                "[node]\nsecret_key = {:?}\nrelay_urls = [{relay:?}]\n\n\
                 [[services]]\nname = \"echo\"\nprotocol = \"tcp\"\nport = 9\n",
                iroh_tunnel::config::encode_secret_key(&key)
            ),
        )?;
        Ok(())
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(BIN);
        command
            .args(args)
            .env("IROH_TUNNEL_STATE_DIR", self.state())
            .env("IROH_TUNNEL_TEST_SERVICE_SUFFIX", &self.suffix)
            .env_remove("IROH_TUNNEL_TEST_INTERACTIVE")
            .stdin(std::process::Stdio::null())
            // A directory unrelated to the pinned paths.
            .current_dir(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\".into()));
        command
    }
    fn ok(&self, args: &[&str]) -> Result<String> {
        let out = self.command(args).output()?;
        anyhow::ensure!(
            out.status.success(),
            "{args:?} failed: {}{}",
            stdout(&out),
            stderr(&out)
        );
        Ok(stdout(&out))
    }
    fn task_exists(&self) -> bool {
        Command::new("schtasks")
            .args([
                "/Query",
                "/TN",
                &format!("iroh-tunnel-serve{}", self.suffix),
            ])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self
            .command(&["serve", "service", "uninstall", "--user"])
            .output();
    }
}

#[test]
fn system_scope_is_unsupported_on_windows() -> Result<()> {
    let fx = Fixture::new()?;
    fx.write_config("https://relay.invalid")?;
    let config = fx.config();
    let config = config.to_str().unwrap();

    for args in [
        vec![
            "serve", "service", "install", "--system", "--config", config,
        ],
        vec!["serve", "service", "start", "--system"],
        vec!["serve", "service", "stop", "--system"],
        vec!["serve", "service", "status", "--system"],
    ] {
        let out = fx.command(&args).output()?;
        let err = stderr(&out);
        assert_eq!(out.status.code(), Some(5), "{args:?}: {err}");
        assert!(
            err.contains("not supported on Windows") && err.contains("--user"),
            "{args:?}: {err}"
        );
    }
    assert!(!fx.task_exists(), "nothing may be registered");
    Ok(())
}

#[test]
fn start_without_an_installed_task_names_the_install_command() -> Result<()> {
    let fx = Fixture::new()?;
    for verb in ["start", "stop", "status"] {
        let out = fx.command(&["serve", "service", verb, "--user"]).output()?;
        let err = stderr(&out);
        assert_eq!(out.status.code(), Some(5), "{verb}: {err}");
        assert!(err.contains("service install"), "{verb}: {err}");
    }
    Ok(())
}

#[tokio::test]
async fn user_background_task_lifecycle() -> Result<()> {
    if std::env::var_os("IROH_TUNNEL_E2E_SERVICE").is_none() {
        eprintln!("skipped: set IROH_TUNNEL_E2E_SERVICE=1 to drive the real Task Scheduler");
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new()?;
    fx.write_config(relay.as_ref())?;
    let config = fx.config();
    let config = config.to_str().unwrap();
    let status_file = fx.state().join("serve-status.json");

    // install registers a task for the current user and starts it.
    fx.ok(&["serve", "service", "install", "--user", "--config", config])?;
    assert!(fx.task_exists(), "task not registered");
    wait_for("status file from the task", || {
        node_id_of(&status_file).is_some()
    })?;
    let first_id = node_id_of(&status_file).unwrap();

    // status reports the task.
    let status = fx.ok(&["serve", "service", "status", "--user"])?;
    assert!(status.contains("iroh-tunnel-serve"), "{status}");

    // starting a running role is refused by role ownership, not duplicated.
    let again = fx
        .command(&["serve", "service", "start", "--user"])
        .output()?;
    assert_eq!(again.status.code(), Some(6), "{}", stderr(&again));

    // stop: the process ends. Whether it drained is reported, not gated.
    let pid = pid_of(&status_file).expect("pid in status file");
    let stopped_at = Instant::now();
    fx.ok(&["serve", "service", "stop", "--user"])?;
    wait_for("the stopped task's process to exit", || !pid_alive(pid))?;
    let drained = !status_file.exists();
    eprintln!(
        "EVIDENCE schtasks /End: pid {pid} gone after {:?}; status file removed (graceful drain): {drained}",
        stopped_at.elapsed()
    );

    // start and restart keep the configured identity.
    fx.ok(&["serve", "service", "start", "--user"])?;
    wait_for("status after start", || node_id_of(&status_file).is_some())?;
    assert_eq!(node_id_of(&status_file).unwrap(), first_id);
    fx.ok(&["serve", "service", "restart", "--user"])?;
    wait_for("status after restart", || {
        node_id_of(&status_file).is_some()
    })?;
    assert_eq!(node_id_of(&status_file).unwrap(), first_id);

    // uninstall removes the task and ends the process.
    let pid = pid_of(&status_file).expect("pid in status file");
    fx.ok(&["serve", "service", "uninstall", "--user"])?;
    assert!(!fx.task_exists(), "task still registered");
    wait_for("the uninstalled task's process to exit", || !pid_alive(pid))?;
    Ok(())
}

/// Whether process `pid` exists, asked of the OS (`tasklist` filters by PID
/// and prints the PID only for a live process).
fn pid_alive(pid: u64) -> bool {
    Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(&format!("\"{pid}\"")))
        .unwrap_or(false)
}

fn pid_of(status_file: &Path) -> Option<u64> {
    let bytes = std::fs::read(status_file).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    json.get("pid")?.as_u64()
}
