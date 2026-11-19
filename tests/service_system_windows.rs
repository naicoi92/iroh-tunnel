//! Task 13 (windows-system-service): end-to-end tests against a REAL Windows
//! Service, driven through the real binary on the elevated GitHub runner.
//!
//! Opt-in (`IROH_TUNNEL_E2E_SERVICE=1`): every test returns early with a
//! `skipped:` line otherwise, because they register services and create
//! directories under `%ProgramFiles%` / `%ProgramData%`.
//!
//! ## Fixture root
//!
//! A system install trusts only admin-owned files, so each test keeps its
//! config and state in `%ProgramData%\iroh-tunnel-e2e-<pid>-<test> data`,
//! created by this elevated process with a protected DACL (SYSTEM and
//! Administrators only), the shape of the real `%ProgramData%\iroh-tunnel\config`.
//! Scratch files a non-elevated child must reach (user-tier configs and
//! state) live under `%LOCALAPPDATA%`.
//!
//! ## PATH
//!
//! `PATH` is pinned to the copied exe's directory plus the Windows system
//! directories: the product registers the `iroh-tunnel` found on `PATH`, and
//! the runner's `PATH` holds the user-owned `target\debug` build.
//!
//! ## Binary trust (R1)
//!
//! An elevated `service install` refuses a binary in a user-writable directory,
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
#![cfg(test)]
#![expect(
    clippy::panic_in_result_fn,
    reason = "tests may assert inside Result-returning functions (#216)"
)]

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
    standalone: bool,
    role: &'static str,
    machine: PathBuf,
    /// Admin-only directory holding the config and state of the service.
    data: PathBuf,
    /// Removes `exe_dir`, `machine` and `data` after the service is gone; declared
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
        let program_data = env_dir("ProgramData", r"C:\ProgramData");
        let machine = program_data.join(format!("iroh-tunnel-e2e-{tag}"));
        let data = program_data.join(format!("iroh-tunnel-e2e-{tag} data"));
        // Created by this elevated process, so they inherit the parents' ACL.
        let mut dirs = DirGuard::default();
        dirs.create(&exe_dir)?;
        dirs.create(&machine)?;
        dirs.create(&data)?;
        // The system tier keeps secrets: SYSTEM and Administrators only.
        icacls(&[
            data.to_str().unwrap(),
            "/inheritance:r",
            "/grant:r",
            ADMIN_ONLY[0],
            ADMIN_ONLY[1],
        ])?;
        let exe = exe_dir.join("iroh-tunnel.exe");
        std::fs::copy(BIN, &exe)?;
        let fx = Self {
            root: tempfile::Builder::new()
                .prefix("iroh-tunnel-e2e-")
                .tempdir_in(local_app_data())?,
            suffix: format!("-e2e-{tag}"),
            exe,
            standalone: false,
            role: "serve",
            machine,
            data,
            _dirs: dirs,
        };
        std::fs::create_dir_all(fx.state())?;
        Ok(fx)
    }

    fn name(&self) -> String {
        format!("iroh-tunnel-{}{}", self.role, self.suffix)
    }

    fn base(&self) -> PathBuf {
        let canonical = self.root.path().canonicalize().expect("canonical root");
        PathBuf::from(canonical.to_string_lossy().trim_start_matches(r"\\?\"))
    }

    fn config(&self) -> PathBuf {
        self.data
            .join("cfg dir")
            .join(format!("{}.toml", self.role))
    }

    fn state(&self) -> PathBuf {
        self.data.join("state")
    }

    fn status_file(&self) -> PathBuf {
        self.state().join(format!("{}-status.json", self.role))
    }

    fn log_file(&self) -> PathBuf {
        self.machine.join("logs").join(format!("{}.log", self.role))
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

    /// Write a user-tier serve config (own key) under `%LOCALAPPDATA%`, where
    /// a non-elevated child can read it; returns its path and node id.
    fn write_user_config(&self, relay: &str) -> Result<(String, String)> {
        let key = iroh::SecretKey::generate();
        let path = self.base().join("user cfg").join("serve.toml");
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(
            &path,
            format!(
                "[node]\nsecret_key = {:?}\nrelay_urls = [{relay:?}]\n\n\
                 [[services]]\nname = \"echo\"\nprotocol = \"tcp\"\nport = 9\n",
                iroh_tunnel_host::config::encode_secret_key(&key)
            ),
        )?;
        Ok((
            path.to_string_lossy().into_owned(),
            key.public().to_string(),
        ))
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
        let mut args = if self.standalone {
            vec!["service", verb]
        } else {
            vec![self.role, "service", verb]
        };
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
        let args = self.install_args(config.to_str().unwrap(), &[]);
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
        let _ = self.service("uninstall", &[]);
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
async fn standalone_serve_service_starts_and_stops_under_scm() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let mut fx = Fixture::new("standalone-lifecycle")?;
    // Keep the combined binary on PATH to verify it is not selected.
    let standalone = fx.exe.with_file_name("renamed role.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_iroh-tunnel-serve"), &standalone)?;
    fx.exe = standalone;
    fx.standalone = true;
    let node_id = fx.write_config(relay.as_ref(), 9)?;
    fx.install_ok(&[])?;
    let pid = wait_for_node(&fx, &node_id)?;
    let expected = format!(
        "\"{}\" run --config \"{}\"",
        fx.exe.display(),
        fx.config().display()
    );
    assert_eq!(fx.image_path()?.to_lowercase(), expected.to_lowercase());
    fx.service_ok("stop", &[])?;
    wait_for("standalone process to exit", || !pid_alive(pid))?;
    assert!(!fx.status_file().exists());
    fx.service_ok("uninstall", &[])?;
    fx.wait_absent()?;
    Ok(())
}

#[tokio::test]
async fn standalone_access_service_starts_and_stops_under_scm() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let mut fx = Fixture::new("standalone-access")?;
    fx.role = "access";
    let standalone = fx.exe.with_file_name("renamed access.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_iroh-tunnel-access"), &standalone)?;
    fx.exe = standalone;
    fx.standalone = true;
    let key = iroh::SecretKey::generate();
    let peer = iroh::SecretKey::generate().public();
    let socket = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = socket.local_addr()?.port();
    drop(socket);
    std::fs::create_dir_all(fx.config().parent().unwrap())?;
    std::fs::write(fx.config(), format!("[node]\nsecret_key = {:?}\nrelay_urls = [{:?}]\n\n[[services]]\nname = \"echo\"\nnode_id = \"{peer}\"\nprotocol = \"tcp\"\nhost = \"127.0.0.1\"\nport = {port}\n", iroh_tunnel_host::config::encode_secret_key(&key), relay.as_ref() as &str))?;
    fx.install_ok(&[])?;
    let pid = wait_for_node(&fx, &key.public().to_string())?;
    let expected = format!(
        "\"{}\" run --config \"{}\"",
        fx.exe.display(),
        fx.config().display()
    );
    assert_eq!(fx.image_path()?.to_lowercase(), expected.to_lowercase());
    fx.service_ok("stop", &[])?;
    wait_for("standalone access process to exit", || !pid_alive(pid))?;
    assert!(!fx.status_file().exists());
    fx.service_ok("uninstall", &[])?;
    fx.wait_absent()?;
    Ok(())
}

