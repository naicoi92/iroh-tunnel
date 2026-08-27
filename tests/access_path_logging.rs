//! Access connection-path logging (issue #58), end-to-end.
//!
//! Same harness shape as `serve_status_conn_path.rs`: serve + access roles
//! against an IN-PROCESS relay (`iroh::test_utils::run_relay_server`,
//! self-signed cert trusted via the crate's `test-utils` feature) — no
//! Internet, no n0 relay network, CI-safe.
//!
//! Logs are captured through a `tracing_subscriber` writer that appends
//! every formatted line into a shared buffer, installed BEFORE the roles
//! spawn. The test drives one multiplexed and one per-channel service
//! through the tunnel and asserts each connection-established line carries
//! the service name, the serve node's full id in `peer=`, and a rendered
//! active transport (`relay=` — via the in-process relay the relay path is
//! active from the handshake on; `direct=` accepted as the flexible
//! fallback).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use iroh::test_utils::run_relay_server;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

use iroh::SecretKey;
use iroh_tunnel::config::encode_secret_key;

/// Bytes the tunnel must transport unchanged in each direction.
const PAYLOAD: &[u8] = b"hello-through-the-path-logs";

/// How long to wait for a log line to show up in the capture.
const LOG_POLL_TIMEOUT: Duration = Duration::from_secs(30);

/// Poller cadence — the real one from the crate (`test-utils` exposes it),
/// so the derived windows below can never drift from production timing.
use iroh_tunnel::access::PATH_POLL_INTERVAL;

/// Quiet window for [`wait_for_quiet`]: one poller tick plus slack.
const QUIET_WINDOW: Duration = Duration::from_secs(PATH_POLL_INTERVAL.as_secs() + 1);

/// Steady-state hold: two poller ticks of silence.
const STEADY_HOLD: Duration = Duration::from_secs(PATH_POLL_INTERVAL.as_secs() * 2);

/// A role task shared with [`retry_connect`]: the connect loop polls it for
/// early exit — only a config error fails the role task outright (a bind
/// failure is logged inside the role's own per-service listen task and the
/// role keeps running, surfacing later as a connect timeout with the
/// `failed to bind` line visible in the captured dump).
type SharedRole = Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<Result<()>>>>>;

