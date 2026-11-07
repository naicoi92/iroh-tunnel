//! Task 13 (windows-system-service): end-to-end tests against a REAL Windows
//! Service, driven through the real binary on the elevated GitHub runner.
//!
//! Opt-in (`IROH_TUNNEL_E2E_SERVICE=1`): every test returns early with a
//! `skipped:` line otherwise, because they register services and create
//! directories under `%ProgramFiles%` / `%ProgramData%`.
//!
//! ## Fixture root
//!
//! Config and state live under `%LOCALAPPDATA%` (the user's own profile
//! volume), not `TEMP`: the product's trust check walks from the volume root,
//! and the runner's TEMP is on `D:\`, which NETWORK SERVICE owns.
//!
//! ## PATH
//!
//! `PATH` is pinned to the copied exe's directory plus the Windows system
//! directories: the product registers the `iroh-tunnel` found on `PATH`, and
//! the runner's `PATH` holds the user-owned `target\debug` build.
//!
//! ## Binary trust (R1)
//!
//! `service install --system` refuses a binary in a user-writable directory,
//! and `CARGO_BIN_EXE_iroh-tunnel` lives in the user-owned `target\`. Each test
//! therefore copies the (self-contained) executable into a fresh
//! `%ProgramFiles%\iroh-tunnel-e2e-<pid>-<test>` created by the elevated test
//! (it inherits Program Files' ACL, owned by Administrators) and runs every CLI
//! command from that copy.
//!
//! ## Machine directory (R2)
//!
//! The `test-utils` build keeps the machine lock directory beside the
//! `HOME`-based user tier, and the LocalSystem service has a different user
//! tier than the runner. The CLI and the service would not share a lock, so
//! each test points both at one directory,
//! `%ProgramData%\iroh-tunnel-e2e-<pid>-<test>`, through
//! `IROH_TUNNEL_MACHINE_LOCK_DIR`. A service gets its environment only from
//! its registry `Environment`: under `test-utils`, `ServiceSpec::render`
//! forwards the installing process's `IROH_TUNNEL_TEST_*` variables and
//! `IROH_TUNNEL_MACHINE_LOCK_DIR` into it, and `set_service_env` edits it
//! afterwards for tests that change a seam later. The service log is
//! `<machine dir>\logs\serve.log`.
//!
//! The `test-utils` feature disables relay TLS verification: only the isolated
//! in-process relay is used, never public relays or real secrets.

#![cfg(windows)]

use anyhow::{ensure, Result};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const BIN: &str = env!("CARGO_BIN_EXE_iroh-tunnel");

fn enabled() -> bool {
    if std::env::var("IROH_TUNNEL_E2E_SERVICE").as_deref() != Ok("1") {
        eprintln!("skipped: set IROH_TUNNEL_E2E_SERVICE=1 (registers a Windows Service)");
        return false;
    }
    true
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Poll `ready` until it holds; 90 s at most.
fn wait_for(what: &str, mut ready: impl FnMut() -> bool) -> Result<()> {
    wait_for_within(what, Duration::from_secs(90), &mut ready)
}

fn wait_for_within(what: &str, limit: Duration, ready: &mut dyn FnMut() -> bool) -> Result<()> {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if ready() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    anyhow::bail!("timeout waiting for {what}")
}

fn status_json(status_file: &Path) -> Option<serde_json::Value> {
    serde_json::from_slice(&std::fs::read(status_file).ok()?).ok()
}

fn node_id_of(status_file: &Path) -> Option<String> {
    status_json(status_file)?
        .get("node_id")?
        .as_str()
        .map(str::to_owned)
}

fn pid_of(status_file: &Path) -> Option<u64> {
    status_json(status_file)?.get("pid")?.as_u64()
}

/// Whether process `pid` exists, asked of the OS.
fn pid_alive(pid: u64) -> bool {
    Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(&format!("\"{pid}\"")))
        .unwrap_or(false)
}

fn pwsh(script: &str) -> Result<Output> {
    Ok(Command::new("pwsh")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()?)
}

fn env_dir(var: &str, fallback: &str) -> PathBuf {
    PathBuf::from(std::env::var(var).unwrap_or_else(|_| fallback.into()))
}

/// The runner's own profile volume (`%LOCALAPPDATA%`), where the trust walk
/// from the volume root succeeds.
fn local_app_data() -> PathBuf {
    PathBuf::from(std::env::var("LOCALAPPDATA").expect("LOCALAPPDATA must be set on Windows"))
}