#[tokio::test]
async fn system_service_lifecycle() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new("lifecycle")?;
    let node_id = fx.write_config(relay.as_ref(), 9)?;

    fx.install_ok(&[])?;
    let pid = wait_for_node(&fx, &node_id)?;
    let status = fx.service_ok("status", &[])?;
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

    // The definition: automatic start, LocalSystem, own process, the SCM
    // marker and no pin of a user's identity.
    let name = fx.name();
    assert_eq!(
        svc::reg_dword(&name, "Start")?,
        2,
        "Start must be AUTO_START"
    );
    assert_eq!(svc::reg_string(&name, "ObjectName")?, "LocalSystem");
    let kind = svc::reg_dword(&name, "Type")?;
    assert_eq!(kind & 0x10, 0x10, "Type {kind:#x} lacks WIN32_OWN_PROCESS");
    let description = svc::reg_string(&name, "Description")?;
    assert!(!description.contains("iroh-tunnel-pin"), "{description}");
    let env = svc::service_env(&name)?;
    assert!(env.contains(&"IROH_TUNNEL_SCM=1".to_owned()), "{env:?}");
    assert!(
        !env.iter()
            .any(|e| e.starts_with("IROH_TUNNEL_TRUSTED_SID=")),
        "{env:?}"
    );

    // stop: the process ends and the status file is removed (the drain).
    fx.service_ok("stop", &[])?;
    wait_for("the process to exit", || !pid_alive(pid))?;
    assert!(
        !fx.status_file().exists(),
        "stop must drain and remove the status file"
    );
    assert!(fx.service_ok("status", &[])?.contains("not running"));

    // start and restart: a new process with the same identity each time.
    fx.service_ok("start", &[])?;
    let started = wait_for_new_pid(&fx, &node_id, pid)?;
    fx.service_ok("restart", &[])?;
    let restarted = wait_for_new_pid(&fx, &node_id, started)?;
    assert_ne!(restarted, started);

    // uninstall removes the service; status then reports not installed.
    fx.service_ok("uninstall", &[])?;
    fx.wait_absent()?;
    let gone = fx.service("status", &[])?;
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
    fx.install_ok(&[])?;
    let old_pid = wait_for_node(&fx, &node_id)?;

    // A second config in the same admin-only directory, same identity.
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
    assert!(fx.service_ok("status", &[])?.contains("is running"));

    let image = svc::image_path(&name)?.to_lowercase();
    assert!(
        image.contains(&second.to_string_lossy().to_lowercase()),
        "ImagePath must name {}: {image}",
        second.display()
    );
    assert!(!image.contains("serve.toml"), "old config in {image}");
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
    fx.install_ok(&[])?;
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
        let mut command = fx.command(&["serve", "service", "stop"]);
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

