//! Configured foreground ownership scopes, exercised with real processes.

use anyhow::Result;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct World(tempfile::TempDir);
impl World {
    fn spawn(&self, name: &str, relay: &str, scope: Option<&Path>) -> Result<(Process, PathBuf)> {
        let root = self.0.path();
        let state = root.join(format!("state-{name}"));
        std::fs::create_dir_all(&state)?;
        let config = root.join(format!("{name}.toml"));
        let key = iroh::SecretKey::generate();
        std::fs::write(&config, format!(
            "[node]\nsecret_key = {:?}\nrelay_urls = [{relay:?}]\n\n[[services]]\nname = \"echo\"\nhost = \"127.0.0.1\"\nport = 9\nprotocol = \"tcp\"\n",
            iroh_tunnel_host::config::encode_secret_key(&key)
        ))?;
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_iroh-tunnel-serve"));
        cmd.args(["run", "--config"])
            .arg(&config)
            .env("HOME", root.join("home"))
            .env("IROH_TUNNEL_MACHINE_LOCK_DIR", root.join("machine"))
            .env("IROH_TUNNEL_STATE_DIR", &state)
            .env_remove("IROH_TUNNEL_ACTION_SCOPE")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(scope) = scope {
            cmd.env("IROH_TUNNEL_ACTION_SCOPE", scope);
        }
        Ok((Process(cmd.spawn()?), state.join("serve-status.json")))
    }
}

fn running(process: &mut Process, status: &Path) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if std::fs::read(status)
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .is_some()
        {
            return Ok(());
        }
        if let Some(exit) = process.0.try_wait()? {
            anyhow::bail!("exited before ready: {exit}");
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "timed out waiting for {}",
            status.display()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn refused(process: &mut Process) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(exit) = process.0.try_wait()? {
            anyhow::ensure!(exit.code() == Some(6), "expected conflict exit, got {exit}");
            return Ok(());
        }
        anyhow::ensure!(Instant::now() < deadline, "contender did not exit");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[tokio::test]
async fn separate_action_scopes_coexist() -> Result<()> {
    let (_, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let world = World(tempfile::tempdir()?);
    let (mut a, sa) = world.spawn(
        "a",
        &relay.to_string(),
        Some(&world.0.path().join("scope-a")),
    )?;
    running(&mut a, &sa)?;
    let (mut b, sb) = world.spawn(
        "b",
        &relay.to_string(),
        Some(&world.0.path().join("scope-b")),
    )?;
    running(&mut b, &sb)
}

#[tokio::test]
async fn same_action_scope_refuses_second_owner() -> Result<()> {
    let (_, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let world = World(tempfile::tempdir()?);
    let scope = world.0.path().join("scope");
    let (mut a, sa) = world.spawn("a", &relay.to_string(), Some(&scope))?;
    running(&mut a, &sa)?;
    let (mut b, _) = world.spawn("b", &relay.to_string(), Some(&scope))?;
    refused(&mut b)
}

#[tokio::test]
async fn racing_action_starts_admit_exactly_one_owner() -> Result<()> {
    let (_, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let world = World(tempfile::tempdir()?);
    let scope = world.0.path().join("scope");
    let mut processes = Vec::new();
    for i in 0..4 {
        processes.push(world.spawn(&i.to_string(), &relay.to_string(), Some(&scope))?);
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let mut owners = 0;
        let mut conflicts = 0;
        for (process, status) in &mut processes {
            if let Some(exit) = process.0.try_wait()? {
                anyhow::ensure!(exit.code() == Some(6), "expected conflict exit, got {exit}");
                conflicts += 1;
            } else if std::fs::read(status)
                .ok()
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                .is_some()
            {
                owners += 1;
            }
        }
        if owners == 1 && conflicts == 3 {
            return Ok(());
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "expected one owner and three conflicts, got {owners} and {conflicts}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[tokio::test]
async fn state_override_does_not_isolate_ordinary_runs() -> Result<()> {
    let (_, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let world = World(tempfile::tempdir()?);
    let (mut a, sa) = world.spawn("a", &relay.to_string(), None)?;
    running(&mut a, &sa)?;
    let (mut b, _) = world.spawn("b", &relay.to_string(), None)?;
    refused(&mut b)
}
