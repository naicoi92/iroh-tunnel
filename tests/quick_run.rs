//! #81: real quick-run processes and TCP fixture. The `test-utils` feature
//! disables TLS verification for the whole test binary; use only isolated
//! local relay fixtures, never secrets or public relays, and do not distribute it.
//! Production builds must not enable that feature.
//!
//! Quick flags: -n/--name, -t/--target, -l/--listen, -N/--node-id,
//! -p/--protocol (TCP default), and --relay-url.

use anyhow::{Context, Result};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

async fn status(path: &std::path::Path, child: &mut Process) -> Result<serde_json::Value> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(data) = std::fs::read(path) {
            if let Ok(json) = serde_json::from_slice(&data) {
                return Ok(json);
            }
        }
        if let Some(exit) = child.0.try_wait()? {
            anyhow::bail!("role exited: {exit}");
        }
        if Instant::now() > deadline {
            anyhow::bail!("status timeout: {}", path.display());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn role(home: &std::path::Path, state: &std::path::Path, args: &[&str]) -> Result<Process> {
    Ok(Process(
        Command::new(env!("CARGO_BIN_EXE_iroh-tunnel"))
            .args(args)
            .env("HOME", home)
            .env("IROH_TUNNEL_STATE_DIR", state)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?,
    ))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn quick_tcp_forwards_bytes_to_fixed_target() -> Result<()> {
    tcp_roundtrip(false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn quick_short_flags_forward_bytes_to_fixed_target() -> Result<()> {
    tcp_roundtrip(true).await
}

async fn tcp_roundtrip(short: bool) -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let relay = relay.to_string();
    let target = TcpListener::bind("127.0.0.1:0").await?;
    let target_addr = target.local_addr()?.to_string();
    let fixture = tokio::spawn(async move {
        let (mut stream, _) = target.accept().await?;
        let mut request = [0u8; 7];
        stream.read_exact(&mut request).await?;
        anyhow::ensure!(&request == b"request", "wrong target request");
        stream.write_all(b"response").await?;
        Ok::<_, anyhow::Error>(())
    });
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let listen = listener.local_addr()?;
    drop(listener);
    let serve_args = if short {
        vec![
            "serve",
            "-n",
            "echo",
            "-t",
            &target_addr,
            "-p",
            "tcp",
            "--relay-url",
            &relay,
        ]
    } else {
        vec![
            "serve",
            "--name",
            "echo",
            "--target",
            &target_addr,
            "--relay-url",
            &relay,
        ]
    };
    let mut serve = role(temp.path(), temp.path(), &serve_args)?;
    let serve_status = status(&temp.path().join("serve-status.json"), &mut serve).await?;
    let node = serve_status["node_id"].as_str().context("serve node id")?;
    let listen_string = listen.to_string();
    let access_args = if short {
        vec![
            "access",
            "-n",
            "echo",
            "-l",
            &listen_string,
            "-N",
            node,
            "-p",
            "tcp",
            "--relay-url",
            &relay,
        ]
    } else {
        vec![
            "access",
            "--name",
            "echo",
            "--listen",
            &listen_string,
            "--node-id",
            node,
            "--relay-url",
            &relay,
        ]
    };
    let mut access = role(temp.path(), temp.path(), &access_args)?;
    status(&temp.path().join("access-status.json"), &mut access).await?;
    let mut client = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Ok(client) = TcpStream::connect(listen).await {
                break client;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await?;
    client.write_all(b"request").await?;
    let mut response = [0u8; 8];
    tokio::time::timeout(Duration::from_secs(30), client.read_exact(&mut response)).await??;
    assert_eq!(&response, b"response");
    fixture.await??;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn quick_roles_generate_ephemeral_identities_without_config() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let relay = relay.to_string();
    for role_name in ["serve", "access"] {
        let mut ids = Vec::new();
        for _ in 0..2 {
            let state = tempfile::tempdir()?;
            let args = if role_name == "serve" {
                vec![
                    "serve",
                    "-n",
                    "echo",
                    "-t",
                    "127.0.0.1:12345",
                    "--relay-url",
                    &relay,
                ]
            } else {
                vec![
                    "access",
                    "-n",
                    "echo",
                    "-l",
                    "127.0.0.1:12346",
                    "-N",
                    "0000000000000000000000000000000000000000000000000000000000000000",
                    "--relay-url",
                    &relay,
                ]
            };
            let mut process = role(temp.path(), state.path(), &args)?;
            let json = status(
                &state.path().join(format!("{role_name}-status.json")),
                &mut process,
            )
            .await?;
            ids.push(json["node_id"].as_str().context("node id")?.to_owned());
            drop(process);
        }
        assert_ne!(ids[0], ids[1], "{role_name} identity persisted");
    }
    assert!(!temp.path().join(".config/iroh-tunnel").exists());
    assert!(!temp
        .path()
        .join("Library/Application Support/iroh-tunnel")
        .exists());
    Ok(())
}

#[test]
fn invalid_quick_inputs_fail_without_config_or_key_writes() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let state = tempfile::tempdir()?;
    for args in [
        vec!["serve", "-n", "bad name", "-t", "127.0.0.1:12345"],
        vec!["serve", "-n", "echo", "-t", "not-an-address"],
        vec!["serve", "-n", "echo", "-t", "127.0.0.1:12345", "-p", "udp"],
        vec![
            "access",
            "-n",
            "echo",
            "-l",
            "127.0.0.1:12346",
            "-N",
            "invalid",
        ],
        vec![
            "access",
            "-n",
            "echo",
            "-l",
            "not-an-address",
            "-N",
            "invalid",
        ],
        vec!["access", "-n", "echo", "-l", "127.0.0.1:12346"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_iroh-tunnel"))
            .args(&args)
            .env("HOME", temp.path())
            .env("IROH_TUNNEL_STATE_DIR", state.path())
            .output()?;
        assert!(!output.status.success(), "accepted {args:?}");
        assert!(!output.stderr.is_empty(), "no diagnostic for {args:?}");
    }
    assert_eq!(std::fs::read_dir(temp.path())?.count(), 0);
    assert_eq!(std::fs::read_dir(state.path())?.count(), 0);
    Ok(())
}