/// From an elevated terminal, `service stop` targets the Windows Service and
/// leaves the same role's user task alone.
#[tokio::test]
async fn elevated_stop_targets_scm_service() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new("elevatedstop")?;
    let node_id = fx.write_config(relay.as_ref(), 9)?;
    let (user_config, user_id) = fx.write_user_config(relay.as_ref())?;
    let _task = TaskGuard(&fx);

    fx.install_ok(&[])?;
    let pid = wait_for_node(&fx, &node_id)?;
    let out = run_user(
        &fx,
        &["serve", "service", "install", "--config", &user_config],
    )?;
    ensure!(
        out.status.success(),
        "user install: {}{}",
        stdout(&out),
        stderr(&out)
    );
    assert!(fx.task_exists(), "the user task must exist");
    let user_pid = wait_for_user_task(&user_id)?;

    fx.service_ok("stop", &[])?;
    wait_for("the service process to exit", || !pid_alive(pid))?;
    assert!(fx.service_ok("status", &[])?.contains("not running"));
    // The user task kept its process and still reports running.
    assert!(pid_alive(user_pid), "an elevated stop ended the user task");
    assert!(fx.task_exists(), "an elevated stop removed the user task");
    let status = run_user(&fx, &["serve", "service", "status"])?;
    assert!(
        stdout(&status).contains(&format!("serve is running (pid {user_pid}")),
        "user task status: {}{}",
        stdout(&status),
        stderr(&status)
    );
    assert_eq!(
        node_id_of(&user_status_file()?).as_deref(),
        Some(user_id.as_str())
    );
    Ok(())
}

