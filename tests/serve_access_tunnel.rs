//! Characterization test: pin the serve↔access tunnel end-to-end.
//!
//! Boot an in-process serve (publishing a TCP echo service via Iroh) and an
//! in-process access (consuming it locally), then pipe bytes through the
//! tunnel from a TCP client and assert they round-trip unchanged.
//!
//! ## Why `#[ignore]`
//!
//! iroh's `Minimal` preset requires a relay server to dial a peer, and we use
//! the n0 default public relays as the fallback when `relay_urls` is empty.
//! That makes this test depend on the public Internet and the n0 relay
//! network, which would be flaky in CI. Run it locally with:
//!
//! ```sh
//! cargo test --test serve_access_tunnel -- --ignored --nocapture
//! ```
//!
//! ## Purpose
//!
//! This test is the safety net for the refactors tracked in
//! [#26](https://github.com/naicoi92/iroh-tunnel/issues/26) — specifically it
//! unlocks #31 (endpoint absorb) and #32 (role-run strategy), which touch the
//! serve/access runtime. If those refactors silently change tunnel behavior,
//! this test will fail.

#![cfg(unix)] // shutdown.rs installs SIGTERM handlers; restrict to unix

use std::time::Duration;

use anyhow::{Context, Result};
use iroh::SecretKey;
use iroh_tunnel::config::{encode_secret_key, ServeConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

/// Bytes the tunnel must transport unchanged in each direction. Small enough
/// to fit one copy_buf flush, large enough to exercise the real path.
const PAYLOAD_OUT: &[u8] = b"hello-through-the-tunnel";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires Internet + n0 relay network; run with --ignored"]
async fn serve_access_tunnel_roundtrips_bytes() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_test_writer()
        .try_init();

    let tmp = tempfile::tempdir().context("tempdir")?;

    // 1. Stand up the local TCP service that `serve` will publish: an echo
    //    server that loops read→write on each accepted connection.
    let echo_listener = TcpListener::bind("127.0.0.1:0").await?;
    let echo_addr = echo_listener.local_addr()?;
    let echo_task = tokio::spawn(echo_server(echo_listener));

    // 2. Write a serve config that points at the echo server. We pin a stable
    //    serve secret key up front so we can read the NodeId before launching
    //    serve::run_with_shutdown — access.toml needs that NodeId.
    let serve_cfg_path = tmp.path().join("serve.toml");
    let serve_key = SecretKey::generate();
    let serve_node_id = serve_key.public().to_string();
    {
        let toml = format!(
            r#"
[node]
secret_key = "{key}"

[[services]]
name = "echo"
protocol = "tcp"
host = "{host}"
port = {port}
"#,
            key = encode_secret_key(&serve_key),
            host = echo_addr.ip(),
            port = echo_addr.port(),
        );
        std::fs::write(&serve_cfg_path, toml)?;
    }

    // 3. Bind the access listener port up front so we can write it into
    //    access.toml. Drop immediately — access rebinds from inside
    //    run_with_shutdown.
    let access_listener = TcpListener::bind("127.0.0.1:0").await?;
    let access_addr = access_listener.local_addr()?;
    drop(access_listener);

    let access_cfg_path = tmp.path().join("access.toml");
    std::fs::write(
        &access_cfg_path,
        format!(
            r#"
[[services]]
name = "echo"
node_id = "{node_id}"
protocol = "tcp"
host = "127.0.0.1"
port = {port}
"#,
            node_id = serve_node_id,
            port = access_addr.port(),
        ),
    )?;

    // 4. Launch serve and access with injected shutdown signals.
    let (serve_tx, serve_rx) = oneshot::channel::<()>();
    let (access_tx, access_rx) = oneshot::channel::<()>();

    let serve_handle = {
        let path = serve_cfg_path.clone();
        tokio::spawn(async move {
            iroh_tunnel::serve::run_with_shutdown(&path, async move {
                let _ = serve_rx.await;
            })
            .await
        })
    };
    let access_handle = {
        let path = access_cfg_path.clone();
        tokio::spawn(async move {
            iroh_tunnel::access::run_with_shutdown(&path, async move {
                let _ = access_rx.await;
            })
            .await
        })
    };

    // 5. Wait for the access listener to come up, then pipe bytes through it.
    //    The relay handshake + first dial can take a few seconds; retry TCP
    //    connect with backoff.
    let mut client = retry_connect(access_addr, Duration::from_secs(30)).await?;
    client.write_all(PAYLOAD_OUT).await?;
    client.flush().await?;

    let mut got = vec![0u8; PAYLOAD_OUT.len()];
    client.read_exact(&mut got).await?;
    assert_eq!(
        got.as_slice(),
        PAYLOAD_OUT,
        "tunnel did not deliver bytes intact"
    );

    // 6. Shutdown both roles gracefully and assert they exited cleanly.
    let _ = serve_tx.send(());
    let _ = access_tx.send(());

    let serve_res = tokio::time::timeout(Duration::from_secs(15), serve_handle)
        .await
        .context("serve did not shut down within 15s")?
        .context("serve task panicked")?
        .context("serve returned error");
    let access_res = tokio::time::timeout(Duration::from_secs(15), access_handle)
        .await
        .context("access did not shut down within 15s")?
        .context("access task panicked")?
        .context("access returned error");

    echo_task.abort();

    // Defer the results to the end so partial failures still get reported.
    serve_res?;
    access_res?;
    Ok(())
}

/// Echo server: loop read→write on each accepted connection until EOF.
async fn echo_server(listener: TcpListener) {
    loop {
        match listener.accept().await {
            Ok((mut sock, peer)) => {
                tracing::info!(%peer, "echo: client connected");
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    loop {
                        match sock.read(&mut buf).await {
                            Ok(0) => break,
                            Ok(n) => {
                                if sock.write_all(&buf[..n]).await.is_err() {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                });
            }
            Err(e) => {
                tracing::warn!(%e, "echo: accept failed, exiting");
                break;
            }
        }
    }
}

/// Retry TCP connect to `addr` until success or `deadline` elapses.
///
/// The access listener is bound asynchronously inside `run_with_shutdown`,
/// and the first dial through the relay takes time. We retry the connect so
/// the test isn't racy on startup ordering.
async fn retry_connect(addr: std::net::SocketAddr, deadline: Duration) -> Result<TcpStream> {
    let start = std::time::Instant::now();
    loop {
        match TcpStream::connect(addr).await {
            Ok(s) => return Ok(s),
            Err(e) => {
                if start.elapsed() > deadline {
                    anyhow::bail!("could not connect to access {addr} within {deadline:?}: {e}");
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
}

// Keep the unused ServeConfig import warning-free — referenced only for the
// docstring link above.
#[allow(dead_code)]
fn _config_type_anchor() -> Option<ServeConfig> {
    None
}
