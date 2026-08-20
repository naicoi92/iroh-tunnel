//! Relay authentication + per-service relay overrides, end-to-end.
//!
//! Unlike `serve_access_tunnel.rs`, these tests run against an IN-PROCESS
//! relay (`iroh::test_utils::run_relay_server_with_access`) with a token
//! access control — no Internet, no n0 relay network, CI-safe.
//!
//! ## Suites
//!
//! 1. `tokened_relay_roundtrips_bytes` — serve and access both configure
//!    `relay_urls` + a matching `relay_token`; a payload round-trips through
//!    the tunnel (issue #53: the token must reach the relay's
//!    `Authorization: Bearer` check on both roles).
//! 2. `wrong_token_never_roundtrips` — the access side carries a WRONG
//!    token; its relay connection is denied, so dials through the relay
//!    cannot complete and no echo ever arrives (the local TCP connection
//!    stays open — the dial retry loop backs off silently).
//! 3. `per_service_relay_override_roundtrips` — the access `[node]` has NO
//!    relay_urls; one service overrides `relay_urls` to the test relay
//!    (issue #54). The roundtrip only works if the endpoint joins the
//!    service's relay (union) AND the dial attaches the override — with the
//!    override ignored, the fallback would be the n0 farm, where the serve
//!    peer is not registered, and the echo would never arrive.
//!
//! The self-signed relay certificate is trusted via the crate's own
//! `test-utils` feature (see `endpoint.rs`), mirroring how iroh's own tests
//! drive `run_relay_server`.

#![cfg(unix)] // shutdown.rs installs SIGTERM handlers; restrict to unix

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use iroh::test_utils::run_relay_server_with_access;
use iroh_relay::server::{Access, AccessControl, ClientRequest};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

use iroh::SecretKey;
use iroh_tunnel::config::encode_secret_key;

/// Bytes the tunnel must transport unchanged in each direction.
const PAYLOAD: &[u8] = b"hello-through-the-tokened-relay";

/// The token the relay demands and (in the happy path) both roles carry.
const TOKEN: &str = "test-relay-token";