/// One test's world: the trusted binary copy, machine and state directories, a
/// config, and a guard that removes the service and every directory.
struct Fixture {
    root: tempfile::TempDir,
    suffix: String,
    exe_dir: PathBuf,
    exe: PathBuf,
    machine: PathBuf,
}

impl Fixture {
    fn new(test: &str) -> Result<Self> {
        let tag = format!("{}-{test}", std::process::id());
        let exe_dir =
            env_dir("ProgramFiles", r"C:\Program Files").join(format!("iroh-tunnel-e2e-{tag}"));
        let machine =
            env_dir("ProgramData", r"C:\ProgramData").join(format!("iroh-tunnel-e2e-{tag}"));
        // Created by this elevated process, so they inherit the parents' ACL.
        std::fs::create_dir(&exe_dir)?;
        std::fs::create_dir(&machine)?;
        let exe = exe_dir.join("iroh-tunnel.exe");
        std::fs::copy(BIN, &exe)?;
        let fx = Self {
            root: tempfile::Builder::new()
                .prefix("iroh-tunnel-e2e-")
                .tempdir_in(local_app_data())?,
            suffix: format!("-e2e-{tag}"),
            exe_dir,
            exe,
            machine,
        };
        std::fs::create_dir_all(fx.state())?;
        Ok(fx)
    }

    fn name(&self) -> String {
        format!("iroh-tunnel-serve{}", self.suffix)
    }

    fn base(&self) -> PathBuf {
        let canonical = self.root.path().canonicalize().expect("canonical root");
        PathBuf::from(canonical.to_string_lossy().trim_start_matches(r"\\?\"))
    }

    fn config(&self) -> PathBuf {
        self.base().join("cfg dir").join("serve.toml")
    }

    fn state(&self) -> PathBuf {
        self.base().join("state")
    }

    fn status_file(&self) -> PathBuf {
        self.state().join("serve-status.json")
    }

    fn log_file(&self) -> PathBuf {
        self.machine.join("logs").join("serve.log")
    }

    /// Write a serve config with a fresh key publishing `echo_port`; returns
    /// the node id.
    fn write_config(&self, relay: &str, echo_port: u16) -> Result<String> {
        let key = iroh::SecretKey::generate();
        std::fs::create_dir_all(self.config().parent().unwrap())?;
        std::fs::write(
            self.config(),
            format!(
                "[node]\nsecret_key = {:?}\nrelay_urls = [{relay:?}]\n\n\
                 [[services]]\nname = \"echo\"\nprotocol = \"tcp\"\nhost = \"127.0.0.1\"\nport = {echo_port}\n",
                iroh_tunnel_host::config::encode_secret_key(&key)
            ),
        )?;
        Ok(key.public().to_string())
    }

    /// Deterministic `PATH`: the protected copy first, then system directories.
    fn pinned_path(&self) -> std::ffi::OsString {
        let root =
            PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into()));
        let dirs = [
            self.exe
                .parent()
                .expect("copied exe has a parent")
                .to_path_buf(),
            root.join("System32"),
            root.clone(),
            root.join(r"System32\WindowsPowerShell\v1.0"),
        ];
        std::env::join_paths(dirs).expect("PATH entries contain no ';'")
    }

