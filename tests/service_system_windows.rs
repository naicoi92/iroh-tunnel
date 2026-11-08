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

mod common;

use common::windows_service as svc;

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
    exe: PathBuf,
    machine: PathBuf,
    /// Removes `exe_dir` and `machine` after the service is gone; declared
    /// last so it drops after `root`.
    _dirs: DirGuard,
}

/// Owns directories created so far and removes them on drop (also when
/// `Fixture::new` fails half way).
#[derive(Default)]
struct DirGuard {
    dirs: Vec<PathBuf>,
}

impl DirGuard {
    fn create(&mut self, dir: &Path) -> Result<()> {
        // Registered first: a failed create may still leave the directory.
        self.dirs.push(dir.to_path_buf());
        std::fs::create_dir(dir)?;
        Ok(())
    }
}

impl Drop for DirGuard {
    fn drop(&mut self) {
        for dir in &self.dirs {
            remove_dir_retrying(dir);
        }
    }
}

/// Remove `dir` (taking ownership first), retrying for up to 30 s while a
/// just-stopped process may still hold a file open.
fn remove_dir_retrying(dir: &Path) {
    let dir_text = dir.to_string_lossy().into_owned();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let _ = pwsh(&format!(
            "if (Test-Path -LiteralPath '{dir_text}') {{ takeown /f '{dir_text}' /r /d y | Out-Null; icacls '{dir_text}' /grant '*S-1-5-32-544:(OI)(CI)F' /t | Out-Null; Remove-Item -LiteralPath '{dir_text}' -Recurse -Force -ErrorAction SilentlyContinue }}"
        ));
        if !dir.exists() || Instant::now() >= deadline {
            if dir.exists() {
                eprintln!("could not remove {}", dir.display());
            }
            return;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

impl Fixture {
    fn new(test: &str) -> Result<Self> {
        let tag = format!("{}-{test}", std::process::id());
        let exe_dir =
            env_dir("ProgramFiles", r"C:\Program Files").join(format!("iroh-tunnel-e2e-{tag}"));
        let machine =
            env_dir("ProgramData", r"C:\ProgramData").join(format!("iroh-tunnel-e2e-{tag}"));
        // Created by this elevated process, so they inherit the parents' ACL.
        let mut dirs = DirGuard::default();
        dirs.create(&exe_dir)?;
        dirs.create(&machine)?;
        let exe = exe_dir.join("iroh-tunnel.exe");
        std::fs::copy(BIN, &exe)?;
        let fx = Self {
            root: tempfile::Builder::new()
                .prefix("iroh-tunnel-e2e-")
                .tempdir_in(local_app_data())?,
            suffix: format!("-e2e-{tag}"),
            exe,
            machine,
            _dirs: dirs,
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
        self.path_starting_with(self.exe.parent().expect("copied exe has a parent"))
    }

    /// Deterministic `PATH` whose first entry is `first`.
    fn path_starting_with(&self, first: &Path) -> std::ffi::OsString {
        let root =
            PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into()));
        let dirs = [
            first.to_path_buf(),
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
        svc::sc(verb, &self.name())
    }

    fn service_exists(&self) -> bool {
        svc::service_exists(&self.name())
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

    /// `ImagePath` of the service, as the SCM stores it.
    fn image_path(&self) -> Result<String> {
        svc::image_path(&self.name())
    }

    fn set_service_env(&self, extra: &[(&str, &str)]) -> Result<()> {
        svc::set_service_env(&self.name(), extra)
    }

    /// Install `config` (another file of the fixture) as the service target.
    fn install_config(&self, config: &Path) -> Result<Output> {
        let args = self.install_args(config.to_str().unwrap(), &["--system"]);
        self.service("install", &args)
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

impl Drop for Fixture {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.dump();
        }
        let _ = self.service("uninstall", &["--system"]);
        if self.service_exists() {
            let _ = self.sc("stop");
            let _ = wait_for("the service to stop", || {
                svc::service_stopped_or_absent(&self.name())
            });
            let _ = Command::new("sc.exe")
                .args(["delete", &self.name()])
                .output();
        }
        let _ = self.wait_absent();
        // `_dirs` removes the directories after this, with retries.
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

    // The definition: automatic start, LocalSystem, own process, pin record
    // and trusted SID.
    let name = fx.name();
    assert_eq!(
        svc::reg_dword(&name, "Start")?,
        2,
        "Start must be AUTO_START"
    );
    assert_eq!(svc::reg_string(&name, "ObjectName")?, "LocalSystem");
    let kind = svc::reg_dword(&name, "Type")?;
    assert_eq!(kind & 0x10, 0x10, "Type {kind:#x} lacks WIN32_OWN_PROCESS");
    let sid = svc::runner_sid()?;
    let description = svc::reg_string(&name, "Description")?;
    assert!(
        description.contains("iroh-tunnel-pin") && description.contains(&format!("sid={sid}")),
        "pin record: {description}"
    );
    assert!(
        svc::service_env(&name)?.contains(&format!("IROH_TUNNEL_TRUSTED_SID={sid}")),
        "environment: {:?}",
        svc::service_env(&name)?
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

#[tokio::test]
async fn reinstall_replaces_the_definition() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new("reinstall")?;
    let node_id = fx.write_config(relay.as_ref(), 9)?;
    fx.install_ok(&["--system"])?;
    let old_pid = wait_for_node(&fx, &node_id)?;

    // A second config of the same owner, same identity.
    let second = fx.config().with_file_name("serve2.toml");
    std::fs::copy(fx.config(), &second)?;
    let out = fx.install_config(&second)?;
    ensure!(
        out.status.success(),
        "reinstall: {}{}",
        stdout(&out),
        stderr(&out)
    );

    let name = fx.name();
    assert_eq!(svc::service_count(&name)?, 1, "exactly one service");
    let new_pid = wait_for_new_pid(&fx, &node_id, old_pid)?;
    wait_for("the old process to exit", || !pid_alive(old_pid))?;
    assert!(pid_alive(new_pid));
    assert!(fx
        .service_ok("status", &["--system"])?
        .contains("is running"));

    let image = svc::image_path(&name)?.to_lowercase();
    assert!(
        image.contains(&second.to_string_lossy().to_lowercase()),
        "ImagePath must name {}: {image}",
        second.display()
    );
    assert!(!image.contains("serve.toml"), "old config in {image}");
    let description = svc::reg_string(&name, "Description")?;
    let wanted = second.to_string_lossy().replace('\\', "/").to_lowercase();
    assert_eq!(
        svc::pin_config(&description).as_deref(),
        Some(wanted.as_str()),
        "pin record: {description}"
    );
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
    // The failure came from the started service, not an earlier refusal: the
    // product reports the service name with a start failure, and only the
    // running service process creates its log file.
    let err = stderr(&out);
    assert!(
        err.contains(&fx.name())
            && (err.contains("stopped right after start")
                || err.contains("did not become Running")),
        "unexpected failure: {err}"
    );
    assert!(
        fx.log_file().exists(),
        "the service never ran (no {}): {err}",
        fx.log_file().display()
    );
    fx.wait_absent()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Trust refusals (Task 13, commit C)
// ---------------------------------------------------------------------------

/// Everyone (`S-1-1-0`) with modify rights, as `icacls` takes it.
const EVERYONE_MODIFY: &str = "*S-1-1-0:M";
const EVERYONE: &str = "*S-1-1-0";

fn icacls(args: &[&str]) -> Result<()> {
    let out = Command::new("icacls").args(args).output()?;
    ensure!(
        out.status.success(),
        "icacls {args:?}: {}{}",
        stdout(&out),
        stderr(&out)
    );
    Ok(())
}

/// Owner SID and sorted access rules (SID, rights, type, inherited) of
/// `path`. SIDs, not SDDL substrings: the runner is the built-in
/// Administrator, which SDDL abbreviates.
fn acl_state(path: &Path) -> Result<String> {
    let p = path.display();
    let script = format!(
        "$a = Get-Acl -LiteralPath '{p}'; \
         'owner=' + $a.GetOwner([System.Security.Principal.SecurityIdentifier]).Value; \
         $a.GetAccessRules($true, $true, [System.Security.Principal.SecurityIdentifier]) | \
         ForEach-Object {{ '{{0}}|{{1}}|{{2}}|{{3}}' -f $_.IdentityReference.Value, \
         $_.FileSystemRights, $_.AccessControlType, $_.IsInherited }} | Sort-Object"
    );
    let out = pwsh(&script)?;
    ensure!(out.status.success(), "Get-Acl {p}: {}", stderr(&out));
    Ok(stdout(&out).replace("\r\n", "\n"))
}

/// Remove the Everyone grant from `path` when dropped.
struct GrantGuard(PathBuf);

impl GrantGuard {
    fn everyone_modify(path: &Path) -> Result<Self> {
        let guard = Self(path.to_path_buf());
        icacls(&[path.to_str().unwrap(), "/grant", EVERYONE_MODIFY])?;
        Ok(guard)
    }
}

impl Drop for GrantGuard {
    fn drop(&mut self) {
        let _ = icacls(&[self.0.to_str().unwrap(), "/remove", EVERYONE]);
    }
}

/// Lowercased, `\`-separated, without a verbatim prefix.
fn normalised(text: &str) -> String {
    text.replace(r"\\?\", "").replace('/', r"\").to_lowercase()
}

fn log_text(fx: &Fixture) -> String {
    std::fs::read_to_string(fx.log_file()).unwrap_or_default()
}

fn wait_for_log(fx: &Fixture, needle: &str) -> Result<()> {
    wait_for(&format!("{needle:?} in the service log"), || {
        normalised(&log_text(fx)).contains(&normalised(needle))
    })
}

fn sc_running(fx: &Fixture) -> bool {
    stdout(&fx.sc("query")).contains("RUNNING")
}

/// How long a refused start is watched after its first failure: the SCM
/// recovery action restarts the service after 5 s, and that start must be
/// refused too.
const RECOVERY_WINDOW: Duration = Duration::from_secs(8);

/// Start the stopped service through the SCM itself (no CLI precheck) and
/// watch it from before the request. The service must never report Running,
/// must end Stopped with a fresh service-specific exit code (the prior clean
/// stop left `Win32(0)`), and the recovery restart must not reach Running
/// either. Returns that exit code. Query errors fail the test.
fn refused_start(fx: &Fixture) -> Result<u32> {
    use windows_service::service::{ServiceAccess, ServiceExitCode, ServiceState};
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

    fn open(name: &str, access: ServiceAccess) -> Result<windows_service::service::Service> {
        let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
        Ok(manager.open_service(name, access)?)
    }
    let name = fx.name();
    let before = open(&name, ServiceAccess::QUERY_STATUS)?.query_status()?;
    ensure!(
        before.current_state == ServiceState::Stopped
            && before.exit_code == ServiceExitCode::Win32(0),
        "the service must be cleanly stopped first: {before:?}"
    );

    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let observer = std::thread::spawn({
        let name = name.clone();
        move || -> Result<u32> {
            let watched = open(&name, ServiceAccess::QUERY_STATUS)?;
            let mut failed: Option<(u32, Instant)> = None;
            let deadline = Instant::now() + Duration::from_secs(60);
            ready_tx.send(()).ok();
            loop {
                let status = watched.query_status()?;
                ensure!(
                    status.current_state != ServiceState::Running,
                    "the service reported Running: {status:?}"
                );
                match (failed, status.exit_code) {
                    (None, ServiceExitCode::ServiceSpecific(code))
                        if status.current_state == ServiceState::Stopped =>
                    {
                        failed = Some((code, Instant::now()));
                    }
                    (Some((code, at)), _) if at.elapsed() >= RECOVERY_WINDOW => return Ok(code),
                    _ => {}
                }
                ensure!(
                    Instant::now() < deadline,
                    "no service-specific failure within 60 s: {status:?}"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    });
    ready_rx.recv()?;
    // A refused start reports Stopped from the service main, so the request
    // itself normally succeeds; the observer's exit code is the evidence.
    let started = open(&name, ServiceAccess::START)?.start::<&str>(&[]);
    observer
        .join()
        .map_err(|_| anyhow::anyhow!("observer panicked"))?
        .map_err(|e| e.context(format!("start request: {started:?}")))
}

/// Stop the running service cleanly, so a refused start can be observed.
fn stop_cleanly(fx: &Fixture) -> Result<()> {
    fx.service_ok("stop", &["--system"])?;
    wait_for("the service to stop", || {
        svc::service_stopped_or_absent(&fx.name())
    })
}

/// Retry `start --system` until the service is Running: after a refusal the
/// SCM recovery action may start it on its own between two attempts.
fn start_until_running(fx: &Fixture) -> Result<()> {
    wait_for("the service to run after start", || {
        if !sc_running(fx) {
            let _ = fx.service("start", &["--system"]);
        }
        sc_running(fx)
    })
}

fn install_with_env(fx: &Fixture, envs: &[(&str, &str)]) -> Result<Output> {
    let config = fx.config();
    let mut command = fx.command(&[
        "serve",
        "service",
        "install",
        "--system",
        "--config",
        config.to_str().unwrap(),
    ]);
    for (key, value) in envs {
        command.env(key, value);
    }
    Ok(command.output()?)
}

fn assert_exit(out: &Output, code: i32) {
    assert_eq!(
        out.status.code(),
        Some(code),
        "{}{}",
        stdout(out),
        stderr(out)
    );
}

#[tokio::test]
async fn unsafe_config_refused_then_fixed() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new("unsafecfg")?;
    let node_id = fx.write_config(relay.as_ref(), 9)?;
    fx.install_ok(&["--system"])?;
    wait_for_node(&fx, &node_id)?;

    stop_cleanly(&fx)?;

    let grant = GrantGuard::everyone_modify(&fx.config())?;
    // icacls rewrites the DACL on grant, so compare against this state.
    let granted = acl_state(&fx.config())?;
    assert_eq!(refused_start(&fx)?, 3, "a permission error");
    wait_for_log(&fx, "permission error")?;
    wait_for_log(&fx, fx.config().to_str().unwrap())?;
    assert_eq!(
        acl_state(&fx.config())?,
        granted,
        "the product must not repair or alter the config ACL while refusing"
    );

    // The user fixes the file; the product then keeps it exactly as set.
    icacls(&[fx.config().to_str().unwrap(), "/remove", EVERYONE])?;
    drop(grant);
    let fixed = acl_state(&fx.config())?;
    start_until_running(&fx)?;
    wait_for_node(&fx, &node_id)?;
    assert_eq!(
        acl_state(&fx.config())?,
        fixed,
        "owner and DACL stay as the user set them"
    );
    Ok(())
}

/// Removes a junction (not its target) when dropped.
struct JunctionGuard(PathBuf);

impl Drop for JunctionGuard {
    fn drop(&mut self) {
        let _ = Command::new("cmd")
            .args(["/c", "rmdir"])
            .arg(&self.0)
            .output();
    }
}

#[tokio::test]
async fn junction_in_config_path_refused() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new("junction")?;
    fx.write_config(relay.as_ref(), 9)?;
    let real = fx.base().join("real");
    std::fs::create_dir(&real)?;
    std::fs::copy(fx.config(), real.join("serve.toml"))?;
    let link = fx.base().join("link");
    let _guard = JunctionGuard(link.clone());
    let made = Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&link)
        .arg(&real)
        .output()?;
    ensure!(made.status.success(), "mklink: {}", stdout(&made));

    let out = fx.install_config(&link.join("serve.toml"))?;
    assert_exit(&out, 3);
    assert!(stderr(&out).contains("reparse point"), "{}", stderr(&out));
    assert!(!fx.service_exists(), "no service may be registered");
    Ok(())
}

#[tokio::test]
async fn invalid_config_never_runs() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new("invalidcfg")?;
    let node_id = fx.write_config(relay.as_ref(), 9)?;
    fx.install_ok(&["--system"])?;
    wait_for_node(&fx, &node_id)?;
    stop_cleanly(&fx)?;
    let before = acl_state(&fx.config())?;

    // Overwriting in place keeps the owner and the DACL.
    std::fs::write(fx.config(), "[node\nthis is = = not toml\n")?;
    // Any service-specific failure: a TOML syntax error is not classified as
    // a config error (exit 1) on every platform, which the spec leaves open.
    assert_ne!(refused_start(&fx)?, 0, "a failure");
    wait_for_log(&fx, "failed to parse")?;
    assert_eq!(acl_state(&fx.config())?, before, "owner and DACL unchanged");
    Ok(())
}

#[tokio::test]
async fn empty_key_never_written() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new("emptykey")?;
    let node_id = fx.write_config(relay.as_ref(), 9)?;
    // Install is allowed to refuse an empty key, so empty it afterwards.
    fx.install_ok(&["--system"])?;
    wait_for_node(&fx, &node_id)?;
    let text = std::fs::read_to_string(fx.config())?;
    let emptied = text
        .lines()
        .map(|l| {
            if l.starts_with("secret_key") {
                "secret_key = \"\""
            } else {
                l
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert_ne!(emptied, text, "the key line must have been replaced");
    std::fs::write(fx.config(), &emptied)?;
    let bytes = std::fs::read(fx.config())?;

    stop_cleanly(&fx)?;
    assert_eq!(refused_start(&fx)?, 2, "a config error");
    wait_for_log(&fx, "config keygen")?;
    assert_eq!(
        std::fs::read(fx.config())?,
        bytes,
        "the service must never write a generated key"
    );
    Ok(())
}

#[tokio::test]
async fn pinless_service_refused() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new("pinless")?;
    let node_id = fx.write_config(relay.as_ref(), 9)?;
    fx.install_ok(&["--system"])?;
    wait_for_node(&fx, &node_id)?;

    stop_cleanly(&fx)?;
    svc::remove_service_env(&fx.name(), "IROH_TUNNEL_TRUSTED_SID")?;
    assert_eq!(refused_start(&fx)?, 3, "a permission error");
    wait_for_log(&fx, "service install --system")?;
    Ok(())
}

const VERIFY_MACHINE_DIR: (&str, &str) = ("IROH_TUNNEL_TEST_VERIFY_MACHINE_DIR", "1");

/// Give the machine directory a known-trusted DACL (SYSTEM and
/// Administrators only), so a later Everyone grant is the only difference.
fn protect_machine_dir(fx: &Fixture) -> Result<()> {
    icacls(&[
        fx.machine.to_str().unwrap(),
        "/inheritance:r",
        "/grant:r",
        "*S-1-5-18:(OI)(CI)F",
        "*S-1-5-32-544:(OI)(CI)F",
    ])
}

#[tokio::test]
async fn untrusted_machine_dir_refused() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;

    // (a) A directory made by someone else, writable by Everyone, at install.
    {
        let fx = Fixture::new("machdir-a")?;
        fx.write_config(relay.as_ref(), 9)?;
        protect_machine_dir(&fx)?;
        let _grant = GrantGuard::everyone_modify(&fx.machine)?;
        let rules = acl_state(&fx.machine)?;
        let out = install_with_env(&fx, &[VERIFY_MACHINE_DIR])?;
        assert_exit(&out, 3);
        assert!(
            normalised(&stderr(&out)).contains(&normalised(fx.machine.to_str().unwrap())),
            "the refusal must name {}: {}",
            fx.machine.display(),
            stderr(&out)
        );
        assert!(!fx.service_exists(), "no service may be registered");
        assert_eq!(acl_state(&fx.machine)?, rules, "never repaired");
    }

    // (b) A trusted directory that becomes untrusted after install.
    let fx = Fixture::new("machdir-b")?;
    let node_id = fx.write_config(relay.as_ref(), 9)?;
    protect_machine_dir(&fx)?;
    let out = install_with_env(&fx, &[VERIFY_MACHINE_DIR])?;
    ensure!(
        out.status.success(),
        "install: {}{}",
        stdout(&out),
        stderr(&out)
    );
    // The service checks the directory too; the CLI below does not.
    fx.set_service_env(&[VERIFY_MACHINE_DIR])?;
    wait_for_node(&fx, &node_id)?;
    stop_cleanly(&fx)?;
    wait_for("the status file to go", || !fx.status_file().exists())?;
    let log_len = std::fs::metadata(fx.log_file())?.len();

    let _grant = GrantGuard::everyone_modify(&fx.machine)?;
    let rules = acl_state(&fx.machine)?;
    // A fresh SCM start that the service itself refuses.
    assert_eq!(refused_start(&fx)?, 3, "a permission error");
    // Its log lives in the untrusted directory, so the service writes none.
    assert_eq!(
        std::fs::metadata(fx.log_file())?.len(),
        log_len,
        "no log may be written into an untrusted directory"
    );
    assert!(fx.machine.join("logs").exists());
    assert_eq!(acl_state(&fx.machine)?, rules, "never repaired");
    Ok(())
}

#[tokio::test]
async fn binary_check_at_install_and_start() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;

    // (a) The `iroh-tunnel` first on PATH sits in a directory the user can
    // write: that is also "binary writable by the pinned user".
    {
        let fx = Fixture::new("binary-a")?;
        fx.write_config(relay.as_ref(), 9)?;
        let user_bin = fx.base().join("userbin");
        std::fs::create_dir(&user_bin)?;
        let copy = user_bin.join("iroh-tunnel.exe");
        std::fs::copy(BIN, &copy)?;
        let config = fx.config();
        let out = fx
            .command_of(
                &copy,
                &[
                    "serve",
                    "service",
                    "install",
                    "--system",
                    "--config",
                    config.to_str().unwrap(),
                ],
            )
            .env("PATH", fx.path_starting_with(&user_bin))
            .output()?;
        assert_exit(&out, 3);
        assert!(
            normalised(&stderr(&out)).contains(&normalised(copy.to_str().unwrap())),
            "the refusal must name {}: {}",
            copy.display(),
            stderr(&out)
        );
        assert!(!fx.service_exists(), "no service may be registered");
    }

    // (b) A protected binary that becomes writable after install.
    let fx = Fixture::new("binary-b")?;
    let node_id = fx.write_config(relay.as_ref(), 9)?;
    fx.install_ok(&["--system"])?;
    wait_for_node(&fx, &node_id)?;
    fx.service_ok("stop", &["--system"])?;
    let _grant = GrantGuard::everyone_modify(&fx.exe)?;
    let out = fx.service("start", &["--system"])?;
    assert_exit(&out, 3);
    assert!(
        normalised(&stderr(&out)).contains(&normalised(fx.exe.to_str().unwrap())),
        "the refusal must name {}: {}",
        fx.exe.display(),
        stderr(&out)
    );
    assert!(svc::service_stopped_or_absent(&fx.name()));
    Ok(())
}

// ---------------------------------------------------------------------------
// Identity, scope and concurrency (Task 13, commit D)
// ---------------------------------------------------------------------------

/// A throwaway local account, deleted on drop.
struct LocalUser {
    name: String,
    sid: String,
}

impl LocalUser {
    /// `itb<pid><tag>` with a generated 14-character password (lower, upper,
    /// digit, symbol). 14 characters keep `net user` from asking about
    /// pre-Windows 2000 compatibility.
    fn create(tag: &str) -> Result<Self> {
        let name = format!("itb{}{tag}", std::process::id());
        let key = iroh::SecretKey::generate();
        let hex: String = key.to_bytes()[..5]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let password = format!("{hex}Aa1!");
        let out = Command::new("net")
            .args(["user", &name, &password, "/add"])
            .stdin(Stdio::null())
            .output()?;
        ensure!(
            out.status.success(),
            "net user /add: {}{}",
            stdout(&out),
            stderr(&out)
        );
        let mut user = Self {
            name,
            sid: String::new(),
        };
        let out = pwsh(&format!(
            "(New-Object System.Security.Principal.NTAccount('{}')).Translate(\
             [System.Security.Principal.SecurityIdentifier]).Value",
            user.name
        ))?;
        ensure!(
            out.status.success(),
            "SID of {}: {}",
            user.name,
            stderr(&out)
        );
        user.sid = stdout(&out).trim().to_owned();
        ensure!(user.sid.starts_with("S-1-5-21-"), "SID {:?}", user.sid);
        Ok(user)
    }
}

impl Drop for LocalUser {
    fn drop(&mut self) {
        let _ = Command::new("net")
            .args(["user", &self.name, "/delete"])
            .stdin(Stdio::null())
            .output();
    }
}

/// The runner's account name (without the machine part), lowercased.
fn runner_name() -> String {
    std::env::var("USERNAME")
        .expect("USERNAME must be set on Windows")
        .to_lowercase()
}

/// Run `command` with `input` on stdin.
fn run_with_input(mut command: Command, input: &str) -> Result<Output> {
    use std::io::Write as _;
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(input.as_bytes())?;
    Ok(child.wait_with_output()?)
}

/// An interactive reinstall of the fixture's config answering `answer`.
fn reinstall_answering(fx: &Fixture, config: &Path, answer: &str) -> Result<Output> {
    let mut command = fx.command(&fx_install_args(config));
    command.env("IROH_TUNNEL_TEST_INTERACTIVE", "1");
    run_with_input(command, answer)
}

fn fx_install_args(config: &Path) -> Vec<&str> {
    vec![
        "serve",
        "service",
        "install",
        "--system",
        "--config",
        config.to_str().unwrap(),
    ]
}

#[tokio::test]
async fn config_owned_by_other_user() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new("otheruser")?;
    fx.write_config(relay.as_ref(), 9)?;
    let bob = LocalUser::create("o")?;
    let dir = fx.config().parent().unwrap().to_path_buf();
    icacls(&[dir.to_str().unwrap(), "/setowner", &bob.name])?;

    let out = fx.install(&["--system"])?;
    assert_exit(&out, 2);
    let err = stderr(&out).to_lowercase();
    assert!(
        err.contains(&bob.name.to_lowercase()) && err.contains(&runner_name()),
        "the refusal must name both accounts: {err}"
    );
    assert!(!fx.service_exists(), "nothing may be registered");
    Ok(())
}

#[tokio::test]
async fn retarget_declined_and_approved() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new("retarget")?;
    let node_id = fx.write_config(relay.as_ref(), 9)?;
    fx.install_ok(&["--system"])?;
    wait_for_node(&fx, &node_id)?;
    let name = fx.name();
    let runner = svc::runner_sid()?;

    // The service now looks as if bob had installed it.
    let bob = LocalUser::create("r")?;
    let ours = svc::reg_string(&name, "Description")?;
    ensure!(
        ours.contains(&format!("sid={runner}")),
        "pin record: {ours}"
    );
    let bobs = ours.replace(&format!("sid={runner}"), &format!("sid={}", bob.sid));
    svc::set_description(&name, &bobs)?;

    // The runner's own config, at another path: the retarget target.
    let mine = fx.config().with_file_name("mine.toml");
    std::fs::copy(fx.config(), &mine)?;
    let definition = |name: &str| -> Result<(String, String, Vec<String>)> {
        Ok((
            svc::image_path(name)?,
            svc::reg_string(name, "Description")?,
            svc::service_env(name)?,
        ))
    };
    let files = || -> Result<(Vec<u8>, Vec<u8>)> {
        Ok((std::fs::read(fx.config())?, std::fs::read(&mine)?))
    };
    let (before, bytes) = (definition(&name)?, files()?);

    // Declined: exit 5, the prompt names bob, nothing changes.
    let out = reinstall_answering(&fx, &mine, "n\n")?;
    assert_exit(&out, 5);
    let err = stderr(&out);
    assert!(err.contains("Retarget"), "{err}");
    assert!(
        err.to_lowercase().contains(&bob.name.to_lowercase()),
        "the prompt must name {}: {err}",
        bob.name
    );
    assert_eq!(definition(&name)?, before, "definition and pin unchanged");
    assert_eq!(files()?, bytes, "both configs unchanged");

    // Approved: the pin is the runner and the new config; both configs kept.
    let out = reinstall_answering(&fx, &mine, "y\n")?;
    ensure!(
        out.status.success(),
        "approved retarget: {}{}",
        stdout(&out),
        stderr(&out)
    );
    let pinned = svc::reg_string(&name, "Description")?;
    assert!(pinned.contains(&format!("sid={runner}")), "{pinned}");
    assert!(!pinned.contains(&bob.sid), "{pinned}");
    let wanted = mine.to_string_lossy().replace('\\', "/").to_lowercase();
    assert_eq!(
        svc::pin_config(&pinned).as_deref(),
        Some(wanted.as_str()),
        "pin record: {pinned}"
    );
    assert!(
        svc::image_path(&name)?
            .to_lowercase()
            .contains(&mine.to_string_lossy().to_lowercase()),
        "ImagePath must name {}",
        mine.display()
    );
    assert_eq!(files()?, bytes, "both configs kept unchanged");

    // Same user again: no prompt (stdin is not a terminal here).
    fx.install_ok(&["--system"])?;
    Ok(())
}

#[tokio::test]
async fn config_dir_owned_by_administrators() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new("admindir")?;
    let node_id = fx.write_config(relay.as_ref(), 9)?;
    let dir = fx.config().parent().unwrap().to_path_buf();
    icacls(&[dir.to_str().unwrap(), "/setowner", "*S-1-5-32-544"])?;

    fx.install_ok(&["--system"])?;
    wait_for_node(&fx, &node_id)?;
    let description = svc::reg_string(&fx.name(), "Description")?;
    assert!(
        description.contains(&format!("sid={}", svc::runner_sid()?)),
        "the token user must be pinned: {description}"
    );
    Ok(())
}

/// Removes this fixture's user task on drop.
struct TaskGuard<'a>(&'a Fixture);

impl Drop for TaskGuard<'_> {
    fn drop(&mut self) {
        let _ = user_scope(self.0, &["serve", "service", "uninstall", "--user"]).output();
    }
}

