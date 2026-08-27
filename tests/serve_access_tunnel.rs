//! Characterization tests: pin the serve↔access tunnel end-to-end.
//!
//! Boot an in-process serve (publishing a TCP echo service via Iroh) and an
//! in-process access (consuming it locally), then pipe bytes through the
//! tunnel from TCP clients and assert they round-trip unchanged.
//!
//! ## Suites
//!
//! 1. `serve_access_tunnel_roundtrips_bytes` — the pre-0.2.0 single-channel
//!    behavior, pinned against the 0.2.0 serve (access- cũ ↔ serve-mới
//!    compatibility: the ALPN is unchanged and the serve still serves
//!    single-stream connections exactly as before).
//! 2. `multiplex_two_streams_share_one_connection` — 0.2.0 multiplexing:
//!    two concurrent local clients are carried as two bidirectional streams
//!    on ONE iroh connection (asserted via a counting mini-serve); closing
//!    one stream does not affect the other.
//! 3. `multiplex_false_matches_legacy_behavior` — access `multiplex = false`:
//!    two sequential channels, two separate connections — the pre-0.2.0
//!    behavior verbatim.
//! 4. `multiplex_survives_idle_beyond_timeout` — keep-alive: after 60 s with
//!    no traffic (noq's default idle timeout is 30 s), a new channel still
//!    rides the SAME connection and both channels keep echoing.
//!
//! ## Rollout contract (no negotiation)
//!
//! Multiplexing requires a 0.2.0+ serve peer; the ALPN carries no version
//! information. The unsupported combination (new access with multiplex
//! enabled vs an old one-stream-per-connection serve) is therefore not
//! tested here — it is avoided by upgrading serve first, or by setting
//! `multiplex = false`.
//!
//! ## Why `#[ignore]`
//!
//! iroh's `Minimal` preset requires a relay server to dial a peer, and we use
//! the n0 default public relays as the fallback when `relay_urls` is empty.
//! That makes these tests depend on the public Internet and the n0 relay
//! network, which would be flaky in CI. Run them locally with:
//!
//! ```sh
//! cargo test --test serve_access_tunnel -- --ignored --nocapture
//! ```

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use iroh::endpoint::presets::Minimal;
use iroh::endpoint::{RecvStream, RelayMode, SendStream};
use iroh::SecretKey;
use iroh_tunnel::config::{encode_secret_key, ServeConfig};
use iroh_tunnel::proto;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

/// Bytes the tunnel must transport unchanged in each direction. Small enough
/// to fit one copy_buf flush, large enough to exercise the real path.
const PAYLOAD_OUT: &[u8] = b"hello-through-the-tunnel";

// ---------------------------------------------------------------------------
// Suite 1 — pre-0.2.0 single-channel behavior pinned
// ---------------------------------------------------------------------------

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

    // 4. A pre-0.2.0-style access config: no `multiplex` field would also
    //    default to true, so pin `false` explicitly — this suite pins the
    //    single-channel (legacy) behavior of the whole stack.
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
multiplex = false
"#,
            node_id = serve_node_id,
            port = access_addr.port(),
        ),
    )?;

    // 5. Launch serve and access with injected shutdown signals.
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

    // 6. Wait for the access listener to come up, then pipe bytes through it.
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

    // 7. Shutdown both roles gracefully and assert they exited cleanly.
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

// ---------------------------------------------------------------------------
// Suites 2–4 — counting mini-serve
// ---------------------------------------------------------------------------

/// An in-test serve endpoint that echoes every bidirectional stream and
/// counts live connections and streams, so tests can assert the multiplexing
/// contract (N streams on 1 connection) directly. `total_conns` counts every
/// connection ever accepted — for sequential-channel tests where the live
/// count never peaks at 2.
struct MiniServe {
    ep: iroh::Endpoint,
    conns: Arc<AtomicUsize>,
    streams: Arc<AtomicUsize>,
    total_conns: Arc<AtomicUsize>,
}

