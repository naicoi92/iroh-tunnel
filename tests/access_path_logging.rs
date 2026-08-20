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

#![cfg(unix)] // shutdown.rs installs SIGTERM handlers; restrict to unix

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

/// A role task shared with [`retry_connect`]: the connect loop polls it for
/// early exit (config error, bind race) so a dead role surfaces fast with
/// its real error instead of spinning to the deadline.
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
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_ansi(false)
        .with_writer(LogSink(sink.clone()))
        .try_init();

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
    let mut mux_client =
        retry_connect(mux_addr, Duration::from_secs(30), &serve_role, &access_role).await?;
    roundtrip(&mut mux_client).await?;

    // Per-channel service: the dial (and its log line) happens per client.
    let mut per_channel_client = retry_connect(
        per_channel_addr,
        Duration::from_secs(30),
        &serve_role,
        &access_role,
    )
    .await?;
    roundtrip(&mut per_channel_client).await?;

    // The lines are already in the buffer by now; wait_for_line only scans.
    let mux_line = wait_for_line(&sink, |line| {
        line.contains("connected to serve peer (multiplexed")
            && line.contains("svc_name=echo")
            && line.contains(&serve_node_id)
            && (line.contains("relay=") || line.contains("direct="))
    })
    .await
    .context("multiplexed established line")?;
    println!("multiplexed established: {mux_line}");

    let per_channel_line = wait_for_line(&sink, |line| {
        line.contains("connected to serve peer (per-channel")
            && line.contains("svc_name=echo2")
            && line.contains(&serve_node_id)
            && (line.contains("relay=") || line.contains("direct="))
    })
    .await
    .context("per-channel established line")?;
    println!("per-channel established: {per_channel_line}");

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

/// If the role task has already exited, take and resolve it, returning the
/// result; `None` while it is still running.
async fn role_exited(role: &SharedRole) -> Option<Result<()>> {
    let handle = role.lock().await.take()?;
    match handle.is_finished() {
        true => Some(handle.await.expect("role task panicked")),
        false => {
            *role.lock().await = Some(handle);
            None
        }
    }
}

/// Take a role task out of its slot and wait for clean shutdown.
async fn finish_role(role: &SharedRole, name: &str) -> Result<()> {
    let handle = role
        .lock()
        .await
        .take()
        .unwrap_or_else(|| panic!("{name} role already finished"));
    handle
        .await
        .unwrap_or_else(|e| panic!("{name} role task panicked: {e}"))
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
/// exit (config error, bind race) fails fast with the role's real result
/// instead of a misleading connect timeout.
async fn retry_connect(
    addr: std::net::SocketAddr,
    deadline: Duration,
    serve_role: &SharedRole,
    access_role: &SharedRole,
) -> Result<TcpStream> {
    let deadline = tokio::time::Instant::now() + deadline;
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
                if tokio::time::Instant::now() >= deadline {
                    anyhow::bail!("could not connect to {addr} within {deadline:?}: {e}");
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}
