//! UDP forwarding through the tunnel, end-to-end (#88).
//!
//! Boots serve + access in-process against an in-process relay (no Internet,
//! CI-safe; same harness shape as `serve_status_conn_path.rs`) and drives
//! real UDP sockets: clients send to the Access listener, a real UDP target
//! sits behind Serve. Contract: `docs/design/udp-integration-contract.md`.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use iroh::test_utils::run_relay_server;
use iroh::SecretKey;
use iroh_tunnel::config::encode_secret_key;
use tokio::net::UdpSocket;
use tokio::sync::oneshot;

/// How long a client waits for a reply before the tunnel is declared broken.
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

/// A running serve + access pair with a UDP echo target behind serve.
struct UdpTunnel {
    /// Where clients send datagrams (the Access listener).
    access_addr: SocketAddr,
    stop: Vec<oneshot::Sender<()>>,
    roles: Vec<tokio::task::JoinHandle<Result<()>>>,
    target: tokio::task::JoinHandle<()>,
    /// Source endpoint of every datagram the target received, in order.
    target_sources: Arc<Mutex<Vec<SocketAddr>>>,
    _cfg: tempfile::TempDir,
    _relay: Box<dyn std::any::Any>,
}

impl UdpTunnel {
    async fn start() -> Result<Self> {
        let cfg = tempfile::tempdir().context("config tempdir")?;
        let (_map, relay_url, relay) = run_relay_server().await?;
        let relay_url = relay_url.to_string();

        // The UDP target behind serve: echoes every datagram to its sender.
        let target_sock = UdpSocket::bind("127.0.0.1:0").await?;
        let target_addr = target_sock.local_addr()?;
        let target_sources = Arc::new(Mutex::new(Vec::new()));
        let target = tokio::spawn(udp_echo(target_sock, target_sources.clone()));

        let serve_key = SecretKey::generate();
        let access_key = SecretKey::generate();

        let serve_cfg = cfg.path().join("serve.toml");
        std::fs::write(
            &serve_cfg,
            format!(
                r#"
[node]
secret_key = "{key}"
relay_urls = ["{relay_url}"]

[[services]]
name = "dns"
protocol = "udp"
host = "{host}"
port = {port}
"#,
                key = encode_secret_key(&serve_key),
                host = target_addr.ip(),
                port = target_addr.port(),
            ),
        )?;

        // Reserve a free UDP port for the Access listener; access rebinds it.
        let probe = UdpSocket::bind("127.0.0.1:0").await?;
        let access_addr = probe.local_addr()?;
        drop(probe);

        let access_cfg = cfg.path().join("access.toml");
        std::fs::write(
            &access_cfg,
            format!(
                r#"
[node]
secret_key = "{key}"
relay_urls = ["{relay_url}"]

[[services]]
name = "dns"
node_id = "{serve_id}"
protocol = "udp"
host = "127.0.0.1"
port = {port}
"#,
                key = encode_secret_key(&access_key),
                serve_id = serve_key.public(),
                port = access_addr.port(),
            ),
        )?;

        let (serve_tx, serve_rx) = oneshot::channel::<()>();
        let (access_tx, access_rx) = oneshot::channel::<()>();
        let serve = tokio::spawn(async move {
            iroh_tunnel::serve::run_with_shutdown(&serve_cfg, async move {
                let _ = serve_rx.await;
            })
            .await
        });
        let access = tokio::spawn(async move {
            iroh_tunnel::access::run_with_shutdown(&access_cfg, async move {
                let _ = access_rx.await;
            })
            .await
        });

        Ok(Self {
            access_addr,
            stop: vec![serve_tx, access_tx],
            roles: vec![serve, access],
            target,
            target_sources,
            _cfg: cfg,
            _relay: Box::new(relay),
        })
    }

    /// Send `payload` from `client` and wait for the reply, retrying the send
    /// while the tunnel is still connecting (UDP gives no connect signal).
    async fn roundtrip(&self, client: &UdpSocket, payload: &[u8]) -> Result<Vec<u8>> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let mut buf = vec![0u8; 65535];
        loop {
            client.send_to(payload, self.access_addr).await?;
            match tokio::time::timeout(Duration::from_millis(500), client.recv_from(&mut buf)).await
            {
                Ok(Ok((n, _))) => return Ok(buf[..n].to_vec()),
                Ok(Err(error)) => return Err(error.into()),
                Err(_) if tokio::time::Instant::now() < deadline => continue,
                Err(_) => anyhow::bail!("no reply through the tunnel within 30s"),
            }
        }
    }

    async fn shutdown(mut self) -> Result<()> {
        for tx in self.stop.drain(..) {
            let _ = tx.send(());
        }
        for role in self.roles.drain(..) {
            tokio::time::timeout(Duration::from_secs(15), role)
                .await
                .context("role did not stop within 15s")??
                .context("role returned an error")?;
        }
        self.target.abort();
        Ok(())
    }
}