/// An elevated install without `--config` registers only a Windows Service
/// (no scheduled task) running `%ProgramData%\iroh-tunnel\config\serve.toml`,
/// which an elevated `config keygen` created admin-only.
#[tokio::test]
async fn elevated_install_uses_programdata_config() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let system = env_dir("ProgramData", r"C:\ProgramData").join("iroh-tunnel");
    let (config_dir, state_dir) = (system.join("config"), system.join("state"));
    if config_dir.exists() || state_dir.exists() {
        eprintln!("skipped: {} already holds a system tier", system.display());
        return Ok(());
    }
    // Remove what this test creates: the whole machine directory when it did
    // not exist before, else only its `config` and `state`.
    let _cleanup = if system.exists() {
        vec![
            CreatedDir(config_dir.clone()),
            CreatedDir(state_dir.clone()),
        ]
    } else {
        // The shape a release build creates: SYSTEM and Administrators full,
        // Users read (the test build keeps its locks in the fixture).
        std::fs::create_dir(&system)?;
        let guard = vec![CreatedDir(system.clone())];
        icacls(&[
            system.to_str().unwrap(),
            "/inheritance:r",
            "/grant:r",
            ADMIN_ONLY[0],
            ADMIN_ONLY[1],
            "*S-1-5-32-545:(OI)(CI)RX",
        ])?;
        guard
    };
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new("programdata")?;
    let config = config_dir.join("serve.toml");
    let system_tier = |args: &[&str]| -> Result<Output> {
        let mut command = fx.command(args);
        command
            .env_remove("IROH_TUNNEL_STATE_DIR")
            .env_remove("IROH_TUNNEL_TEST_CONFIG_DIR")
            .env_remove("IROH_TUNNEL_TEST_TIER");
        Ok(command.output()?)
    };
    let relay_url: &str = relay.as_ref();
    for args in [
        &["serve", "config", "keygen"][..],
        &["serve", "config", "node", "--relay-url", relay_url],
        &[
            "serve", "config", "set", "-n", "echo", "-p", "tcp", "-t", "9",
        ],
    ] {
        let out = system_tier(args)?;
        ensure!(out.status.success(), "{args:?}: {}", stderr(&out));
    }
    assert!(config.exists(), "keygen must write {}", config.display());

    // The secret is hidden from a non-elevated user: the file's own DACL,
    // then a read attempt from a restricted token.
    assert_admin_only_readable(&config_dir)?;
    assert_admin_only_readable(&config)?;
    let read = common::restricted::run_non_elevated(
        Command::new("cmd").args(["/c", "type"]).arg(&config),
    )?;
    assert!(!read.status.success(), "a non-elevated user read the key");
    assert!(read.stdout.is_empty(), "no secret bytes may leak");

    let out = system_tier(&["serve", "service", "install"])?;
    ensure!(
        out.status.success(),
        "install: {}{}",
        stdout(&out),
        stderr(&out)
    );
    assert!(fx.service_exists(), "the Windows Service must exist");
    assert!(!fx.task_exists(), "no scheduled task may be created");
    let name = fx.name();
    let image = fx.image_path()?.to_lowercase();
    assert!(
        image.ends_with(&format!(
            "serve run --config \"{}\"",
            config.display().to_string().to_lowercase()
        )),
        "{image}"
    );
    assert_eq!(
        svc::reg_dword(&name, "Start")?,
        2,
        "Start must be AUTO_START"
    );
    assert_eq!(svc::reg_string(&name, "ObjectName")?, "LocalSystem");
    let status = state_dir.join("serve-status.json");
    wait_for("the service to report status", || {
        status_json(&status).is_some()
    })?;
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
        let out = fx.service(verb, &[])?;
        let err = stderr(&out);
        assert_eq!(out.status.code(), Some(5), "{verb}: {err}");
        assert!(err.contains("service install"), "{verb}: {err}");
    }
    let out = fx.service("uninstall", &[])?;
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));

    // Stopping an already stopped service is fine.
    fx.install_ok(&[])?;
    fx.service_ok("stop", &[])?;
    let again = fx.service("stop", &[])?;
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