/// A `--user` command: the task cannot carry `IROH_TUNNEL_STATE_DIR`.
fn user_scope(fx: &Fixture, args: &[&str]) -> Command {
    let mut command = fx.command(args);
    command.env_remove("IROH_TUNNEL_STATE_DIR");
    command
}

#[tokio::test]
async fn cross_scope_refused_both_ways() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new("crossscope")?;
    fx.write_config(relay.as_ref(), 9)?;
    let config = fx.config();
    let config = config.to_str().unwrap();
    let _task = TaskGuard(&fx);

    // A system service exists: a user install is refused.
    fx.install_ok(&["--system"])?;
    let out = user_scope(
        &fx,
        &["serve", "service", "install", "--user", "--config", config],
    )
    .output()?;
    assert_exit(&out, 5);
    assert!(
        stderr(&out).contains("service uninstall --system"),
        "{}",
        stderr(&out)
    );
    assert!(!fx.task_exists(), "no user task may be registered");

    // A user task exists: a system install is refused.
    fx.service_ok("uninstall", &["--system"])?;
    fx.wait_absent()?;
    let out = user_scope(
        &fx,
        &["serve", "service", "install", "--user", "--config", config],
    )
    .output()?;
    ensure!(
        out.status.success(),
        "user install: {}{}",
        stdout(&out),
        stderr(&out)
    );
    assert!(fx.task_exists(), "the user task must exist");
    let out = fx.install(&["--system"])?;
    assert_exit(&out, 5);
    assert!(
        stderr(&out).contains("service uninstall --user"),
        "{}",
        stderr(&out)
    );
    assert!(!fx.service_exists(), "no system service may be registered");
    Ok(())
}