/// Shared log sink: a `MakeWriter` whose `io::Write` appends raw bytes.
#[derive(Clone)]
struct LogSink(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for LogSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogSink {
    type Writer = LogSink;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// The captured log so far, split into lines.
fn captured_lines(sink: &Arc<Mutex<Vec<u8>>>) -> Vec<String> {
    let buf = sink.lock().unwrap().clone();
    String::from_utf8_lossy(&buf)
        .lines()
        .map(str::to_string)
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn access_logs_connection_paths_on_established() -> Result<()> {
    let sink = Arc::new(Mutex::new(Vec::new()));
    if let Err(e) = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
                // Pin this crate to info even when ambient RUST_LOG (e.g.
                // `warn`) filters it out — otherwise the asserted INFO
                // lines never reach the sink and the test fails with two
                // misleading 30 s timeouts instead of a missing line.
                .add_directive("iroh_tunnel=info".parse().unwrap()),
        )
        .with_ansi(false)
        .with_writer(LogSink(sink.clone()))
        .try_init()
    {
        // A subscriber is already installed, so the LogSink is NOT wired:
        // every log assertion below would fail as opaque 30 s timeouts.
        // Fail fast with the root cause instead of burning them.
        anyhow::bail!(
            "tracing subscriber already installed ({e}); LogSink not wired — log assertions cannot pass"
        );
    }

    let cfg_tmp = tempfile::tempdir().context("config tempdir")?;
    let state_tmp = tempfile::tempdir().context("serve state tempdir")?;

    let (_relay_map, relay_url, _server) = run_relay_server().await?;
    let relay = relay_url.to_string();

    // Echo service behind serve — both configured services share it.
    let echo_listener = TcpListener::bind("127.0.0.1:0").await?;
    let echo_addr = echo_listener.local_addr()?;
    let echo = tokio::spawn(echo_server(echo_listener));

    // Pinned keys so the serve node id is known up front for assertions.
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

[[services]]
name = "echo2"
protocol = "tcp"
host = "{host}"
port = {port}
"#,
            serve_key = encode_secret_key(&serve_key),
            host = echo_addr.ip(),
            port = echo_addr.port(),
        ),
    )?;

    // Bind the access ports up front (access rebinds inside run_with_shutdown).
    let mux_listener = TcpListener::bind("127.0.0.1:0").await?;
    let mux_addr = mux_listener.local_addr()?;
    drop(mux_listener);
    let per_channel_listener = TcpListener::bind("127.0.0.1:0").await?;
    let per_channel_addr = per_channel_listener.local_addr()?;
    drop(per_channel_listener);

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
port = {mux_port}
multiplex = true

[[services]]
name = "echo2"
node_id = "{serve_node_id}"
protocol = "tcp"
host = "127.0.0.1"
port = {per_channel_port}
multiplex = false
"#,
            access_key = encode_secret_key(&access_key),
            mux_port = mux_addr.port(),
            per_channel_port = per_channel_addr.port(),
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

    // Multiplexed service: one client establishes the shared connection and
    // its established log line.
    let mut mux_client = retry_connect(
        mux_addr,
        Duration::from_secs(30),
        &serve_role,
        &access_role,
        &sink,
    )
    .await?;
    roundtrip(&mut mux_client).await?;

    // Per-channel service: the dial (and its log line) happens per client.
    let mut per_channel_client = retry_connect(
        per_channel_addr,
        Duration::from_secs(30),
        &serve_role,
        &access_role,
        &sink,
    )
    .await?;
    roundtrip(&mut per_channel_client).await?;

    // The lines are already in the buffer by now; wait_for_line only scans.
    let peer_field = format!("peer={serve_node_id}");
    let mux_line = wait_for_line(&sink, |line| {
        line.contains("connected to serve peer (multiplexed")
            // `svc_name` is the trailing field; `ends_with` keeps `echo`
            // from matching `echo2`.
            && line.ends_with("svc_name=echo")
            // The full id as the `peer=` field (not a bare substring that
            // could match the id appearing anywhere else in the line).
            && line.contains(&peer_field)
            && (line.contains("relay=") || line.contains("direct="))
    })
    .await
    .context("multiplexed established line")?;
    println!("multiplexed established: {mux_line}");

    let per_channel_line = wait_for_line(&sink, |line| {
        line.contains("connected to serve peer (per-channel")
            && line.ends_with("svc_name=echo2")
            && line.contains(&peer_field)
            && (line.contains("relay=") || line.contains("direct="))
    })
    .await
    .context("per-channel established line")?;
    println!("per-channel established: {per_channel_line}");

    // The serve role's accept side logs the same connect with the peer's
    // active transports (`peer connected via …`) — symmetric with the
    // access established lines. Right after the handshake the snapshot may
    // still be empty, so accept the `paths pending` rendering too; the
    // transports rendering itself is unit-tested in conn_path.
    let serve_peer_field = format!("peer={access_node_id}");
    let serve_connect_line = wait_for_line(&sink, |line| {
        line.contains(": peer connected via ")
            && line.contains(&serve_peer_field)
            // Trailing-field boundary: `echo` must not match `echo2`'s line.
            && line.ends_with("service=echo")
    })
    .await
    .context("serve peer-connected line")?;
    println!("serve peer connected: {serve_connect_line}");
    // Steady-state pin (issue #58): the poller must be silent while nothing
    // changes. On this localhost harness *legitimate* migrations are
    // expected early (QUIC address discovery lands after the handshakes;
    // the peer-level remote map is shared by both services), so first let
    // them settle, then hold for two poller ticks and require the count not
    // to move. A late-but-legitimate migration during the hold is absorbed
    // by one bounded re-settle — only continuous churn fails.
    let mut settled_changes = wait_for_quiet(&sink).await?;
    for _ in 0..3 {
        tokio::time::sleep(STEADY_HOLD).await;
        if path_changed_count(&sink) == settled_changes {
            break;
        }
        settled_changes = wait_for_quiet(&sink).await?;
    }
    let changes_after = path_changed_count(&sink);
    assert_eq!(
        changes_after, settled_changes,
        "poller must stay silent while nothing changes"
    );
    drop(mux_client);
    drop(per_channel_client);
    let _ = access_tx.send(());
    finish_role(&access_role, "access").await?;
    let _ = serve_tx.send(());
    finish_role(&serve_role, "serve").await?;
    echo.abort();
    Ok(())
}

/// Send the payload, read the echo back, byte-exact.
async fn roundtrip(client: &mut TcpStream) -> Result<()> {
    client.write_all(PAYLOAD).await?;
    client.flush().await?;
    let mut got = vec![0u8; PAYLOAD.len()];
    tokio::time::timeout(Duration::from_secs(10), client.read_exact(&mut got))
        .await
        .context("no echo through the in-process relay within 10s")?
        .context("echo read failed")?;
    assert_eq!(got.as_slice(), PAYLOAD);
    Ok(())
}

