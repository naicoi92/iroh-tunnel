//! Cooperative shutdown is scoped to one action workspace.
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

fn wait_for(path: &Path, child: &mut Process) -> Result<()> {
    let end = Instant::now() + Duration::from_secs(30);
    while !path.exists() {
        anyhow::ensure!(
            child.0.try_wait()?.is_none(),
            "child exited before {}",
            path.display()
        );
        anyhow::ensure!(Instant::now() < end, "timed out: {}", path.display());
        std::thread::sleep(Duration::from_millis(25));
    }
    Ok(())
}

fn wait_exit(child: &mut Process) -> Result<std::process::ExitStatus> {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.0.try_wait()? {
            return Ok(status);
        }
        anyhow::ensure!(Instant::now() < end, "child did not exit");
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn spawn(root: &Path, name: &str, relay: &str) -> Result<(Process, PathBuf)> {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir)?;
    let config = dir.join("config.toml");
    let key = iroh::SecretKey::generate();
    std::fs::write(&config, format!(
        "[node]\nsecret_key = {:?}\nrelay_urls = [{relay:?}]\n\n[[services]]\nname = \"echo\"\nhost = \"127.0.0.1\"\nport = 9\nprotocol = \"tcp\"\n",
        iroh_tunnel_host::config::encode_secret_key(&key)
    ))?;
    let child = Command::new(env!("CARGO_BIN_EXE_iroh-tunnel-serve"))
        .args(["run", "--config"])
        .arg(config)
        .env("HOME", &dir)
        .env("IROH_TUNNEL_MACHINE_LOCK_DIR", root.join("machine"))
        .env("IROH_TUNNEL_STATE_DIR", &dir)
        .env("IROH_TUNNEL_ACTION_SCOPE", &dir)
        .env("IROH_TUNNEL_ACTION_SHUTDOWN_DIR", &dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok((Process(child), dir))
}

#[tokio::test]
async fn request_stops_only_its_own_process() -> Result<()> {
    let (_, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let temp = tempfile::tempdir()?;
    let (mut a, da) = spawn(temp.path(), "a", &relay.to_string())?;
    let (mut b, db) = spawn(temp.path(), "b", &relay.to_string())?;
    wait_for(&da.join("serve-status.json"), &mut a)?;
    wait_for(&db.join("serve-status.json"), &mut b)?;
    std::fs::write(da.join("shutdown-request"), "shutdown")?;
    wait_for(&da.join("shutdown-complete"), &mut a)?;
    anyhow::ensure!(b.0.try_wait()?.is_none(), "other invocation exited");
    anyhow::ensure!(!db.join("shutdown-complete").exists());
    std::fs::write(db.join("shutdown-request"), "shutdown")?;
    wait_for(&db.join("shutdown-complete"), &mut b)?;
    Ok(())
}

#[tokio::test]
async fn duplicate_channel_cannot_confirm_the_original_has_stopped() -> Result<()> {
    let (_, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let temp = tempfile::tempdir()?;
    let (mut original, dir) = spawn(temp.path(), "invocation", &relay.to_string())?;
    wait_for(&dir.join("serve-status.json"), &mut original)?;
    let (mut duplicate, _) = spawn(temp.path(), "invocation", &relay.to_string())?;
    anyhow::ensure!(!wait_exit(&mut duplicate)?.success());
    anyhow::ensure!(original.0.try_wait()?.is_none(), "original exited");
    anyhow::ensure!(
        !dir.join("shutdown-complete").exists(),
        "a duplicate must not confirm shutdown of the original"
    );
    std::fs::write(dir.join("shutdown-request"), "shutdown")?;
    anyhow::ensure!(wait_exit(&mut original)?.success());
    anyhow::ensure!(std::fs::read(dir.join("shutdown-complete"))? == b"complete");
    Ok(())
}

#[test]
fn failed_start_still_confirms_shutdown() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let dir = temp.path();
    let config = dir.join("invalid.toml");
    std::fs::write(&config, "invalid TOML [")?;
    let status = Command::new(env!("CARGO_BIN_EXE_iroh-tunnel-serve"))
        .args(["run", "--config"])
        .arg(config)
        .env("IROH_TUNNEL_MACHINE_LOCK_DIR", dir.join("machine"))
        .env("IROH_TUNNEL_STATE_DIR", dir)
        .env("IROH_TUNNEL_ACTION_SCOPE", dir)
        .env("IROH_TUNNEL_ACTION_SHUTDOWN_DIR", dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    anyhow::ensure!(!status.success());
    anyhow::ensure!(std::fs::read(dir.join("shutdown-complete"))? == b"complete");
    Ok(())
}
