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
    /// The node id of the last config written, to find this fixture's process.
    node_id: std::cell::RefCell<Option<String>>,
}

impl Fixture {
    fn new() -> Result<Self> {
        Ok(Self {
            root: tempfile::tempdir()?,
            suffix: format!("-e2e{}", std::process::id()),
            node_id: std::cell::RefCell::new(None),
        })
    }
    fn base(&self) -> PathBuf {
        self.root.path().canonicalize().expect("canonical root")
    }
    fn config(&self) -> PathBuf {
        self.base().join("cfg dir").join("serve.toml")
    }
    /// The task cannot carry `IROH_TUNNEL_STATE_DIR`, so the real per-user
    /// default is used (the test is opt-in and removes its task on exit).
    fn state(&self) -> PathBuf {
        iroh_tunnel_host::paths::default_state_dir().expect("default state dir")
    }
    /// Write a config with a fresh key; returns the node id it runs as.
    fn write_config(&self, relay: &str) -> Result<String> {
        let key = iroh::SecretKey::generate();
        std::fs::create_dir_all(self.config().parent().unwrap())?;
        std::fs::write(
            self.config(),
            format!(
                "[node]\nsecret_key = {:?}\nrelay_urls = [{relay:?}]\n\n\
                 [[services]]\nname = \"echo\"\nprotocol = \"tcp\"\nport = 9\n",
                iroh_tunnel_host::config::encode_secret_key(&key)
            ),
        )?;
        let node_id = key.public().to_string();
        *self.node_id.borrow_mut() = Some(node_id.clone());
        Ok(node_id)
    }
    /// Wait for the status file of THIS fixture's node. The state directory
    /// is the real per-user one, shared by every test, and a hard `/End`
    /// leaves an earlier test's snapshot behind, so any status file is not
    /// enough.
    fn wait_for_own_status(&self, status_file: &Path, node_id: &str) -> Result<()> {
        wait_for("status file from this test's task", || {
            node_id_of(status_file).as_deref() == Some(node_id)
        })
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(BIN);
        command
            .args(args)
            .env_remove("IROH_TUNNEL_STATE_DIR")
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
    /// Remove the task and wait for its process to exit: `/End` returns before
    /// the process is gone, and the next test would see the role as running.
    fn drop(&mut self) {
        let status_file = self.state().join("serve-status.json");
        let own_pid = self
            .node_id
            .borrow()
            .as_deref()
            .filter(|id| node_id_of(&status_file).as_deref() == Some(id))
            .and_then(|_| pid_of(&status_file));
        let _ = self
            .command(&["serve", "service", "uninstall", "--user"])
            .output();
        if let Some(pid) = own_pid {
            let _ = wait_for("this test's task process to exit", || !pid_alive(pid));
        }
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
fn install_refuses_a_state_directory_the_task_cannot_carry() -> Result<()> {
    let fx = Fixture::new()?;
    fx.write_config("https://relay.invalid")?;
    let config = fx.config();
    let other = fx.base().join("elsewhere");
    let out = fx
        .command(&[
            "serve",
            "service",
            "install",
            "--user",
            "--config",
            config.to_str().unwrap(),
        ])
        .env("IROH_TUNNEL_STATE_DIR", &other)
        .output()?;
    let err = stderr(&out);
    assert_eq!(out.status.code(), Some(2), "{err}");
    assert!(err.contains("IROH_TUNNEL_STATE_DIR"), "{err}");
    assert!(!fx.task_exists(), "nothing may be registered");
    Ok(())
}

#[test]
fn start_without_an_installed_task_names_the_install_command() -> Result<()> {
    // Uses the real per-user lock: `start` checks role ownership first, so a
    // real serve running on a developer machine would change the outcome.
    if std::env::var_os("IROH_TUNNEL_E2E_SERVICE").is_none() {
        return Ok(());
    }
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
    let first_id = fx.write_config(relay.as_ref())?;
    let config = fx.config();
    let config = config.to_str().unwrap();
    let status_file = fx.state().join("serve-status.json");

    // install registers a task for the current user and starts it.
    fx.ok(&["serve", "service", "install", "--user", "--config", config])?;
    assert!(fx.task_exists(), "task not registered");
    fx.wait_for_own_status(&status_file, &first_id)?;

    // status reports the task.
    let status = fx.ok(&["serve", "service", "status", "--user"])?;
    assert!(status.contains("serve is running (pid "), "{status}");
    assert!(status.contains("iroh-tunnel-serve"), "{status}");

    // starting a running role is refused by role ownership, not duplicated.
    let again = fx
        .command(&["serve", "service", "start", "--user"])
        .output()?;
    assert_eq!(again.status.code(), Some(6), "{}", stderr(&again));

    // stop: the process ends. Whether it drained is reported, not gated.
    let pid = pid_of(&status_file).expect("pid in status file");
    eprintln!("DIAG process tree before /End:\n{}", process_tree());
    let stopped_at = Instant::now();
    fx.ok(&["serve", "service", "stop", "--user"])?;
    let exited = wait_for("the stopped task's process to exit", || !pid_alive(pid));
    if exited.is_err() {
        eprintln!("DIAG process tree after /End:\n{}", process_tree());
    }
    exited?;
    let drained = !status_file.exists();
    eprintln!(
        "EVIDENCE schtasks /End: pid {pid} gone after {:?}; status file removed (graceful drain): {drained}",
        stopped_at.elapsed()
    );

    let stopped = fx.ok(&["serve", "service", "status", "--user"])?;
    assert!(stopped.contains("serve is not running"), "{stopped}");

    // A hard /End leaves a stale snapshot: remove it so every later read is
    // from the new process, and require a new pid.
    let _ = std::fs::remove_file(&status_file);

    // start and restart keep the configured identity in a NEW process.
    fx.ok(&["serve", "service", "start", "--user"])?;
    wait_for("status after start", || pid_of(&status_file).is_some())?;
    let started_pid = pid_of(&status_file).unwrap();
    assert_ne!(started_pid, pid, "start must launch a new process");
    assert_eq!(node_id_of(&status_file).unwrap(), first_id);

    fx.ok(&["serve", "service", "restart", "--user"])?;
    wait_for("a new process after restart", || {
        pid_of(&status_file).is_some_and(|p| p != started_pid)
    })?;
    assert!(!pid_alive(started_pid), "restart left the old process");
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

/// `name pid parent` for every iroh-tunnel and cmd process (diagnostics only).
fn process_tree() -> String {
    let script = "Get-CimInstance Win32_Process | Where-Object { $_.Name -match 'iroh-tunnel|cmd' } \
                  | ForEach-Object { \"$($_.Name) pid=$($_.ProcessId) parent=$($_.ParentProcessId) $($_.CommandLine)\" }";
    Command::new("pwsh")
        .args(["-NoProfile", "-Command", script])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

fn pid_of(status_file: &Path) -> Option<u64> {
    let bytes = std::fs::read(status_file).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    json.get("pid")?.as_u64()
}

// ---------------------------------------------------------------------------
// The host service API driven by a caller with no terminal (the
// `manage_service` example), rather than by the CLI.
// ---------------------------------------------------------------------------

/// The `manage_service` example binary, built once with the host's
/// `test-utils` feature (the suffix seam). Asking cargo for the executable
/// path from its JSON messages stays correct under `CARGO_TARGET_DIR`.
fn manage_service_binary() -> &'static Path {
    static BINARY: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BINARY.get_or_init(|| {
        let out = Command::new(env!("CARGO"))
            .args([
                "build",
                "--locked",
                "-p",
                "iroh-tunnel-host",
                "--features",
                "test-utils",
                "--example",
                "manage_service",
                "--message-format=json",
            ])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .stderr(std::process::Stdio::inherit())
            .output()
            .expect("run cargo build");
        assert!(out.status.success(), "building manage_service failed");
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter(|message| message["reason"] == "compiler-artifact")
            .filter(|message| message["target"]["name"] == "manage_service")
            .find_map(|message| message["executable"].as_str().map(PathBuf::from))
            .expect("cargo reported no manage_service executable")
    })
}

impl Fixture {
    /// The example, run as a caller with no terminal; stdout and stderr are
    /// captured. A successful run must leave stderr empty.
    fn caller(&self, args: &[&str]) -> Result<String> {
        let out = Command::new(manage_service_binary())
            .args(["--role", "serve", "--scope", "user"])
            .args(args)
            .env_remove("IROH_TUNNEL_STATE_DIR")
            .env("IROH_TUNNEL_TEST_SERVICE_SUFFIX", &self.suffix)
            .stdin(std::process::Stdio::null())
            .current_dir(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\".into()))
            .output()?;
        anyhow::ensure!(
            out.status.success(),
            "caller {args:?} failed: {}{}",
            stdout(&out),
            stderr(&out)
        );
        anyhow::ensure!(
            out.stderr.is_empty(),
            "the caller leaked output to its stderr: {}",
            stderr(&out)
        );
        Ok(stdout(&out))
    }
}

#[tokio::test]
async fn caller_exit_leaves_user_service_running() -> Result<()> {
    if std::env::var_os("IROH_TUNNEL_E2E_SERVICE").is_none() {
        eprintln!("skipped: set IROH_TUNNEL_E2E_SERVICE=1 to drive the real Task Scheduler");
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new()?;
    let node_id = fx.write_config(relay.as_ref())?;
    let config = fx.config();
    let status_file = fx.state().join("serve-status.json");

    let installed = fx.caller(&[
        "--action",
        "install",
        "--config",
        config.to_str().unwrap(),
        "--exe",
        BIN,
    ])?;
    assert!(installed.contains("Installed"), "{installed}");
    // The caller has exited; the task and the role remain.
    assert!(fx.task_exists(), "task not registered");
    fx.wait_for_own_status(&status_file, &node_id)?;
    let status = fx.ok(&["serve", "status"])?;
    assert!(
        status.contains("serve is running") || status.contains("node_id"),
        "{status}"
    );

    let pid = pid_of(&status_file).expect("pid in status file");
    let removed = fx.caller(&["--action", "uninstall"])?;
    // A scheduled task has no registration file: deleting it is `Disabled`.
    assert!(removed.contains("Disabled"), "{removed}");
    assert!(!fx.task_exists(), "task still registered");
    wait_for("the uninstalled task's process to exit", || !pid_alive(pid))?;
    Ok(())
}

#[tokio::test]
async fn captured_schtasks_status_is_returned() -> Result<()> {
    if std::env::var_os("IROH_TUNNEL_E2E_SERVICE").is_none() {
        eprintln!("skipped: set IROH_TUNNEL_E2E_SERVICE=1 to drive the real Task Scheduler");
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new()?;
    let node_id = fx.write_config(relay.as_ref())?;
    let config = fx.config();
    let status_file = fx.state().join("serve-status.json");

    fx.caller(&[
        "--action",
        "install",
        "--config",
        config.to_str().unwrap(),
        "--exe",
        BIN,
    ])?;
    fx.wait_for_own_status(&status_file, &node_id)?;
    // `caller` pins an empty stderr: the native text comes back in the report.
    let report = fx.caller(&["--action", "status"])?;
    assert!(
        report.contains("program: \"schtasks"),
        "no captured schtasks output in: {report}"
    );
    Ok(())
}
