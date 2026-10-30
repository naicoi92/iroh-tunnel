//! #87: role ownership and status, driven through real processes.
//!
//! The `test-utils` feature disables relay TLS verification for the spawned
//! binaries: use only the isolated in-process relay, never public relays or
//! real secrets. Every spawn gets its own `HOME` (the per-user ownership
//! scope) and its own machine-lock directory (`IROH_TUNNEL_MACHINE_LOCK_DIR`,
//! a `test-utils` stand-in for the root-owned system location), so these tests
//! never touch a real service on the developer machine. The system tier is
//! therefore exercised through that directory, not through a real root
//! process or a real system service.
//!
//! Acceptance map:
//! - AC-RO-01 / AC-RO-04: same role refused in either start order and across
//!   binary names, config and state overrides; start races admit one owner.
//! - AC-RO-02 / RO-02: different roles coexist with distinct keys; the same
//!   public identity is rejected across roles.
//! - AC-RO-03 / AC-RO-05 / AC-RO-06: status names role and owner, and never
//!   trusts a pid alone; a `kill -9` leftover neither blocks a restart nor
//!   reads as live.
//! - RO-03: `service start` is refused while the role runs.

#![cfg(unix)]

use anyhow::Result;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

const ALL_IN_ONE: &str = env!("CARGO_BIN_EXE_iroh-tunnel");
const SERVE: &str = env!("CARGO_BIN_EXE_iroh-tunnel-serve");
const ACCESS: &str = env!("CARGO_BIN_EXE_iroh-tunnel-access");

/// Exit code of a start refused because the role (or identity) is taken.
const CONFLICT_EXIT: i32 = 6;

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// One user's world: `HOME` (the ownership scope) plus the machine-lock dir.
struct World {
    root: tempfile::TempDir,
}

impl World {
    fn new() -> Result<Self> {
        let world = Self {
            root: tempfile::tempdir()?,
        };
        std::fs::create_dir_all(world.home())?;
        std::fs::create_dir_all(world.machine())?;
        Ok(world)
    }
    fn home(&self) -> PathBuf {
        self.root.path().join("home")
    }
    fn machine(&self) -> PathBuf {
        self.root.path().join("machine")
    }
    fn state(&self, name: &str) -> PathBuf {
        let dir = self.root.path().join(format!("state-{name}"));
        std::fs::create_dir_all(&dir).expect("state dir");
        dir
    }
    fn command(&self, exe: &str, prefix: Option<&str>, state: &Path) -> Command {
        let mut command = Command::new(exe);
        command
            .args(prefix)
            .env("HOME", self.home())
            .env("XDG_CONFIG_HOME", self.home())
            .env("IROH_TUNNEL_TEST_CONFIG_DIR", self.home())
            .env("IROH_TUNNEL_MACHINE_LOCK_DIR", self.machine())
            .env("IROH_TUNNEL_STATE_DIR", state)
            .env("RUST_LOG", "iroh_tunnel=info");
        command
    }
}

fn write_key(dir: &Path, name: &str) -> Result<(PathBuf, String)> {
    let key = iroh::SecretKey::generate();
    let path = dir.join(name);
    std::fs::write(&path, iroh_tunnel::config::encode_secret_key(&key))?;
    Ok((path, key.public().to_string()))
}

/// Quick-mode serve arguments (no config file needed).
fn serve_args(relay: &str, key: &Path) -> Vec<String> {
    [
        "--name",
        "echo",
        "--target",
        "127.0.0.1:9",
        "--relay-url",
        relay,
        "--key-file",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([key.display().to_string()])
    .collect()
}

fn free_loopback() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("free port");
    listener.local_addr().expect("addr").to_string()
}