/// UDP echo target: replies to each datagram's sender with the same bytes.
async fn udp_echo(sock: UdpSocket, sources: Arc<Mutex<Vec<SocketAddr>>>) {
    let mut buf = vec![0u8; 65535];
    while let Ok((n, from)) = sock.recv_from(&mut buf).await {
        sources.lock().unwrap().push(from);
        let _ = sock.send_to(&buf[..n], from).await;
    }
}

/// AC-UDP-01 (slice 1): one client's datagram crosses Access and Serve to the
/// target and the echoed reply comes back with the payload unchanged.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_datagram_round_trips_through_the_tunnel() -> Result<()> {
    let tunnel = UdpTunnel::start().await?;
    let client = UdpSocket::bind("127.0.0.1:0").await?;

    let payload = b"hello-through-the-udp-tunnel";
    let reply = tokio::time::timeout(REPLY_TIMEOUT * 4, tunnel.roundtrip(&client, payload))
        .await
        .context("round trip timed out")??;

    assert_eq!(reply.as_slice(), payload);
    tunnel.shutdown().await
}

/// Discard replies still in flight from earlier sends. `roundtrip` resends
/// while the tunnel connects, so late duplicates are expected after warm-up.
async fn drain(client: &UdpSocket) {
    let mut buf = vec![0u8; 65535];
    while tokio::time::timeout(Duration::from_millis(700), client.recv_from(&mut buf))
        .await
        .is_ok()
    {}
}

/// Wait for one datagram on `client`.
async fn recv_one(client: &UdpSocket) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; 65535];
    let (n, _) = tokio::time::timeout(REPLY_TIMEOUT, client.recv_from(&mut buf))
        .await
        .context("no reply within the timeout")??;
    Ok(buf[..n].to_vec())
}

/// AC-UDP-01 / AC-UDP-02: concurrent clients each receive their own reply,
/// payloads (empty, binary, varied length) arrive unchanged, and boundaries
/// are kept: three datagrams sent back-to-back come back as three datagrams.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn interleaved_clients_get_their_own_unchanged_replies() -> Result<()> {
    let tunnel = UdpTunnel::start().await?;
    let a = UdpSocket::bind("127.0.0.1:0").await?;
    let b = UdpSocket::bind("127.0.0.1:0").await?;

    // Warm the tunnel up so the interleaved phase below is not about dialing.
    tunnel.roundtrip(&a, b"warm-a").await?;
    tunnel.roundtrip(&b, b"warm-b").await?;
    drain(&a).await;
    drain(&b).await;

    let binary: Vec<u8> = (0..=255u8).collect();
    let a_msgs: Vec<Vec<u8>> = vec![b"a-1".to_vec(), Vec::new(), binary.clone()];
    let b_msgs: Vec<Vec<u8>> = vec![b"b-1-longer".to_vec(), vec![0u8; 900], b"b-3".to_vec()];

    // Interleave the sends from the two clients.
    for i in 0..3 {
        a.send_to(&a_msgs[i], tunnel.access_addr).await?;
        b.send_to(&b_msgs[i], tunnel.access_addr).await?;
    }

    // UDP may reorder, so compare as sets, but each client must get exactly
    // its own three and nothing of the other's.
    let mut got_a = Vec::new();
    let mut got_b = Vec::new();
    for _ in 0..3 {
        got_a.push(recv_one(&a).await?);
        got_b.push(recv_one(&b).await?);
    }
    got_a.sort();
    got_b.sort();
    let mut want_a = a_msgs;
    let mut want_b = b_msgs;
    want_a.sort();
    want_b.sort();
    assert_eq!(got_a, want_a, "client a must get exactly its own datagrams");
    assert_eq!(got_b, want_b, "client b must get exactly its own datagrams");

    tunnel.shutdown().await
}

/// AC-UDP-03 / UDP-03: Serve uses a dedicated upstream socket per flow. Two
/// clients reach the target from two different source endpoints, and each
/// client's endpoint stays the same for the life of its flow.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn each_flow_has_its_own_stable_upstream_endpoint() -> Result<()> {
    let tunnel = UdpTunnel::start().await?;
    let a = UdpSocket::bind("127.0.0.1:0").await?;
    let b = UdpSocket::bind("127.0.0.1:0").await?;

    tunnel.roundtrip(&a, b"a-0").await?;
    tunnel.roundtrip(&b, b"b-0").await?;
    let before = tunnel.target_sources.lock().unwrap().len();
    for i in 0..3u8 {
        tunnel.roundtrip(&a, &[b'a', i]).await?;
        tunnel.roundtrip(&b, &[b'b', i]).await?;
    }

    let sources = tunnel.target_sources.lock().unwrap().clone();
    let a_sources: std::collections::HashSet<_> =
        sources.iter().skip(before).step_by(2).copied().collect();
    let b_sources: std::collections::HashSet<_> = sources
        .iter()
        .skip(before + 1)
        .step_by(2)
        .copied()
        .collect();
    assert_eq!(a_sources.len(), 1, "flow a must keep one upstream endpoint");
    assert_eq!(b_sources.len(), 1, "flow b must keep one upstream endpoint");
    assert_ne!(
        a_sources, b_sources,
        "each flow needs its own upstream socket"
    );

    tunnel.shutdown().await
}