// Needs the root `test-utils` seam `IROH_TUNNEL_TEST_EXIT_AFTER_MS`.
#[cfg(feature = "test-utils")]
#[tokio::test]
async fn crash_is_restarted() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new("crash")?;
    let node_id = fx.write_config(relay.as_ref(), 9)?;
    fx.install_ok(&[])?;
    let first = wait_for_node(&fx, &node_id)?;

    fx.set_service_env(&[("IROH_TUNNEL_TEST_EXIT_AFTER_MS", "1500")])?;
    fx.service_ok("restart", &[])?;
    let crashing = wait_for_new_pid(&fx, &node_id, first)?;
    // The process fails after 1.5 s; recovery restarts it after 5 s.
    let recovered = wait_for_new_pid(&fx, &node_id, crashing)?;
    assert_ne!(recovered, crashing);
    Ok(())
}

// Needs the root `test-utils` seam `IROH_TUNNEL_TEST_FAIL_BEFORE_READY`.
#[cfg(feature = "test-utils")]
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

/// SYSTEM and Administrators with full control, inherited, as `icacls` takes it.
const ADMIN_ONLY: [&str; 2] = ["*S-1-5-18:(OI)(CI)F", "*S-1-5-32-544:(OI)(CI)F"];
/// Users (`S-1-5-32-545`) with inherited read, as `icacls` takes it.
const USERS_READ: &str = "*S-1-5-32-545:(OI)(CI)R";

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

/// SYSTEM, Administrators and TrustedInstaller: the only principals a
/// system-tier secret may be owned by or readable to.
const ADMIN_PRINCIPALS: [&str; 3] = [
    "S-1-5-18",
    "S-1-5-32-544",
    "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464",
];
/// `FILE_READ_DATA | GENERIC_READ | GENERIC_ALL`: the rights that let an ACE
/// read a file's content.
const READ_RIGHTS: u64 = 0x1 | 0x8000_0000 | 0x1000_0000;

/// One access rule of a DACL.
#[derive(Debug)]
struct Ace {
    /// `Allow` or `Deny`.
    kind: String,
    sid: String,
    mask: u64,
}

/// Owner SID and every explicit or inherited ACE of `path`. Inherit-only
/// ACEs are kept: on a directory they decide who can read the files created
/// inside.
fn dacl_entries(path: &Path) -> Result<(String, Vec<Ace>)> {
    let p = path.display();
    // `FileSystemRights` is an Int32 enum: generic bits make it negative, so
    // widen and mask instead of casting to an unsigned type.
    let script = format!(
        "$a = Get-Acl -LiteralPath '{p}'; \
         'owner|' + $a.GetOwner([System.Security.Principal.SecurityIdentifier]).Value; \
         $a.GetAccessRules($true, $true, [System.Security.Principal.SecurityIdentifier]) | \
         ForEach-Object {{ '{{0}}|{{1}}|{{2}}' -f $_.AccessControlType, \
         $_.IdentityReference.Value, ([int64][int]$_.FileSystemRights -band 4294967295) }}"
    );
    let out = pwsh(&script)?;
    ensure!(out.status.success(), "Get-Acl {p}: {}", stderr(&out));
    let text = stdout(&out);
    let mut owner = None;
    let mut aces = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let fields: Vec<&str> = line.split('|').collect();
        match fields.as_slice() {
            ["owner", sid] => owner = Some((*sid).to_owned()),
            [kind, sid, mask] => aces.push(Ace {
                kind: (*kind).to_owned(),
                sid: (*sid).to_owned(),
                mask: mask
                    .parse()
                    .map_err(|e| anyhow::anyhow!("mask {mask:?} in {line:?}: {e}"))?,
            }),
            _ => anyhow::bail!("unexpected Get-Acl line {line:?}"),
        }
    }
    let owner = owner.ok_or_else(|| anyhow::anyhow!("no owner for {p}: {text}"))?;
    Ok((owner, aces))
}