    /// The copied CLI with the shared test environment.
    fn command_of(&self, program: &Path, args: &[&str]) -> Command {
        let mut command = Command::new(program);
        command
            .args(args)
            .env("PATH", self.pinned_path())
            .env("IROH_TUNNEL_STATE_DIR", self.state())
            .env("IROH_TUNNEL_MACHINE_LOCK_DIR", &self.machine)
            .env("IROH_TUNNEL_TEST_SERVICE_SUFFIX", &self.suffix)
            .env_remove("IROH_TUNNEL_TEST_INTERACTIVE")
            .env_remove("IROH_TUNNEL_TEST_EXIT_AFTER_MS")
            .env_remove("IROH_TUNNEL_TEST_FAIL_BEFORE_READY")
            .stdin(Stdio::null())
            .current_dir(std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\".into()));
        command
    }

    fn command(&self, args: &[&str]) -> Command {
        self.command_of(&self.exe, args)
    }

    /// `serve service <verb> [flags]` from the trusted copy.
    fn service(&self, verb: &str, flags: &[&str]) -> Result<Output> {
        let mut args = vec!["serve", "service", verb];
        args.extend_from_slice(flags);
        Ok(self.command(&args).output()?)
    }

    fn service_ok(&self, verb: &str, flags: &[&str]) -> Result<String> {
        let out = self.service(verb, flags)?;
        ensure!(
            out.status.success(),
            "service {verb} {flags:?} failed: {}{}",
            stdout(&out),
            stderr(&out)
        );
        Ok(stdout(&out))
    }

    fn install_args<'a>(&self, config: &'a str, flags: &[&'a str]) -> Vec<&'a str> {
        let mut args = flags.to_vec();
        args.extend(["--config", config]);
        args
    }

    /// Install, failing with stdout, stderr and the service log on error.
    fn install_ok(&self, flags: &[&str]) -> Result<()> {
        let out = self.install(flags)?;
        ensure!(
            out.status.success(),
            "install: {}{}\nservice log {}: {}",
            stdout(&out),
            stderr(&out),
            self.log_file().display(),
            std::fs::read_to_string(self.log_file()).unwrap_or_else(|e| format!("<{e}>"))
        );
        Ok(())
    }

    fn install(&self, flags: &[&str]) -> Result<Output> {
        let config = self.config();
        let config = config.to_str().unwrap();
        self.service("install", &self.install_args(config, flags))
    }

    fn sc(&self, verb: &str) -> Output {
        Command::new("sc.exe")
            .args([verb, &self.name()])
            .output()
            .expect("run sc.exe")
    }

    fn service_exists(&self) -> bool {
        self.sc("query").status.success()
    }

    fn task_exists(&self) -> bool {
        Command::new("schtasks")
            .args(["/Query", "/TN", &self.name()])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn wait_absent(&self) -> Result<()> {
        wait_for("the service to be gone", || !self.service_exists())
    }

    fn registry_key(&self) -> String {
        format!(r"HKLM\SYSTEM\CurrentControlSet\Services\{}", self.name())
    }

    /// `ImagePath` of the service, as the SCM stores it.
    fn image_path(&self) -> Result<String> {
        let out = Command::new("reg")
            .args(["query", &self.registry_key(), "/v", "ImagePath"])
            .output()?;
        ensure!(
            out.status.success(),
            "reg query ImagePath: {}",
            stderr(&out)
        );
        reg_value(&stdout(&out), "REG_EXPAND_SZ")
            .or_else(|| reg_value(&stdout(&out), "REG_SZ"))
            .ok_or_else(|| anyhow::anyhow!("no ImagePath in {}", stdout(&out)))
    }

    /// Upsert `extra` into the service's `Environment` (REG_MULTI_SZ). The
    /// change applies from the next start.
    fn set_service_env(&self, extra: &[(&str, &str)]) -> Result<()> {
        let out = Command::new("reg")
            .args(["query", &self.registry_key(), "/v", "Environment"])
            .output()?;
        ensure!(
            out.status.success(),
            "reg query Environment: {}",
            stderr(&out)
        );
        let mut entries: Vec<String> = reg_value(&stdout(&out), "REG_MULTI_SZ")
            .unwrap_or_default()
            .split("\\0")
            .filter(|e| !e.is_empty())
            .map(str::to_owned)
            .collect();
        for (key, value) in extra {
            entries.retain(|e| !e.starts_with(&format!("{key}=")));
            entries.push(format!("{key}={value}"));
        }
        let out = Command::new("reg")
            .args([
                "add",
                &self.registry_key(),
                "/v",
                "Environment",
                "/t",
                "REG_MULTI_SZ",
                "/d",
                &entries.join("\\0"),
                "/f",
            ])
            .output()?;
        ensure!(
            out.status.success(),
            "reg add Environment: {}",
            stderr(&out)
        );
        Ok(())
    }

    fn dump(&self) {
        eprintln!("--- sc query ---\n{}", stdout(&self.sc("query")));
        eprintln!(
            "--- {} ---\n{}",
            self.log_file().display(),
            std::fs::read_to_string(self.log_file()).unwrap_or_else(|e| format!("<{e}>"))
        );
    }
}

/// The data of a `reg query` line of the given type.
fn reg_value(output: &str, kind: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let (_, data) = line.split_once(kind)?;
        Some(data.trim().to_owned())
    })
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.dump();
        }
        let pid = pid_of(&self.status_file());
        let _ = self.service("uninstall", &["--system"]);
        if self.service_exists() {
            let _ = self.sc("stop");
            let _ = Command::new("sc.exe")
                .args(["delete", &self.name()])
                .output();
        }
        let _ = self.wait_absent();
        if let Some(pid) = pid {
            let _ = wait_for("the service process to exit", || !pid_alive(pid));
        }
        for dir in [&self.exe_dir, &self.machine] {
            let dir = dir.to_string_lossy().into_owned();
            let _ = pwsh(&format!(
                "if (Test-Path -LiteralPath '{dir}') {{ takeown /f '{dir}' /r /d y | Out-Null; icacls '{dir}' /grant '*S-1-5-32-544:(OI)(CI)F' /t | Out-Null; Remove-Item -LiteralPath '{dir}' -Recurse -Force }}"
            ));
        }
    }
}

