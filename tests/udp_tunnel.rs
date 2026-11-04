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

/// Wait up to `limit` for one datagram on `client`.
///
/// `None` means none arrived. Windows reports the ICMP port-unreachable of an
/// earlier send (os error 10054) on the next receive of an unconnected UDP
/// socket; that is not a failure of the tunnel, so it is skipped.
async fn try_recv(client: &UdpSocket, limit: Duration) -> Result<Option<Vec<u8>>> {
    let deadline = tokio::time::Instant::now() + limit;
    let mut buf = vec![0u8; 65535];
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, client.recv_from(&mut buf)).await {
            Ok(Ok((n, _))) => return Ok(Some(buf[..n].to_vec())),
            Ok(Err(error)) if iroh_tunnel::udp::is_transient_recv_error(&error) => continue,
            Ok(Err(error)) => return Err(error.into()),
            Err(_) => return Ok(None),
        }
    }
}

/// A running serve + access pair with a UDP echo target behind serve.
struct UdpTunnel {
    /// Where clients send datagrams (the Access listener).
    access_addr: SocketAddr,
    stop: Vec<oneshot::Sender<()>>,
    roles: Vec<tokio::task::JoinHandle<Result<()>>>,
    target: Option<tokio::task::JoinHandle<()>>,
    /// Source endpoint of every datagram the target received, in order.
    target_sources: Arc<Mutex<Vec<SocketAddr>>>,
    relay_url: String,
    serve_id: String,
    cfg: tempfile::TempDir,
    _relay: Box<dyn std::any::Any>,
}

impl UdpTunnel {
    async fn start() -> Result<Self> {
        // The UDP target behind serve: echoes every datagram to its sender.
        let target_sock = UdpSocket::bind("127.0.0.1:0").await?;
        let target_addr = target_sock.local_addr()?;
        let target_sources = Arc::new(Mutex::new(Vec::new()));
        let target = tokio::spawn(udp_echo(target_sock, target_sources.clone()));
        Self::start_with_target(target_addr, Some(target), target_sources).await
    }

    /// Start the tunnel in front of `target_addr`. `target` is the task that
    /// serves it, if the caller already runs one.
    async fn start_with_target(
        target_addr: SocketAddr,
        target: Option<tokio::task::JoinHandle<()>>,
        target_sources: Arc<Mutex<Vec<SocketAddr>>>,
    ) -> Result<Self> {
        let cfg = tempfile::tempdir().context("config tempdir")?;
        let (_map, relay_url, relay) = run_relay_server().await?;
        let relay_url = relay_url.to_string();

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
            relay_url,
            serve_id: serve_key.public().to_string(),
            cfg,
            _relay: Box::new(relay),
        })
    }

    /// Start a second Access peer (own identity) for the same service and
    /// return the address its clients send to.
    async fn add_access_peer(&mut self, tag: &str) -> Result<SocketAddr> {
        let probe = UdpSocket::bind("127.0.0.1:0").await?;
        let addr = probe.local_addr()?;
        drop(probe);
        let key = SecretKey::generate();
        let path = self.cfg.path().join(format!("access-{tag}.toml"));
        std::fs::write(
            &path,
            format!(
                r#"
[node]
secret_key = "{key}"
relay_urls = ["{relay}"]

[[services]]
name = "dns"
node_id = "{serve_id}"
protocol = "udp"
host = "127.0.0.1"
port = {port}
"#,
                key = encode_secret_key(&key),
                relay = self.relay_url,
                serve_id = self.serve_id,
                port = addr.port(),
            ),
        )?;
        let (tx, rx) = oneshot::channel::<()>();
        self.stop.push(tx);
        self.roles.push(tokio::spawn(async move {
            iroh_tunnel::access::run_with_shutdown(&path, async move {
                let _ = rx.await;
            })
            .await
        }));
        Ok(addr)
    }

    /// Send `payload` from `client` and wait for the reply, retrying the send
    /// while the tunnel is still connecting (UDP gives no connect signal).
    async fn roundtrip(&self, client: &UdpSocket, payload: &[u8]) -> Result<Vec<u8>> {
        roundtrip_via(self.access_addr, client, payload).await
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
        if let Some(target) = self.target.take() {
            target.abort();
        }
        Ok(())
    }
}