/// Admits a connection only when it presents [`TOKEN`] — the same check a
/// self-hosted relay performs for `access.shared_token` (see
/// docs/self-hosted-relay.md).
#[derive(Debug)]
struct TokenAccess(&'static str);

impl AccessControl for TokenAccess {
    async fn on_connect(&self, request: &ClientRequest) -> Access {
        if request.auth_token().as_deref() == Some(self.0) {
            Access::Allow
        } else {
            Access::Deny { reason: None }
        }
    }
}

// ---------------------------------------------------------------------------
// Suite 1 — correct token on both roles
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tokened_relay_roundtrips_bytes() -> Result<()> {
    let (_relay_map, relay_url, _server) =
        run_relay_server_with_access(false, Arc::new(TokenAccess(TOKEN))).await?;
    let relay = relay_url.to_string();
    let (access_addr, task, echo) = boot_serve_and_access(&relay, TOKEN, &relay, TOKEN).await?;

    let mut client = retry_connect(access_addr, Duration::from_secs(30)).await?;
    client.write_all(PAYLOAD).await?;
    client.flush().await?;
    let mut got = vec![0u8; PAYLOAD.len()];
    client
        .read_exact(&mut got)
        .await
        .context("no echo through the tokened relay")?;
    assert_eq!(got.as_slice(), PAYLOAD);

    task.shutdown().await?;
    echo.abort();
    Ok(())
}

// ---------------------------------------------------------------------------
// Suite 2 — wrong token on the access side
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wrong_token_never_roundtrips() -> Result<()> {
    let (_relay_map, relay_url, _server) =
        run_relay_server_with_access(false, Arc::new(TokenAccess(TOKEN))).await?;
    let relay = relay_url.to_string();

    // Serve carries the correct token (it registers on the relay); access
    // carries a WRONG one, so its relay connection is denied.
    let (access_addr, task, echo) =
        boot_serve_and_access(&relay, TOKEN, &relay, "wrong-token").await?;

    let mut client = retry_connect(access_addr, Duration::from_secs(30)).await?;
    client.write_all(PAYLOAD).await?;
    client.flush().await?;

    let mut got = vec![0u8; PAYLOAD.len()];
    let read = tokio::time::timeout(Duration::from_secs(5), client.read_exact(&mut got)).await;
    assert!(
        read.is_err(),
        "unexpected echo despite a denied relay token — got {:?}",
        read.map(|r| r.map(|_| ()))
    );

    // The dial retry loop backs off forever; dropping the client and the
    // shutdown signal is the graceful way out for this suite.
    drop(client);
    task.shutdown().await?;
    echo.abort();
    Ok(())
}

// ---------------------------------------------------------------------------
// Suite 3 — per-service relay override (issue #54)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn per_service_relay_override_roundtrips() -> Result<()> {
    let (_relay_map, relay_url, _server) =
        run_relay_server_with_access(false, Arc::new(TokenAccess(TOKEN))).await?;
    let relay = relay_url.to_string();

    // Serve registers on the relay; the access NODE has no relay_urls at
    // all — only the service override points at the relay.
    let (access_addr, task, echo) = boot_serve_and_access(&relay, TOKEN, "", TOKEN).await?;

    let mut client = retry_connect(access_addr, Duration::from_secs(30)).await?;
    client.write_all(PAYLOAD).await?;
    client.flush().await?;
    let mut got = vec![0u8; PAYLOAD.len()];
    client
        .read_exact(&mut got)
        .await
        .context("no echo through the per-service relay override")?;
    assert_eq!(got.as_slice(), PAYLOAD);

    task.shutdown().await?;
    echo.abort();
    Ok(())
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// What [`boot_serve_and_access`] leaves running.
struct Roles {
    serve_tx: oneshot::Sender<()>,
    serve_handle: tokio::task::JoinHandle<Result<()>>,
    access_tx: oneshot::Sender<()>,
    access_handle: tokio::task::JoinHandle<Result<()>>,
    /// Keeps the configs' tempdir alive for as long as the roles run —
    /// `resolve_and_save_key` writes back into it.
    _tmp: tempfile::TempDir,
}

impl Roles {
    /// Signal shutdown and assert both roles exited cleanly.
    async fn shutdown(self) -> Result<()> {
        let Roles {
            serve_tx,
            serve_handle,
            access_tx,
            access_handle,
            _tmp: _,
        } = self;
        let _ = serve_tx.send(());
        let _ = access_tx.send(());
        let serve_res = tokio::time::timeout(Duration::from_secs(15), serve_handle)
            .await
            .context("serve did not shut down within 15s")?
            .context("serve task panicked")?;
        let access_res = tokio::time::timeout(Duration::from_secs(15), access_handle)
            .await
            .context("access did not shut down within 15s")?
            .context("access task panicked")?;
        serve_res.context("serve returned error")?;
        access_res.context("access returned error")?;
        Ok(())
    }
}

/// Stand up an echo service behind `serve`, an `access` tunnel to it, and
/// return the local address to connect to plus the running tasks.
///
/// `serve_relays` / `serve_token` configure the serve role; `access_relays`
/// configures the access NODE relay list ("" → no `[node] relay_urls`) and
/// `access_token` the node token. The access service always carries a
/// per-service `relay_urls` override equal to `access_relays` when the node
/// list is empty (suite 3), and no override otherwise.
#[allow(clippy::too_many_arguments)]
async fn boot_serve_and_access(
    serve_relays: &str,
    serve_token: &str,
    access_relays: &str,
    access_token: &str,
) -> Result<(std::net::SocketAddr, Roles, tokio::task::JoinHandle<()>)> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_test_writer()
        .try_init();

    let tmp = tempfile::tempdir().context("tempdir")?;

    // Echo service behind serve.
    let echo_listener = TcpListener::bind("127.0.0.1:0").await?;
    let echo_addr = echo_listener.local_addr()?;
    let echo = tokio::spawn(echo_server(echo_listener));

    // Serve config: pinned key (so the NodeId is known up front), relay +
    // token as given.
    let serve_key = SecretKey::generate();
    let serve_node_id = serve_key.public().to_string();
    let serve_node_block = if serve_relays.is_empty() {
        format!(r#"secret_key = "{}""#, encode_secret_key(&serve_key))
    } else {
        format!(
            r#"secret_key = "{}"
relay_urls = ["{serve_relays}"]
relay_token = "{serve_token}""#,
            encode_secret_key(&serve_key)
        )
    };
    let serve_cfg_path = tmp.path().join("serve.toml");
    std::fs::write(
        &serve_cfg_path,
        format!(
            r#"
[node]
{serve_node_block}

[[services]]
name = "echo"
protocol = "tcp"
host = "{host}"
port = {port}
"#,
            host = echo_addr.ip(),
            port = echo_addr.port(),
        ),
    )?;

    // Bind the access port up front (access rebinds inside run_with_shutdown).
    let access_listener = TcpListener::bind("127.0.0.1:0").await?;
    let access_addr = access_listener.local_addr()?;
    drop(access_listener);

    // Access config: node relays/token as given; the service override is
    // set exactly when the node list is empty (the per-service suite).
    let access_node_block = if access_relays.is_empty() {
        format!(r#"relay_token = "{access_token}""#)
    } else {
        format!(
            r#"relay_urls = ["{access_relays}"]
relay_token = "{access_token}""#
        )
    };
    // The service-level relay override is set exactly when the access node
    // has no relay_urls of its own (suite 3) — pointing the service at the
    // relay its serve peer registered on.
    let service_override = if access_relays.is_empty() {
        format!(r#"relay_urls = ["{serve_relays}"]"#)
    } else {
        String::new()
    };
    let access_cfg_path = tmp.path().join("access.toml");
    std::fs::write(
        &access_cfg_path,
        format!(
            r#"
[node]
{access_node_block}

[[services]]
name = "echo"
node_id = "{node_id}"
protocol = "tcp"
host = "127.0.0.1"
port = {port}
{service_override}
multiplex = true
"#,
            node_id = serve_node_id,
            port = access_addr.port(),
        ),
    )?;

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

    let roles = Roles {
        serve_tx,
        serve_handle,
        access_tx,
        access_handle,
        _tmp: tmp,
    };
    Ok((access_addr, roles, echo))
}

/// Echo server: loop read→write on each accepted connection until EOF.
async fn echo_server(listener: TcpListener) {
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            loop {
                match stream.read(&mut buf).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => {
                        if stream.write_all(&buf[..n]).await.is_err() {
                            return;
                        }
                    }
                }
            }
        });
    }
}

/// Retry TCP connect until `deadline` so the test isn't racy on startup.
async fn retry_connect(addr: std::net::SocketAddr, deadline: Duration) -> Result<TcpStream> {
    let start = std::time::Instant::now();
    loop {
        match TcpStream::connect(addr).await {
            Ok(s) => return Ok(s),
            Err(e) => {
                if start.elapsed() > deadline {
                    anyhow::bail!("could not connect to {addr} within {deadline:?}: {e}");
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}