impl MiniServe {
    /// Start a mini-serve under the service's (single, unchanged) ALPN.
    async fn start(svc_name: &str) -> Result<Self> {
        let ep = iroh::Endpoint::builder(Minimal)
            .secret_key(SecretKey::generate())
            .alpns(vec![proto::alpn_for(svc_name)])
            .relay_mode(RelayMode::Default)
            .bind()
            .await
            .context("mini-serve bind")?;

        let conns = Arc::new(AtomicUsize::new(0));
        let streams = Arc::new(AtomicUsize::new(0));
        let total_conns = Arc::new(AtomicUsize::new(0));

        let accept_ep = ep.clone();
        let conns2 = conns.clone();
        let streams2 = streams.clone();
        let total2 = total_conns.clone();
        tokio::spawn(async move {
            loop {
                let Some(incoming) = accept_ep.accept().await else {
                    return;
                };
                let Ok(conn) = incoming.await else {
                    continue;
                };
                let conns = conns2.clone();
                let streams = streams2.clone();
                let total = total2.clone();
                conns.fetch_add(1, Ordering::SeqCst);
                total.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    // 0.2.0 serve semantics: accept every stream until the
                    // connection dies.
                    loop {
                        match conn.accept_bi().await {
                            Ok((send, recv)) => {
                                streams.fetch_add(1, Ordering::SeqCst);
                                tokio::spawn(echo_stream(recv, send, streams.clone()));
                            }
                            Err(_) => {
                                conns.fetch_sub(1, Ordering::SeqCst);
                                return;
                            }
                        }
                    }
                });
            }
        });

        Ok(Self {
            ep,
            conns,
            streams,
            total_conns,
        })
    }
}

/// Echo one QUIC stream (read→write loop), decrementing the live-stream
/// counter when done.
async fn echo_stream(mut recv: RecvStream, mut send: SendStream, streams: Arc<AtomicUsize>) {
    let mut buf = [0u8; 4096];
    while let Ok(Some(n)) = recv.read(&mut buf).await {
        if send.write_all(&buf[..n]).await.is_err() {
            break;
        }
    }
    let _ = send.finish();
    streams.fetch_sub(1, Ordering::SeqCst);
}

/// Boot a real `access::run_with_shutdown` against `serve_node_id`, exposing
/// the service locally on a fresh port. Returns the local listen address, the
/// shutdown sender, and the task handle.
#[allow(clippy::type_complexity)]
async fn boot_access(
    tmp: &tempfile::TempDir,
    serve_node_id: &str,
    multiplex: bool,
) -> Result<(
    std::net::SocketAddr,
    oneshot::Sender<()>,
    tokio::task::JoinHandle<Result<()>>,
)> {
    let probe = TcpListener::bind("127.0.0.1:0").await?;
    let access_addr = probe.local_addr()?;
    drop(probe);

    let cfg_path = tmp.path().join("access.toml");
    std::fs::write(
        &cfg_path,
        format!(
            r#"
[[services]]
name = "echo"
node_id = "{node_id}"
protocol = "tcp"
host = "127.0.0.1"
port = {port}
multiplex = {multiplex}
"#,
            node_id = serve_node_id,
            port = access_addr.port(),
        ),
    )?;

    let (tx, rx) = oneshot::channel::<()>();
    let handle = {
        let path = cfg_path.clone();
        tokio::spawn(async move {
            iroh_tunnel::access::run_with_shutdown(&path, async move {
                let _ = rx.await;
            })
            .await
        })
    };
    Ok((access_addr, tx, handle))
}

/// Round-trip one payload over an established local client connection.
async fn echo_once(client: &mut TcpStream) -> Result<()> {
    client.write_all(PAYLOAD_OUT).await?;
    client.flush().await?;
    let mut got = vec![0u8; PAYLOAD_OUT.len()];
    client.read_exact(&mut got).await?;
    assert_eq!(got.as_slice(), PAYLOAD_OUT, "bytes corrupted in transit");
    Ok(())
}

/// Poll `f` until it returns true or 30 s elapse.
async fn wait_until<F: Fn() -> bool>(f: F, what: &str) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(30), async {
        while !f() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .with_context(|| format!("timed out waiting for {what}"))
    .map(|_| ())
}

