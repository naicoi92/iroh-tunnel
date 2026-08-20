//! Serve status file `connections` observability (issue #57), end-to-end.
//!
//! Boots serve + access against an IN-PROCESS relay (same harness shape as
//! `relay_auth.rs`: `iroh::test_utils::run_relay_server`, self-signed cert
//! trusted via the crate's `test-utils` feature) — no Internet, no n0 relay
//! network, CI-safe. The serve role's state dir is *injected* via
//! [`iroh_tunnel::serve::run_with_shutdown_with_state_dir`] (no
//! process-global env mutation), so the test reads a real
//! `serve-status.json` exactly as an operator would.
//!
//! Phases:
//!
//! 1. A payload round-trips through the tunnel (connection established).
//! 2. `serve-status.json` gains a `connections[0]` entry for the access
//!    node id (pinned key), carrying the `echo` service, non-empty
//!    transports, and endpoint-wide `local_bound_addrs`.
//! 3. After the access role shuts down, the entry disappears — untracking
//!    works and the 5 s flush picks it up.

#![cfg(unix)] // shutdown.rs installs SIGTERM handlers; restrict to unix

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use iroh::test_utils::run_relay_server;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

use iroh::SecretKey;
use iroh_tunnel::config::encode_secret_key;

/// Bytes the tunnel must transport unchanged in each direction.
const PAYLOAD: &[u8] = b"hello-through-the-status-file";

/// How long to wait for the 5 s status flush to reflect a change.
const STATUS_POLL_TIMEOUT: Duration = Duration::from_secs(25);

/// A role task shared with [`retry_connect`]: the connect loop polls it for
/// early exit (config error, bind race) so a dead role surfaces fast with
/// its real error instead of spinning to the deadline.
type SharedRole = Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<Result<()>>>>>;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn serve_status_file_reports_connected_peer_paths() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_test_writer()
        .try_init();

    let cfg_tmp = tempfile::tempdir().context("config tempdir")?;
    let state_tmp = tempfile::tempdir().context("state tempdir")?;

    let (_relay_map, relay_url, _server) = run_relay_server().await?;
    let relay = relay_url.to_string();

    // Echo service behind serve.
    let echo_listener = TcpListener::bind("127.0.0.1:0").await?;
    let echo_addr = echo_listener.local_addr()?;
    let echo = tokio::spawn(echo_server(echo_listener));

    // Pinned keys so both node ids are known up front.
    let serve_key = SecretKey::generate();
    let serve_node_id = serve_key.public().to_string();
    let access_key = SecretKey::generate();
    let access_node_id = access_key.public().to_string();

    let serve_cfg = cfg_tmp.path().join("serve.toml");
    std::fs::write(
        &serve_cfg,
        format!(
            r#"
[node]
secret_key = "{serve_key}"
relay_urls = ["{relay}"]

[[services]]
name = "echo"
protocol = "tcp"
host = "{host}"
port = {port}
"#,
            serve_key = encode_secret_key(&serve_key),
            host = echo_addr.ip(),
            port = echo_addr.port(),
        ),
    )?;

    // Bind the access port up front (access rebinds inside run_with_shutdown).
    let access_listener = TcpListener::bind("127.0.0.1:0").await?;
    let access_addr = access_listener.local_addr()?;
    drop(access_listener);

    let access_cfg = cfg_tmp.path().join("access.toml");
    std::fs::write(
        &access_cfg,
        format!(
            r#"
[node]
secret_key = "{access_key}"
relay_urls = ["{relay}"]

[[services]]
name = "echo"
node_id = "{serve_node_id}"
protocol = "tcp"
host = "127.0.0.1"
port = {port}
multiplex = true
"#,
            access_key = encode_secret_key(&access_key),
            port = access_addr.port(),
        ),
    )?;

    let (serve_tx, serve_rx) = oneshot::channel::<()>();
    let (access_tx, access_rx) = oneshot::channel::<()>();
    let serve_spawn = {
        let path = serve_cfg.clone();
        let state_dir = state_tmp.path().to_path_buf();
        tokio::spawn(async move {
            iroh_tunnel::serve::run_with_shutdown_with_state_dir(&state_dir, &path, async move {
                let _ = serve_rx.await;
            })
            .await
        })
    };
    let access_spawn = {
        let path = access_cfg.clone();
        tokio::spawn(async move {
            iroh_tunnel::access::run_with_shutdown(&path, async move {
                let _ = access_rx.await;
            })
            .await
        })
    };
    let serve_role: SharedRole = Arc::new(tokio::sync::Mutex::new(Some(serve_spawn)));
    let access_role: SharedRole = Arc::new(tokio::sync::Mutex::new(Some(access_spawn)));

    // Phase 1 — the tunnel works (this also establishes the connection the
    // status file must then report).
    let mut client = retry_connect(
        access_addr,
        Duration::from_secs(30),
        &serve_role,
        &access_role,
    )
    .await?;
    client.write_all(PAYLOAD).await?;
    client.flush().await?;
    let mut got = vec![0u8; PAYLOAD.len()];
    tokio::time::timeout(Duration::from_secs(10), client.read_exact(&mut got))
        .await
        .context("no echo through the in-process relay within 10s")?
        .context("echo read failed")?;
    assert_eq!(got.as_slice(), PAYLOAD);

    // Phase 2 — serve-status.json reports the access peer.
    let status_path = state_tmp.path().join("serve-status.json");
    let with_peer = poll_status(
        &status_path,
        STATUS_POLL_TIMEOUT,
        &[&serve_role, &access_role],
        |status| {
            !status["connections"]
                .as_array()
                .map(Vec::is_empty)
                .unwrap_or(true)
        },
    )
    .await?;
    assert_eq!(with_peer["node_id"], serve_node_id);

    let conns = with_peer["connections"].as_array().unwrap();
    assert_eq!(conns.len(), 1, "exactly one connected peer expected");
    let conn = &conns[0];
    assert_eq!(
        conn["peer"], access_node_id,
        "peer must be the access node id"
    );
    assert_eq!(conn["services"], serde_json::json!(["echo"]));
    let transports = conn["transports"].as_array().unwrap();
    assert!(!transports.is_empty(), "transports must not be empty");
    for t in transports {
        let kind = t["kind"].as_str().unwrap_or_default();
        assert!(
            kind == "relay" || kind == "direct",
            "transport kind must be relay|direct, got {kind:?}"
        );
        assert!(!t["addr"].as_str().unwrap_or_default().is_empty());
        assert!(t["active"].is_boolean(), "active must be a boolean");
    }
    assert!(
        !conn["local_bound_addrs"].as_array().unwrap().is_empty(),
        "local_bound_addrs must list the endpoint's UDP candidates"
    );

    println!(
        "serve-status.json connections:\n{}",
        serde_json::to_string_pretty(&with_peer["connections"]).unwrap()
    );

    // Phase 3 — access goes away, the peer entry disappears.
    drop(client);
    let _ = access_tx.send(());
    finish_role(&access_role, "access").await?;

    let emptied = poll_status(
        &status_path,
        STATUS_POLL_TIMEOUT,
        &[&serve_role],
        |status| {
            status["connections"]
                .as_array()
                .map(Vec::is_empty)
                .unwrap_or(true)
        },
    )
    .await?;
    assert_eq!(
        emptied["connections"].as_array().unwrap().len(),
        0,
        "peer must be untracked after its connection closes"
    );

    let _ = serve_tx.send(());
    finish_role(&serve_role, "serve").await?;
    echo.abort();
    Ok(())
}