/// `path` is owned by an admin principal and every Allow ACE that grants a
/// read right belongs to one: no one else can read the secret it holds.
fn assert_admin_only_readable(path: &Path) -> Result<()> {
    let (owner, aces) = dacl_entries(path)?;
    ensure!(
        ADMIN_PRINCIPALS.contains(&owner.as_str()),
        "{} is owned by {owner}, not an admin principal",
        path.display()
    );
    ensure!(!aces.is_empty(), "{} has an empty DACL", path.display());
    for Ace { kind, sid, mask } in &aces {
        ensure!(
            kind != "Allow" || mask & READ_RIGHTS == 0 || ADMIN_PRINCIPALS.contains(&sid.as_str()),
            "{} lets {sid} read (mask {mask:#x}): {aces:?}",
            path.display()
        );
    }
    Ok(())
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
    fx.service_ok("stop", &[])?;
    wait_for("the service to stop", || {
        svc::service_stopped_or_absent(&fx.name())
    })
}

/// Retry `start --system` until the service is Running: after a refusal the
/// SCM recovery action may start it on its own between two attempts.
fn start_until_running(fx: &Fixture) -> Result<()> {
    wait_for("the service to run after start", || {
        if !sc_running(fx) {
            let _ = fx.service("start", &[]);
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
    fx.install_ok(&[])?;
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
    let real = fx.data.join("real");
    std::fs::create_dir(&real)?;
    std::fs::copy(fx.config(), real.join("serve.toml"))?;
    let link = fx.data.join("link");
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
    fx.install_ok(&[])?;
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

/// An Administrators-owned config directory that lets Users read is
/// refused before anything is registered, and its DACL is left alone.
#[tokio::test]
async fn readable_config_dir_refused() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new("readabledir")?;
    fx.write_config(relay.as_ref(), 9)?;
    let dir = fx.config().parent().unwrap().to_path_buf();
    icacls(&[dir.to_str().unwrap(), "/grant", USERS_READ])?;
    let rules = acl_state(&dir)?;

    let out = fx.install(&[])?;
    assert_exit(&out, 3);
    let err = stderr(&out);
    assert!(
        normalised(&err).contains(&normalised(dir.to_str().unwrap())) && err.contains("Users"),
        "the refusal must name {} and Users: {err}",
        dir.display()
    );
    assert!(!fx.service_exists(), "no service may be registered");
    assert_eq!(acl_state(&dir)?, rules, "never repaired");
    Ok(())
}

/// A service registered by an earlier version (it pins the installing
/// user's SID) is refused with a config error naming the system config.
#[tokio::test]
async fn legacy_sid_service_refused() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new("legacysid")?;
    let node_id = fx.write_config(relay.as_ref(), 9)?;
    fx.install_ok(&[])?;
    wait_for_node(&fx, &node_id)?;
    stop_cleanly(&fx)?;
    let bytes = std::fs::read(fx.config())?;

    fx.set_service_env(&[("IROH_TUNNEL_TRUSTED_SID", &svc::runner_sid()?)])?;
    assert_eq!(refused_start(&fx)?, 2, "a config error");
    let system_config =
        env_dir("ProgramData", r"C:\ProgramData").join(r"iroh-tunnel\config\serve.toml");
    wait_for_log(&fx, system_config.to_str().unwrap())?;
    wait_for_log(&fx, "iroh-tunnel serve service install")?;
    wait_for_log(&fx, "Run as administrator")?;
    assert_eq!(
        std::fs::read(fx.config())?,
        bytes,
        "the config is untouched"
    );
    Ok(())
}

/// A system config with an empty key: the service generates one, persists it
/// and the file stays readable by admins only.
#[tokio::test]
async fn empty_key_persisted_admin_only() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new("emptykey")?;
    let node_id = fx.write_config(relay.as_ref(), 9)?;
    fx.install_ok(&[])?;
    wait_for_node(&fx, &node_id)?;
    stop_cleanly(&fx)?;
    let emptied = std::fs::read_to_string(fx.config())?
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
    std::fs::write(fx.config(), &emptied)?;

    let _ = std::fs::remove_file(fx.status_file());
    start_until_running(&fx)?;
    wait_for("a status file with a new identity", || {
        node_id_of(&fx.status_file()).is_some_and(|id| id != node_id)
    })?;
    let text = std::fs::read_to_string(fx.config())?;
    let doc: toml::Value = toml::from_str(&text)?;
    let key = doc["node"]["secret_key"].as_str().unwrap_or_default();
    assert!(!key.is_empty(), "the generated key must be persisted");

    // The proof is the file's own descriptor; the restricted read below is
    // an end-to-end check only (the fixture's ancestors are admin-only too).
    assert_admin_only_readable(&fx.config())?;
    let read = common::restricted::run_non_elevated(
        Command::new("cmd").args(["/c", "type"]).arg(fx.config()),
    )?;
    assert!(!read.status.success(), "a non-elevated user read the key");
    assert!(read.stdout.is_empty(), "no secret bytes may leak");
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
    fx.install_ok(&[])?;
    wait_for_node(&fx, &node_id)?;
    fx.service_ok("stop", &[])?;
    let _grant = GrantGuard::everyone_modify(&fx.exe)?;
    let out = fx.service("start", &[])?;
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
// Tiers and concurrency
// ---------------------------------------------------------------------------

/// Removes this fixture's user task on drop.
struct TaskGuard<'a>(&'a Fixture);

impl Drop for TaskGuard<'_> {
    fn drop(&mut self) {
        let _ = run_user(self.0, &["serve", "service", "uninstall"]);
    }
}