/// Suite 2 — the multiplexing contract: two concurrent local clients ride two
/// streams on ONE connection; closing one stream leaves the other untouched.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires Internet + n0 relay network; run with --ignored"]
async fn multiplex_two_streams_share_one_connection() -> Result<()> {
    init_tracing();
    let tmp = tempfile::tempdir().context("tempdir")?;

    let serve = MiniServe::start("echo").await?;
    let serve_node_id = serve.ep.secret_key().public().to_string();
    let (access_addr, _tx, _access) = boot_access(&tmp, &serve_node_id, true).await?;

    // Two concurrent local clients.
    let mut c1 = retry_connect(access_addr, Duration::from_secs(30)).await?;
    let mut c2 = retry_connect(access_addr, Duration::from_secs(30)).await?;
    echo_once(&mut c1).await?;
    echo_once(&mut c2).await?;

    // Both channels open and echoing: with multiplexing this is exactly one
    // iroh connection carrying two live streams.
    wait_until(
        || serve.conns.load(Ordering::SeqCst) == 1 && serve.streams.load(Ordering::SeqCst) == 2,
        "one connection with two live streams",
    )
    .await?;

    // Closing client 1 closes only its stream; client 2 keeps working on the
    // same connection.
    drop(c1);
    wait_until(
        || serve.streams.load(Ordering::SeqCst) == 1,
        "stream 1 to close",
    )
    .await?;
    assert_eq!(
        serve.conns.load(Ordering::SeqCst),
        1,
        "connection must survive a stream closing"
    );
    echo_once(&mut c2).await?;
    Ok(())
}

/// Suite 3 — access `multiplex = false`: the pre-0.2.0 behavior, unchanged
/// (two sequential channels each get their own connection).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires Internet + n0 relay network; run with --ignored"]
async fn multiplex_false_matches_legacy_behavior() -> Result<()> {
    init_tracing();
    let tmp = tempfile::tempdir().context("tempdir")?;

    let serve = MiniServe::start("echo").await?;
    let serve_node_id = serve.ep.secret_key().public().to_string();
    let (access_addr, _tx, _access) = boot_access(&tmp, &serve_node_id, false).await?;

    let mut c1 = retry_connect(access_addr, Duration::from_secs(30)).await?;
    echo_once(&mut c1).await?;
    drop(c1);

    let mut c2 = retry_connect(access_addr, Duration::from_secs(30)).await?;
    echo_once(&mut c2).await?;
    // One connection per channel: two sequential channels, two connections in
    // total (the live count never peaks at 2 — c1 closes before c2).
    wait_until(
        || serve.total_conns.load(Ordering::SeqCst) == 2,
        "two per-channel connections in total",
    )
    .await?;
    Ok(())
}

/// Suite 4 — keep-alive: a multiplexed connection with no traffic for 60 s
/// (twice noq's default 30 s idle timeout) must stay alive via the 5 s
/// keep-alive; a channel opened afterwards rides the SAME connection.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires Internet + n0 relay network; run with --ignored; slow ~70s"]
async fn multiplex_survives_idle_beyond_timeout() -> Result<()> {
    init_tracing();
    let tmp = tempfile::tempdir().context("tempdir")?;

    let serve = MiniServe::start("echo").await?;
    let serve_node_id = serve.ep.secret_key().public().to_string();
    let (access_addr, _tx, _access) = boot_access(&tmp, &serve_node_id, true).await?;

    let mut c1 = retry_connect(access_addr, Duration::from_secs(30)).await?;
    echo_once(&mut c1).await?;
    wait_until(
        || serve.conns.load(Ordering::SeqCst) == 1 && serve.streams.load(Ordering::SeqCst) == 1,
        "first channel established",
    )
    .await?;

    // Idle window twice the 30 s QUIC idle timeout. The connection must be
    // kept alive by the 5 s keep-alive pings.
    tokio::time::sleep(Duration::from_secs(60)).await;

    // New channel: must ride the SAME connection (no new handshake).
    let mut c2 = retry_connect(access_addr, Duration::from_secs(30)).await?;
    echo_once(&mut c2).await?;
    wait_until(
        || serve.streams.load(Ordering::SeqCst) == 2,
        "second stream on same connection",
    )
    .await?;
    assert_eq!(
        serve.conns.load(Ordering::SeqCst),
        1,
        "idle window must not have dropped the multiplexed connection"
    );

    // The first channel is still alive too.
    echo_once(&mut c1).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_test_writer()
        .try_init();
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
                    anyhow::bail!("failed to connect to {addr} within {deadline:?}: {e}");
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