/// Quick-mode access arguments toward `node` (a public key).
fn access_args(relay: &str, key: &Path, node: &str) -> Vec<String> {
    [
        "--name",
        "echo",
        "--listen",
        &free_loopback(),
        "--node-id",
        node,
        "--relay-url",
        relay,
        "--key-file",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([key.display().to_string()])
    .collect()
}

fn spawn(mut command: Command, args: &[String]) -> Result<Process> {
    Ok(Process(
        command
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?,
    ))
}

/// Wait until `status_file` appears (the role owns its scope) or it exits.
fn wait_running(status_file: &Path, process: &mut Process) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if std::fs::read(status_file)
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .is_some()
        {
            return Ok(());
        }
        if let Some(exit) = process.0.try_wait()? {
            anyhow::bail!("role exited before running: {exit}");
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "timeout waiting for {}",
            status_file.display()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Wait for a refused process to exit; returns code and stderr.
fn wait_refused(process: Process) -> Result<(Option<i32>, String)> {
    let mut process = process;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = process.0.try_wait()? {
            let mut stderr = String::new();
            if let Some(mut pipe) = process.0.stderr.take() {
                std::io::Read::read_to_string(&mut pipe, &mut stderr)?;
            }
            return Ok((status.code(), stderr));
        }
        anyhow::ensure!(Instant::now() < deadline, "process was not refused");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn assert_refused(refused: (Option<i32>, String), reason: &str) {
    let (code, stderr) = refused;
    assert_eq!(code, Some(CONFLICT_EXIT), "stderr: {stderr}");
    assert!(stderr.contains(reason), "stderr lacks {reason:?}: {stderr}");
}

fn start_serve(
    world: &World,
    exe: &str,
    prefix: Option<&str>,
    relay: &str,
    key: &Path,
    state: &Path,
) -> Result<Process> {
    let mut process = spawn(world.command(exe, prefix, state), &serve_args(relay, key))?;
    wait_running(&state.join("serve-status.json"), &mut process)?;
    Ok(process)
}

async fn relay() -> Result<(String, impl Sized)> {
    let (_map, url, server) = iroh::test_utils::run_relay_server().await?;
    Ok((url.to_string(), server))
}

#[tokio::test]
async fn second_serve_is_refused_across_binaries_overrides_and_start_order() -> Result<()> {
    let (relay, _server) = relay().await?;
    let world = World::new()?;
    let (key_a, _) = write_key(world.root.path(), "a.key")?;
    let (key_b, _) = write_key(world.root.path(), "b.key")?;

    // all-in-one first, standalone second, different state override.
    let first_state = world.state("first");
    let first = start_serve(
        &world,
        ALL_IN_ONE,
        Some("serve"),
        &relay,
        &key_a,
        &first_state,
    )?;
    let second = spawn(
        world.command(SERVE, None, &world.state("second")),
        &serve_args(&relay, &key_b),
    )?;
    assert_refused(wait_refused(second)?, "already running");
    drop(first);

    // Reverse order: standalone first, all-in-one second.
    let first = start_serve(&world, SERVE, None, &relay, &key_a, &world.state("third"))?;
    let second = spawn(
        world.command(ALL_IN_ONE, Some("serve"), &world.state("fourth")),
        &serve_args(&relay, &key_b),
    )?;
    assert_refused(wait_refused(second)?, "already running");
    drop(first);
    Ok(())
}

#[tokio::test]
async fn roles_with_distinct_keys_coexist_but_a_shared_key_is_rejected() -> Result<()> {
    let (relay, _server) = relay().await?;
    let world = World::new()?;
    let (serve_key, serve_id) = write_key(world.root.path(), "serve.key")?;
    let (access_key, _) = write_key(world.root.path(), "access.key")?;
    let serve_state = world.state("serve");
    let _serve = start_serve(
        &world,
        ALL_IN_ONE,
        Some("serve"),
        &relay,
        &serve_key,
        &serve_state,
    )?;

    // Distinct roles with distinct identities coexist.
    let access_state = world.state("access");
    let mut access = spawn(
        world.command(ACCESS, None, &access_state),
        &access_args(&relay, &access_key, &serve_id),
    )?;
    wait_running(&access_state.join("access-status.json"), &mut access)?;

    let other = World::new()?;
    let other_serve_state = other.state("serve");
    let _serve = start_serve(&other, SERVE, None, &relay, &serve_key, &other_serve_state)?;
    let clash = spawn(
        other.command(ACCESS, None, &other.state("access")),
        &access_args(&relay, &serve_key, &serve_id),
    )?;
    assert_refused(wait_refused(clash)?, "same public identity");
    Ok(())
}

#[tokio::test]
async fn concurrent_starts_admit_exactly_one_owner() -> Result<()> {
    let (relay, _server) = relay().await?;
    let world = World::new()?;
    let (key, _) = write_key(world.root.path(), "k.key")?;
    let mut contenders = Vec::new();
    for i in 0..4 {
        let exe = if i % 2 == 0 { SERVE } else { ALL_IN_ONE };
        let prefix = (i % 2 == 1).then_some("serve");
        contenders.push(spawn(
            world.command(exe, prefix, &world.state(&format!("race{i}"))),
            &serve_args(&relay, &key),
        )?);
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    while contenders.len() > 1 {
        anyhow::ensure!(Instant::now() < deadline, "race did not settle");
        let mut still_running = Vec::new();
        for mut p in contenders {
            match p.0.try_wait()? {
                Some(status) => assert_eq!(status.code(), Some(CONFLICT_EXIT)),
                None => still_running.push(p),
            }
        }
        contenders = still_running;
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(contenders.len(), 1, "exactly one owner survives");
    Ok(())
}

#[tokio::test]
async fn refused_config_run_neither_persists_a_key_nor_touches_the_owner() -> Result<()> {
    let (relay, _server) = relay().await?;
    let world = World::new()?;
    let (key, _) = write_key(world.root.path(), "k.key")?;
    let state = world.state("owner");
    let _owner = start_serve(&world, ALL_IN_ONE, Some("serve"), &relay, &key, &state)?;
    let before = std::fs::read(state.join("serve-status.json"))?;

    // A configured run with an empty secret_key would generate and save one.
    let config = world.root.path().join("serve.toml");
    let body = format!(
        "[node]\nsecret_key = \"\"\nrelay_urls = [\"{relay}\"]\n\n[[services]]\nname = \"echo\"\nhost = \"127.0.0.1\"\nport = 9\nprotocol = \"tcp\"\n"
    );
    std::fs::write(&config, &body)?;
    let refused = spawn(
        world.command(SERVE, None, &world.state("other")),
        &[
            "run".into(),
            "--config".into(),
            config.display().to_string(),
        ],
    )?;
    assert_refused(wait_refused(refused)?, "already running");
    assert_eq!(std::fs::read_to_string(&config)?, body, "key was persisted");
    assert_eq!(std::fs::read(state.join("serve-status.json"))?, before);
    Ok(())
}

#[tokio::test]
async fn kill_nine_leftover_neither_blocks_restart_nor_reads_as_live() -> Result<()> {
    let (relay, _server) = relay().await?;
    let world = World::new()?;
    let (key, _) = write_key(world.root.path(), "k.key")?;
    let state = world.state("main");
    let mut first = start_serve(&world, ALL_IN_ONE, Some("serve"), &relay, &key, &state)?;
    first.0.kill()?; // SIGKILL: no cleanup runs
    first.0.wait()?;
    assert!(
        state.join("serve-status.json").exists(),
        "leftover expected"
    );

    let stale = world
        .command(ALL_IN_ONE, Some("serve"), &state)
        .args(["status"])
        .output()?;
    let text = out(&stale);
    assert!(
        text.contains("serve is not running") && text.contains("stale"),
        "crash snapshot must not read as live: {text}"
    );

    let _restart = start_serve(&world, ALL_IN_ONE, Some("serve"), &relay, &key, &state)?;
    Ok(())
}

#[test]
fn status_does_not_trust_a_reused_pid() -> Result<()> {
    let world = World::new()?;
    let state = world.state("main");
    // A live, unrelated process (this test) carries the recorded pid; no
    // role holds the lock.
    let body = serde_json::json!({
        "node_id": "00", "home_relay": null, "pid": std::process::id(),
        "started_at": 1, "services": [], "connections": []
    });
    std::fs::write(state.join("serve-status.json"), body.to_string())?;
    let output = world
        .command(SERVE, None, &state)
        .args(["status"])
        .output()?;
    let text = out(&output);
    assert!(
        text.contains("serve is not running") && text.contains("stale"),
        "pid alone must not prove liveness: {text}"
    );
    Ok(())
}

#[tokio::test]
async fn running_role_status_names_role_and_owner_pid() -> Result<()> {
    let (relay, _server) = relay().await?;
    let world = World::new()?;
    let (key, _) = write_key(world.root.path(), "k.key")?;
    let state = world.state("main");
    let serve = start_serve(&world, SERVE, None, &relay, &key, &state)?;
    // Query from a different state override: the snapshot is absent there,
    // but the live owner must still be reported, never "not running".
    let output = world
        .command(ALL_IN_ONE, Some("serve"), &world.state("elsewhere"))
        .args(["status"])
        .output()?;
    let text = out(&output);
    assert!(text.contains("running"), "{text}");
    assert!(
        text.contains(&serve.0.id().to_string()),
        "owner pid: {text}"
    );
    assert!(!text.contains("not running"), "{text}");
    Ok(())
}

#[tokio::test]
async fn system_tier_lock_refuses_user_start_and_blocks_a_system_start() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    // Root ignores file modes, so the non-elevated view cannot be simulated.
    if running_as_root() {
        eprintln!("skipped: running as root");
        return Ok(());
    }
    let (relay, _server) = relay().await?;
    let world = World::new()?;
    let (key, _) = write_key(world.root.path(), "k.key")?;

    // A "system service": the test holds the machine lock exclusively on a
    // file the user process cannot open for writing (read-only, like a
    // root-owned file seen by a non-elevated user).
    let machine_lock = world.machine().join("serve.lock");
    drop(File::create(&machine_lock)?);
    let _restore = RestoreMode(world.machine());
    std::fs::set_permissions(&machine_lock, std::fs::Permissions::from_mode(0o444))?;
    std::fs::set_permissions(world.machine(), std::fs::Permissions::from_mode(0o555))?;
    let system = File::open(&machine_lock)?;
    system.try_lock()?;
    let refused = spawn(
        world.command(ALL_IN_ONE, Some("serve"), &world.state("user")),
        &serve_args(&relay, &key),
    )?;
    assert_refused(wait_refused(refused)?, "system service");
    drop(system);

    // Reverse order: a running user process stops a later system start.
    let mut user = spawn(
        world.command(ALL_IN_ONE, Some("serve"), &world.state("user2")),
        &serve_args(&relay, &key),
    )?;
    wait_running(&world.state("user2").join("serve-status.json"), &mut user)?;
    let later = File::open(&machine_lock)?;
    assert!(
        matches!(later.try_lock(), Err(std::fs::TryLockError::WouldBlock)),
        "a running user role must hold the machine lock shared"
    );
    Ok(())
}

#[test]
fn service_start_is_refused_while_the_role_runs() -> Result<()> {
    let world = World::new()?;
    let user_lock_dir = world.home().join(".local/state/iroh-tunnel");
    std::fs::create_dir_all(&user_lock_dir)?;
    let held = File::create(user_lock_dir.join("serve.lock"))?;
    held.try_lock()?;
    let output = world
        .command(ALL_IN_ONE, Some("serve"), &world.state("main"))
        .args(["service", "start"])
        .output()?;
    assert_eq!(output.status.code(), Some(CONFLICT_EXIT));
    assert!(String::from_utf8_lossy(&output.stderr).contains("already running"));
    Ok(())
}

fn out(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn running_as_root() -> bool {
    // `id -u` avoids an unsafe libc call in the test.
    Command::new("id")
        .arg("-u")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "0")
        .unwrap_or(false)
}

/// Gives a directory its write bit back so the tempdir can be removed even
/// when an assertion fails.
struct RestoreMode(PathBuf);
impl Drop for RestoreMode {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
    }
}