#[tokio::test]
async fn user_task_blocked_while_service_runs() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new("userblocked")?;
    let node_id = fx.write_config(relay.as_ref(), 9)?;
    fx.install_ok(&["--system"])?;
    wait_for_node(&fx, &node_id)?;

    let config = fx.config();
    let mut command = fx.command(&["serve", "run", "--config", config.to_str().unwrap()]);
    command.env("IROH_TUNNEL_STATE_DIR", fx.base().join("user-state"));
    let out = common::restricted::run_non_elevated(&mut command)?;
    assert_exit(&out, 6);
    assert!(stderr(&out).contains("system service"), "{}", stderr(&out));
    assert!(sc_running(&fx), "the system service keeps running");
    Ok(())
}

/// `WIN32_EXIT_CODE` of the service, as `sc query` reports it.
fn win32_exit_code(fx: &Fixture) -> Option<u32> {
    stdout(&fx.sc("query")).lines().find_map(|line| {
        let (_, rest) = line.split_once("WIN32_EXIT_CODE")?;
        rest.trim_start_matches([' ', ':'])
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    })
}

#[tokio::test]
async fn already_running_is_clean_stop() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new("alreadyrunning")?;
    let node_id = fx.write_config(relay.as_ref(), 9)?;
    // The service runs once, so the machine lock file exists.
    fx.install_ok(&["--system"])?;
    wait_for_node(&fx, &node_id)?;
    fx.service_ok("stop", &["--system"])?;
    wait_for("the service to stop", || {
        svc::service_stopped_or_absent(&fx.name())
    })?;

    // A non-elevated foreground run holds the machine lock shared.
    let user_state = fx.base().join("user-state");
    let config = fx.config();
    let mut command = fx.command(&["serve", "run", "--config", config.to_str().unwrap()]);
    command
        .env("IROH_TUNNEL_STATE_DIR", &user_state)
        .env("IROH_TUNNEL_TEST_EXIT_AFTER_MS", "40000");
    let foreground = std::thread::spawn(move || common::restricted::run_non_elevated(&mut command));
    let status = user_state.join("serve-status.json");
    wait_for("the foreground run to be ready", || {
        node_id_of(&status).as_deref() == Some(node_id.as_str())
    })?;

    // The SCM starts the service directly (no CLI precheck).
    let log_before = log_text(&fx).len();
    let start = fx.sc("start");
    ensure!(start.status.success(), "sc start: {}", stdout(&start));
    wait_for("the service to stop by itself", || {
        svc::service_stopped_or_absent(&fx.name())
    })?;
    assert_eq!(
        win32_exit_code(&fx),
        Some(0),
        "a role already running is a clean stop:\n{}",
        stdout(&fx.sc("query"))
    );
    wait_for("\"already running\" in the new log", || {
        log_text(&fx)
            .get(log_before..)
            .is_some_and(|new| new.contains("already running"))
    })?;
    // A clean stop triggers no recovery action.
    let end = Instant::now() + Duration::from_secs(8);
    while Instant::now() < end {
        assert!(
            svc::service_stopped_or_absent(&fx.name()),
            "the SCM restarted the service:\n{}",
            stdout(&fx.sc("query"))
        );
        std::thread::sleep(Duration::from_millis(250));
    }
    let foreground = foreground.join().expect("foreground thread")?;
    // It held the lock through the whole window: it ends only by the test
    // fault deadline (40 s), never early on a conflict.
    assert_ne!(
        foreground.status.code(),
        Some(6),
        "the foreground run lost the lock: {}",
        stderr(&foreground)
    );
    Ok(())
}