/// `args` run in the user tier: a non-elevated child (restricted token) of
/// the elevated runner. The task cannot carry `IROH_TUNNEL_STATE_DIR`.
fn run_user(fx: &Fixture, args: &[&str]) -> Result<Output> {
    let mut command = fx.command(args);
    command.env_remove("IROH_TUNNEL_STATE_DIR");
    common::restricted::run_non_elevated(&mut command)
}

/// The runner's own user-tier serve snapshot. The restricted child runs as
/// the same account, so it shares this `%LOCALAPPDATA%` state directory.
fn user_status_file() -> Result<PathBuf> {
    Ok(
        iroh_tunnel_host::paths::default_state_dir_for(iroh_tunnel_host::Tier::User)?
            .join("serve-status.json"),
    )
}

/// Wait until the user task runs `node_id` and return its live pid.
fn wait_for_user_task(node_id: &str) -> Result<u64> {
    let status = user_status_file()?;
    wait_for("the user task to report its own status", || {
        node_id_of(&status).as_deref() == Some(node_id) && pid_of(&status).is_some_and(pid_alive)
    })?;
    pid_of(&status).ok_or_else(|| anyhow::anyhow!("no pid in the user snapshot"))
}

/// The user tier's notice that the system service runs.
const SYSTEM_NOTICE: &str = "note: serve is also running as a system service";