/// Wait for the status file of this test's node and return its pid.
fn wait_for_node(fx: &Fixture, node_id: &str) -> Result<u64> {
    wait_for("the status file of this service", || {
        node_id_of(&fx.status_file()).as_deref() == Some(node_id)
    })?;
    pid_of(&fx.status_file()).ok_or_else(|| anyhow::anyhow!("no pid in the status file"))
}

/// Wait for a status file with `node_id` and a pid other than `old`.
fn wait_for_new_pid(fx: &Fixture, node_id: &str, old: u64) -> Result<u64> {
    wait_for("a new service process", || {
        node_id_of(&fx.status_file()).as_deref() == Some(node_id)
            && pid_of(&fx.status_file()).is_some_and(|p| p != old)
    })?;
    Ok(pid_of(&fx.status_file()).unwrap())
}

#[tokio::test]
async fn system_service_lifecycle() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new("lifecycle")?;
    let node_id = fx.write_config(relay.as_ref(), 9)?;

    fx.install_ok(&["--system"])?;
    let pid = wait_for_node(&fx, &node_id)?;
    let status = fx.service_ok("status", &["--system"])?;
    assert!(status.contains("is running"), "{status}");

    // The registered command line is the trusted copy, quoted, then the args.
    let expected = format!(
        "\"{}\" serve run --config \"{}\"",
        fx.exe.display(),
        fx.config().display()
    );
    assert_eq!(
        fx.image_path()?.to_lowercase(),
        expected.to_lowercase(),
        "registered command line must start with the protected copy {}",
        fx.exe.display()
    );

    // stop: the process ends and the status file is removed (the drain).
    fx.service_ok("stop", &["--system"])?;
    wait_for("the process to exit", || !pid_alive(pid))?;
    assert!(
        !fx.status_file().exists(),
        "stop must drain and remove the status file"
    );
    assert!(fx
        .service_ok("status", &["--system"])?
        .contains("not running"));

    // start and restart: a new process with the same identity each time.
    fx.service_ok("start", &["--system"])?;
    let started = wait_for_new_pid(&fx, &node_id, pid)?;
    fx.service_ok("restart", &["--system"])?;
    let restarted = wait_for_new_pid(&fx, &node_id, started)?;
    assert_ne!(restarted, started);

    // uninstall removes the service; status then reports not installed.
    fx.service_ok("uninstall", &["--system"])?;
    fx.wait_absent()?;
    let gone = fx.service("status", &["--system"])?;
    assert_eq!(gone.status.code(), Some(5), "{}", stderr(&gone));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stop_pending_reported_while_draining() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new("drain")?;

    let echo = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let echo_port = echo.local_addr()?.port();
    let echo_task = tokio::spawn(async move {
        while let Ok((mut stream, _)) = echo.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                while let Ok(n) = stream.read(&mut buf).await {
                    if n == 0 || stream.write_all(&buf[..n]).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    let node_id = fx.write_config(relay.as_ref(), echo_port)?;
    fx.install_ok(&["--system"])?;
    wait_for_node(&fx, &node_id)?;

    // An access in this process, tunnelling to the service.
    let access_port = {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        l.local_addr()?.port()
    };
    let access_cfg = fx.base().join("access.toml");
    std::fs::write(
        &access_cfg,
        format!(
            "[node]\nrelay_urls = [{:?}]\n\n[[services]]\nname = \"echo\"\nnode_id = \"{node_id}\"\n\
             protocol = \"tcp\"\nhost = \"127.0.0.1\"\nport = {access_port}\n",
            relay.as_ref() as &str
        ),
    )?;
    let (access_tx, access_rx) = tokio::sync::oneshot::channel::<()>();
    let access = tokio::spawn({
        let path = access_cfg.clone();
        async move {
            iroh_tunnel_host::testing::run_unowned(
                iroh_tunnel_host::Role::Access,
                &path,
                None,
                async move {
                    let _ = access_rx.await;
                },
            )
            .await
        }
    });

    // An active stream through the tunnel, kept open across the stop.
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut client = loop {
        match tokio::net::TcpStream::connect(("127.0.0.1", access_port)).await {
            Ok(s) => break s,
            Err(e) if Instant::now() > deadline => anyhow::bail!("access never listened: {e}"),
            Err(_) => tokio::time::sleep(Duration::from_millis(200)).await,
        }
    };
    client.write_all(b"ping").await?;
    let mut got = [0u8; 4];
    tokio::time::timeout(Duration::from_secs(60), client.read_exact(&mut got)).await??;
    assert_eq!(&got, b"ping");

    let stopper = {
        let mut command = fx.command(&["serve", "service", "stop", "--system"]);
        std::thread::spawn(move || command.output())
    };
    let name = fx.name();
    let observed = tokio::task::spawn_blocking(move || -> Result<Option<Duration>> {
        use windows_service::service::{ServiceAccess, ServiceState};
        use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
        let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
        let service = manager.open_service(&name, ServiceAccess::QUERY_STATUS)?;
        let end = Instant::now() + Duration::from_secs(60);
        while Instant::now() < end {
            let status = service.query_status()?;
            if status.current_state == ServiceState::StopPending {
                return Ok(Some(status.wait_hint));
            }
            if status.current_state == ServiceState::Stopped {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(None)
    })
    .await??;
    let stop = stopper.join().expect("stop thread")?;
    drop(client);
    let _ = access_tx.send(());
    let _ = access.await;
    echo_task.abort();
    ensure!(
        stop.status.success(),
        "stop: {}{}",
        stdout(&stop),
        stderr(&stop)
    );

    let hint = observed.expect("StopPending was never observed while the stream was open");
    assert!(
        hint >= Duration::from_millis(13_000),
        "wait hint {hint:?} is too short for the drain"
    );
    Ok(())
}

#[tokio::test]
async fn elevated_without_flag_targets_system() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new("noflag")?;
    let node_id = fx.write_config(relay.as_ref(), 9)?;

    fx.install_ok(&[])?;
    assert!(fx.service_exists(), "the service must exist");
    assert!(!fx.task_exists(), "no scheduled task may exist");
    let pid = wait_for_node(&fx, &node_id)?;

    fx.service_ok("stop", &[])?;
    wait_for("the process to exit", || !pid_alive(pid))?;
    assert!(fx.service_ok("status", &[])?.contains("not running"));
    Ok(())
}

#[tokio::test]
async fn missing_service_verbs() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new("missing")?;
    fx.write_config(relay.as_ref(), 9)?;

    for verb in ["start", "stop", "status"] {
        let out = fx.service(verb, &["--system"])?;
        let err = stderr(&out);
        assert_eq!(out.status.code(), Some(5), "{verb}: {err}");
        assert!(err.contains("service install --system"), "{verb}: {err}");
    }
    let out = fx.service("uninstall", &["--system"])?;
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));

    // Stopping an already stopped service is fine.
    fx.install_ok(&["--system"])?;
    fx.service_ok("stop", &["--system"])?;
    let again = fx.service("stop", &["--system"])?;
    assert_eq!(again.status.code(), Some(0), "{}", stderr(&again));
    Ok(())
}