#[tokio::test]
async fn non_elevated_system_operations() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new("nonelevated")?;
    fx.write_config(relay.as_ref(), 9)?;
    let config = fx.config();

    let out = common::restricted::run_non_elevated(&mut fx.command(&fx_install_args(&config)))?;
    assert_exit(&out, 3);
    let err = stderr(&out);
    assert!(err.contains("Run as administrator"), "{err}");
    assert!(err.contains("serve service install --system"), "{err}");
    assert!(
        normalised(&err).contains(&normalised(config.to_str().unwrap())),
        "the guidance must name {}: {err}",
        config.display()
    );
    assert!(!fx.service_exists(), "no service may be registered");
    assert!(!fx.task_exists(), "no task may be registered");

    for verb in ["uninstall", "start", "stop", "restart"] {
        let out = common::restricted::run_non_elevated(
            &mut fx.command(&["serve", "service", verb, "--system"]),
        )?;
        assert_exit(&out, 3);
        let err = stderr(&out);
        assert!(err.contains("Run as administrator"), "{verb}: {err}");
        assert!(
            err.contains(&format!("iroh-tunnel serve service {verb} --system")),
            "{verb}: {err}"
        );
    }
    assert!(!fx.service_exists(), "nothing may be registered");
    Ok(())
}

#[tokio::test]
async fn first_start_creates_the_machine_dir_and_logs() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new("firststart")?;
    let node_id = fx.write_config(relay.as_ref(), 9)?;
    // A host that never ran a system service: no machine directory yet. The
    // verified (production) path creates it on the first service start.
    std::fs::remove_dir(&fx.machine)?;
    let out = install_with_env(&fx, &[VERIFY_MACHINE_DIR])?;
    ensure!(
        out.status.success(),
        "install: {}{}",
        stdout(&out),
        stderr(&out)
    );
    wait_for_node(&fx, &node_id)?;
    assert!(
        fx.log_file().exists(),
        "the first start must log to {}",
        fx.log_file().display()
    );
    let owner = acl_state(&fx.machine)?;
    assert!(
        owner.starts_with("owner=S-1-5-32-544\n"),
        "created by the service with the protected descriptor: {owner}"
    );
    Ok(())
}