/// If the role task has already exited, take and resolve it, returning the
/// result; `None` while it is still running.
async fn role_exited(role: &SharedRole) -> Option<Result<()>> {
    let mut guard = role.lock().await;
    match guard.as_ref() {
        Some(handle) if handle.is_finished() => {
            // Take the handle out and DROP the lock before awaiting: a guard
            // held across the await would block `retry_connect` and
            // `finish_role` on this slot if a future edit ever awaits an
            // unfinished handle here — a fast-fail path must not turn into
            // a hang.
            let handle = guard.take().expect("checked is_finished above");
            drop(guard);
            Some(handle.await.expect("role task panicked"))
        }
        _ => None,
    }
}

/// Take a role task out of its slot and wait for clean shutdown.
async fn finish_role(role: &SharedRole, name: &str) -> Result<()> {
    let handle = role
        .lock()
        .await
        .take()
        .with_context(|| format!("{name} task already consumed"))?;
    tokio::time::timeout(Duration::from_secs(15), handle)
        .await
        .with_context(|| format!("{name} did not shut down within 15s"))?
        .with_context(|| format!("{name} task panicked"))?
        .with_context(|| format!("{name} returned error"))?;
    Ok(())
}

/// Re-read the status file until `pred` accepts it, returning the last parse.
///
/// `watch` lists role tasks that must still be running while we poll: if
/// one exits, its real result fails the test immediately instead of an
/// opaque timeout later.
async fn poll_status(
    path: &std::path::Path,
    timeout: Duration,
    watch: &[&SharedRole],
    pred: impl Fn(&serde_json::Value) -> bool,
) -> Result<serde_json::Value> {
    let start = std::time::Instant::now();
    let mut last_parse: Option<serde_json::Value> = None;
    loop {
        // Fast-fail: a watched role exiting mid-poll means the file will
        // never reach the expected state — surface the role's real result.
        for role in watch {
            if let Some(result) = role_exited(role).await {
                anyhow::bail!("watched role exited while waiting for the status file: {result:?}");
            }
        }
        if let Ok(body) = std::fs::read_to_string(path) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) {
                if pred(&value) {
                    return Ok(value);
                }
                last_parse = Some(value);
            }
        }
        if start.elapsed() > timeout {
            anyhow::bail!(
                "status file did not reach the expected state within {timeout:?}; last seen: {}",
                last_parse
                    .map(|v| serde_json::to_string_pretty(&v).unwrap_or_default())
                    .unwrap_or_else(|| "<unreadable>".to_string())
            );
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
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

/// Retry TCP connect until `deadline`, polling both role tasks so an early
/// exit (config error, bind race) fails fast with the role's real error
/// instead of a misleading connect timeout.
async fn retry_connect(
    addr: std::net::SocketAddr,
    deadline: Duration,
    serve_role: &SharedRole,
    access_role: &SharedRole,
) -> Result<TcpStream> {
    let start = std::time::Instant::now();
    loop {
        if let Some(res) = role_exited(serve_role).await {
            anyhow::bail!("serve exited early: {res:?}");
        }
        if let Some(res) = role_exited(access_role).await {
            anyhow::bail!("access exited early: {res:?}");
        }
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