/// Poll the captured lines until one satisfies `pred`; the deadline fails
/// the test with everything captured so far (the established lines are
/// emitted asynchronously by the role tasks).
async fn wait_for_line(sink: &Arc<Mutex<Vec<u8>>>, pred: impl Fn(&str) -> bool) -> Result<String> {
    let deadline = tokio::time::Instant::now() + LOG_POLL_TIMEOUT;
    loop {
        if let Some(line) = captured_lines(sink).into_iter().find(|l| pred(l)) {
            return Ok(line);
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!(
                "expected log line not captured within {LOG_POLL_TIMEOUT:?}; captured:\n{}",
                captured_lines(sink).join("\n")
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
/// Count captured `path changed` lines so far.
fn path_changed_count(sink: &Arc<Mutex<Vec<u8>>>) -> usize {
    captured_lines(sink)
        .iter()
        // Match the message prefix, not a bare substring: every default-
        // format line renders as `<target>: <message> <fields>`, so
        // `": path changed "` cannot match an unrelated field value.
        .filter(|l| l.contains(": path changed "))
        .count()
}

/// Wait until one full poller interval passes with no NEW `path changed`
/// line, returning the settled count. The initial migrations (localhost
/// QUIC address discovery landing right after the handshakes) are
/// legitimate poller output — only *after* they settle is silence the
/// correct expectation.
async fn wait_for_quiet(sink: &Arc<Mutex<Vec<u8>>>) -> Result<usize> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut last_count = path_changed_count(sink);
    let mut last_change_at = tokio::time::Instant::now();
    loop {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let count = path_changed_count(sink);
        if count != last_count {
            last_count = count;
            last_change_at = tokio::time::Instant::now();
        }
        if last_change_at.elapsed() > QUIET_WINDOW {
            return Ok(last_count);
        }
        if tokio::time::Instant::now() > deadline {
            anyhow::bail!("path-changed lines never settled within 30 s (last count {last_count})");
        }
    }
}

/// If the role task has already exited, take and resolve it, returning the
/// result; `None` while it is still running.
///
/// One guard held across the finished-check and the take — no
/// take-then-reinsert window for a concurrent caller to observe an empty
/// slot.
async fn role_exited(role: &SharedRole) -> Option<Result<()>> {
    let mut guard = role.lock().await;
    if !guard.as_ref()?.is_finished() {
        return None;
    }
    let handle = guard.take()?;
    drop(guard);
    Some(handle.await.expect("role task panicked"))
}

/// Take a role task out of its slot and wait for clean shutdown, bounded so
/// a hung role fails with a diagnostic instead of stalling the test.
async fn finish_role(role: &SharedRole, name: &str) -> Result<()> {
    let handle = role
        .lock()
        .await
        .take()
        .with_context(|| format!("{name} role already finished before shutdown"))?;
    match tokio::time::timeout(Duration::from_secs(30), handle).await {
        // Three layers: Elapsed (bounded), JoinError (panic/cancel), then
        // the role's own result.
        Ok(joined) => joined.with_context(|| format!("{name} role task failed to join"))?,
        Err(_) => Err(anyhow::anyhow!("{name} role did not shut down within 30 s")),
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

/// Retry TCP connect until `deadline`, polling both role tasks so a config
/// error fails fast with the role's real result instead of a misleading
/// connect timeout. A bind failure does NOT exit the role (it is logged by
/// the role's per-service listen task) — the captured sink is scanned for
/// the `failed to bind` line each round so a bind race bails immediately
/// instead of burning the whole deadline.
async fn retry_connect(
    addr: std::net::SocketAddr,
    deadline: Duration,
    serve_role: &SharedRole,
    access_role: &SharedRole,
    sink: &Arc<Mutex<Vec<u8>>>,
) -> Result<TcpStream> {
    let start = tokio::time::Instant::now();
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
                if captured_lines(sink)
                    .iter()
                    .any(|l| l.contains("failed to bind"))
                {
                    anyhow::bail!(
                        "role reported a bind failure (see captured logs); \
                         connect to {addr} will never succeed"
                    );
                }
                if start.elapsed() > deadline {
                    anyhow::bail!("could not connect to {addr} within {deadline:?}: {e}");
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}