#[tokio::test]
async fn scm_marker_outside_scm_is_config_error() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new("marker")?;
    fx.write_config(relay.as_ref(), 9)?;
    let config = fx.config();
    let out = fx
        .command(&["serve", "run", "--config", config.to_str().unwrap()])
        .env("IROH_TUNNEL_SCM", "1")
        .output()?;
    assert_eq!(
        out.status.code(),
        Some(2),
        "{}{}",
        stdout(&out),
        stderr(&out)
    );
    Ok(())
}

#[tokio::test]
async fn crash_is_restarted() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new("crash")?;
    let node_id = fx.write_config(relay.as_ref(), 9)?;
    fx.install_ok(&["--system"])?;
    let first = wait_for_node(&fx, &node_id)?;

    fx.set_service_env(&[("IROH_TUNNEL_TEST_EXIT_AFTER_MS", "1500")])?;
    fx.service_ok("restart", &["--system"])?;
    let crashing = wait_for_new_pid(&fx, &node_id, first)?;
    // The process fails after 1.5 s; recovery restarts it after 5 s.
    let recovered = wait_for_new_pid(&fx, &node_id, crashing)?;
    assert_ne!(recovered, crashing);
    Ok(())
}

#[tokio::test]
async fn rollback_when_start_fails() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new("rollback")?;
    fx.write_config(relay.as_ref(), 9)?;
    let config = fx.config();
    let out = fx
        .command(&[
            "serve",
            "service",
            "install",
            "--system",
            "--config",
            config.to_str().unwrap(),
        ])
        // Forwarded into the service environment by the test-utils install.
        .env("IROH_TUNNEL_TEST_FAIL_BEFORE_READY", "1")
        .output()?;
    assert!(!out.status.success(), "install must fail: {}", stdout(&out));
    fx.wait_absent()?;
    Ok(())
}