/// The logged-in user's task runs next to the Windows Service: the user
/// install proceeds with the other-tier notice and both keep running.
#[tokio::test]
async fn user_task_and_service_coexist() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new("coexist")?;
    let node_id = fx.write_config(relay.as_ref(), 9)?;
    let (user_config, user_id) = fx.write_user_config(relay.as_ref())?;
    let _task = TaskGuard(&fx);
    fx.install_ok(&[])?;
    let pid = wait_for_node(&fx, &node_id)?;

    let out = run_user(
        &fx,
        &["serve", "service", "install", "--config", &user_config],
    )?;
    ensure!(
        out.status.success(),
        "user install: {}{}",
        stdout(&out),
        stderr(&out)
    );
    assert!(stderr(&out).contains(SYSTEM_NOTICE), "{}", stderr(&out));
    assert!(fx.task_exists(), "the user task must exist");
    wait_for_user_task(&user_id)?;
    assert!(pid_alive(pid), "the system service must keep running");
    assert!(sc_running(&fx));
    Ok(())
}

/// `WIN32_EXIT_CODE` of the service, as `sc query` reports it.
#[cfg(feature = "test-utils")]
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

// Needs the root `test-utils` seam `IROH_TUNNEL_TEST_EXIT_AFTER_MS`.
#[cfg(feature = "test-utils")]
#[tokio::test]
async fn already_running_is_clean_stop() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new("alreadyrunning")?;
    let node_id = fx.write_config(relay.as_ref(), 9)?;
    // The service runs once, so the machine lock file exists.
    fx.install_ok(&[])?;
    wait_for_node(&fx, &node_id)?;
    fx.service_ok("stop", &[])?;
    wait_for("the service to stop", || {
        svc::service_stopped_or_absent(&fx.name())
    })?;

    // An elevated foreground run is a second system-tier holder of the
    // role: it claims the same system lock the service needs.
    let holder_state = fx.data.join("holder-state");
    let config = fx.config();
    let mut command = fx.command(&["serve", "run", "--config", config.to_str().unwrap()]);
    command
        .env("IROH_TUNNEL_STATE_DIR", &holder_state)
        .env("IROH_TUNNEL_TEST_EXIT_AFTER_MS", "40000");
    let foreground = std::thread::spawn(move || command.output());
    let status = holder_state.join("serve-status.json");
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

/// Removes a config directory this test created, even on failure.
struct CreatedDir(PathBuf);

impl Drop for CreatedDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Fix round 1 (Task 5): from an elevated terminal whose working directory
/// is a Users-readable `%ProgramData%\iroh-tunnel` with `config` missing,
/// `serve config keygen --config config\serve.toml` (relative) creates the
/// directory admin-only, writes an admin-only secret and leaves the machine
/// directory's DACL unchanged.
#[test]
fn relative_keygen_from_readable_machine_dir_is_admin_only() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let machine = env_dir("ProgramData", r"C:\ProgramData").join("iroh-tunnel");
    let config_dir = machine.join("config");
    if config_dir.exists() {
        eprintln!("skipped: {} already exists", config_dir.display());
        return Ok(());
    }
    std::fs::create_dir_all(&machine)?;
    let before = acl_state(&machine)?;
    ensure!(
        before.contains("S-1-5-32-545|"),
        "precondition: the machine directory must be Users-readable: {before}"
    );
    let _cleanup = CreatedDir(config_dir.clone());

    let out = Command::new(BIN)
        .args([
            "serve",
            "config",
            "keygen",
            "--config",
            r"config\serve.toml",
        ])
        .current_dir(&machine)
        .env_remove("IROH_TUNNEL_TEST_CONFIG_DIR")
        .env_remove("IROH_TUNNEL_TEST_TIER")
        .stdin(Stdio::null())
        .output()?;
    ensure!(out.status.success(), "keygen: {}", stderr(&out));

    for path in [config_dir.clone(), config_dir.join("serve.toml")] {
        let acl = acl_state(&path)?;
        for outsider in ["S-1-5-32-545|", "S-1-5-11|", "S-1-1-0|"] {
            ensure!(
                !acl.contains(outsider),
                "{} grants {outsider}: {acl}",
                path.display()
            );
        }
    }
    assert_eq!(acl_state(&machine)?, before, "machine DACL unchanged");
    Ok(())
}