/// UDP echo target: replies to each datagram's sender with the same bytes.
async fn udp_echo(sock: UdpSocket, sources: Arc<Mutex<Vec<SocketAddr>>>) {
    let mut buf = vec![0u8; 65535];
    loop {
        match sock.recv_from(&mut buf).await {
            Ok((n, from)) => {
                sources.lock().unwrap().push(from);
                let _ = sock.send_to(&buf[..n], from).await;
            }
            // Windows reports an earlier reply's ICMP error here; keep serving.
            Err(error) if iroh_tunnel::udp::is_transient_recv_error(&error) => continue,
            Err(_) => break,
        }
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
    while let Ok(Some(_)) = try_recv(client, Duration::from_millis(700)).await {}
}

/// Wait for one datagram on `client`.
async fn recv_one(client: &UdpSocket) -> Result<Vec<u8>> {
    try_recv(client, REPLY_TIMEOUT)
        .await?
        .context("no reply within the timeout")
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
    // `roundtrip` resends while connecting; let late duplicates reach the
    // target before counting, or the a/b alternation below would be skewed.
    tokio::time::sleep(Duration::from_secs(2)).await;
    drain(&a).await;
    drain(&b).await;
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

/// Like `UdpTunnel::roundtrip`, for a client of another Access peer.
async fn roundtrip_via(access: SocketAddr, client: &UdpSocket, payload: &[u8]) -> Result<Vec<u8>> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        // Nothing listens on `access` until the role has bound it, so this
        // send may be refused; just try again.
        let _ = client.send_to(payload, access).await;
        if let Some(reply) = try_recv(client, Duration::from_millis(500)).await? {
            return Ok(reply);
        }
        anyhow::ensure!(
            tokio::time::Instant::now() < deadline,
            "no reply through the tunnel within 30s"
        );
    }
}

/// #88 clarification: clients on two Access peers sharing one Serve are never
/// cross-delivered, even though both peers number their first flow 0 (flow
/// identifiers are scoped to a peer's session, not global).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn clients_on_two_access_peers_are_not_cross_delivered() -> Result<()> {
    let mut tunnel = UdpTunnel::start().await?;
    let second = tunnel.add_access_peer("second").await?;
    let first = tunnel.access_addr;
    let a = UdpSocket::bind("127.0.0.1:0").await?;
    let b = UdpSocket::bind("127.0.0.1:0").await?;

    roundtrip_via(first, &a, b"warm-a").await?;
    roundtrip_via(second, &b, b"warm-b").await?;
    drain(&a).await;
    drain(&b).await;

    for i in 0..5u8 {
        a.send_to(&[b'a', i], first).await?;
        b.send_to(&[b'b', i], second).await?;
    }
    let mut got_a = Vec::new();
    let mut got_b = Vec::new();
    for _ in 0..5 {
        got_a.push(recv_one(&a).await?);
        got_b.push(recv_one(&b).await?);
    }
    got_a.sort();
    got_b.sort();
    let want_a: Vec<Vec<u8>> = (0..5u8).map(|i| vec![b'a', i]).collect();
    let want_b: Vec<Vec<u8>> = (0..5u8).map(|i| vec![b'b', i]).collect();
    assert_eq!(got_a, want_a, "client of the first peer got foreign data");
    assert_eq!(got_b, want_b, "client of the second peer got foreign data");

    tunnel.shutdown().await
}

/// Contract: "Upstream send error on Serve: datagram dropped; the flow stays".
/// A target that is not up yet makes the connected upstream socket report
/// ICMP port-unreachable. That must not end the flow's reply path: once the
/// target is up, the same client gets its reply on the same flow.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_flow_survives_its_target_being_down_at_first() -> Result<()> {
    // Reserve the target's address, but leave nothing listening on it yet.
    let reserved = UdpSocket::bind("127.0.0.1:0").await?;
    let target_addr = reserved.local_addr()?;
    drop(reserved);

    let sources = Arc::new(Mutex::new(Vec::new()));
    let tunnel = UdpTunnel::start_with_target(target_addr, None, sources.clone()).await?;
    let client = UdpSocket::bind("127.0.0.1:0").await?;

    // Keep sending while nothing listens: the flow is created and the kernel
    // reports the refused port on its upstream socket.
    for _ in 0..20 {
        client.send_to(b"too-early", tunnel.access_addr).await?;
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    // Now the target comes up on the same address.
    let target = UdpSocket::bind(target_addr).await?;
    let echo = tokio::spawn(udp_echo(target, sources));

    let reply = tokio::time::timeout(
        Duration::from_secs(20),
        tunnel.roundtrip(&client, b"now-it-is-up"),
    )
    .await
    .context("the flow never recovered after the target came up")??;
    assert!(
        reply == b"now-it-is-up" || reply == b"too-early",
        "unexpected reply {reply:?}"
    );

    echo.abort();
    tunnel.shutdown().await
}
