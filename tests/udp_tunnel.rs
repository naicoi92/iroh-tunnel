//! UDP forwarding through the tunnel, end-to-end (#88).
//!
//! Boots serve + access in-process against an in-process relay (no Internet,
//! CI-safe; same harness shape as `serve_status_conn_path.rs`) and drives
//! real UDP sockets: clients send to the Access listener, a real UDP target
//! sits behind Serve. Contract: `docs/udp-integration-contract.md`.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use iroh::test_utils::run_relay_server;
use iroh::SecretKey;
use iroh_tunnel::config::encode_secret_key;
use tokio::net::UdpSocket;
use tokio::sync::oneshot;

/// Next Access listener port to try. These ports sit below every OS's
/// ephemeral range, so a test client's socket (bound to port 0) can never be
/// handed a port another test has reserved but not yet bound.
static NEXT_ACCESS_PORT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0);

/// Reserve a free UDP port for an Access listener; the role rebinds it.
async fn reserve_access_port() -> Result<SocketAddr> {
    use std::sync::atomic::Ordering::Relaxed;
    // Spread concurrently running test binaries apart by process id.
    let first = 20_000 + (std::process::id() % 200) as u16 * 100;
    let _ = NEXT_ACCESS_PORT.compare_exchange(0, first, Relaxed, Relaxed);
    for _ in 0..500 {
        let port = NEXT_ACCESS_PORT.fetch_add(1, Relaxed);
        anyhow::ensure!(port < 40_000, "ran out of access test ports");
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        if let Ok(probe) = UdpSocket::bind(addr).await {
            drop(probe);
            return Ok(addr);
        }
    }
    anyhow::bail!("no free access test port")
}

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
    /// State dirs of the two primary roles, where their status files land.
    serve_state: tempfile::TempDir,
    access_state: tempfile::TempDir,
    _relay: Box<dyn std::any::Any>,
}

impl UdpTunnel {
    async fn start() -> Result<Self> {
        Self::start_limited("").await
    }

    /// Start with extra lines in the serve service table (UDP limits).
    async fn start_limited(serve_extra: &str) -> Result<Self> {
        // The UDP target behind serve: echoes every datagram to its sender.
        let target_sock = UdpSocket::bind("127.0.0.1:0").await?;
        let target_addr = target_sock.local_addr()?;
        let target_sources = Arc::new(Mutex::new(Vec::new()));
        let target = tokio::spawn(udp_echo(target_sock, target_sources.clone()));
        Self::start_configured(target_addr, Some(target), target_sources, serve_extra).await
    }

    async fn start_with_target(
        target_addr: SocketAddr,
        target: Option<tokio::task::JoinHandle<()>>,
        target_sources: Arc<Mutex<Vec<SocketAddr>>>,
    ) -> Result<Self> {
        Self::start_configured(target_addr, target, target_sources, "").await
    }

    /// Start the tunnel in front of `target_addr`. `target` is the task that
    /// serves it, if the caller already runs one.
    async fn start_configured(
        target_addr: SocketAddr,
        target: Option<tokio::task::JoinHandle<()>>,
        target_sources: Arc<Mutex<Vec<SocketAddr>>>,
        serve_extra: &str,
    ) -> Result<Self> {
        Self::start_with(target_addr, target, target_sources, serve_extra, "").await
    }

    /// Start with extra lines in both the serve and the access service table.
    async fn start_with(
        target_addr: SocketAddr,
        target: Option<tokio::task::JoinHandle<()>>,
        target_sources: Arc<Mutex<Vec<SocketAddr>>>,
        serve_extra: &str,
        access_extra: &str,
    ) -> Result<Self> {
        // Opt-in diagnostics: RUST_LOG=iroh_tunnel=debug shows both roles.
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_test_writer()
            .try_init();
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
{serve_extra}
"#,
                key = encode_secret_key(&serve_key),
                host = target_addr.ip(),
                port = target_addr.port(),
            ),
        )?;

        let access_addr = reserve_access_port().await?;

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
{access_extra}
"#,
                key = encode_secret_key(&access_key),
                serve_id = serve_key.public(),
                port = access_addr.port(),
            ),
        )?;

        let serve_state = tempfile::tempdir().context("serve state dir")?;
        let access_state = tempfile::tempdir().context("access state dir")?;
        let (serve_tx, serve_rx) = oneshot::channel::<()>();
        let (access_tx, access_rx) = oneshot::channel::<()>();
        let serve = {
            let state = serve_state.path().to_path_buf();
            tokio::spawn(async move {
                iroh_tunnel::host::testing::run_unowned(
                    iroh_tunnel::host::status::Role::Serve,
                    &serve_cfg,
                    Some(&state),
                    async move {
                        let _ = serve_rx.await;
                    },
                )
                .await
            })
        };
        let access = {
            let state = access_state.path().to_path_buf();
            tokio::spawn(async move {
                iroh_tunnel::host::testing::run_unowned(
                    iroh_tunnel::host::status::Role::Access,
                    &access_cfg,
                    Some(&state),
                    async move {
                        let _ = access_rx.await;
                    },
                )
                .await
            })
        };

        Ok(Self {
            access_addr,
            stop: vec![serve_tx, access_tx],
            roles: vec![serve, access],
            target,
            target_sources,
            relay_url,
            serve_id: serve_key.public().to_string(),
            cfg,
            serve_state,
            access_state,
            _relay: Box::new(relay),
        })
    }

    /// Start a second Access peer (own identity) for the same service and
    /// return the address its clients send to.
    async fn add_access_peer(&mut self, tag: &str) -> Result<SocketAddr> {
        let addr = reserve_access_port().await?;
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
            iroh_tunnel::host::testing::run_unowned(
                iroh_tunnel::host::status::Role::Access,
                &path,
                None,
                async move {
                    let _ = rx.await;
                },
            )
            .await
        }));
        Ok(addr)
    }

    /// Send `payload` from `client` and wait for the reply, retrying the send
    /// while the tunnel is still connecting (UDP gives no connect signal).
    async fn roundtrip(&self, client: &UdpSocket, payload: &[u8]) -> Result<Vec<u8>> {
        roundtrip_via(self.access_addr, client, payload).await
    }

    /// Poll a role's status file until `ready` accepts its JSON.
    async fn status_when(
        dir: &std::path::Path,
        file: &str,
        ready: impl Fn(&serde_json::Value) -> bool,
    ) -> Result<serde_json::Value> {
        Self::status_within(dir, file, Duration::from_secs(30), ready).await
    }

    /// [`Self::status_when`] with an explicit patience.
    async fn status_within(
        dir: &std::path::Path,
        file: &str,
        patience: Duration,
        ready: impl Fn(&serde_json::Value) -> bool,
    ) -> Result<serde_json::Value> {
        let path = dir.join(file);
        let deadline = tokio::time::Instant::now() + patience;
        let mut last = String::from("(file never readable)");
        loop {
            if let Ok(body) = std::fs::read_to_string(&path) {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                    if ready(&json) {
                        return Ok(json);
                    }
                    last = json["services"][0]["udp"].to_string();
                }
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "{file} never reached the expected state; last udp block: {last}"
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Stop the Serve role and start a new one with the same identity, config
    /// and target. Every connection to the old one is gone.
    async fn restart_serve(&mut self) -> Result<()> {
        let _ = self.stop.remove(0).send(());
        tokio::time::timeout(Duration::from_secs(15), self.roles.remove(0))
            .await
            .context("old serve did not stop within 15s")??
            .context("old serve returned an error")?;
        let (tx, rx) = oneshot::channel::<()>();
        let state = self.serve_state.path().to_path_buf();
        let path = self.cfg.path().join("serve.toml");
        self.stop.insert(0, tx);
        self.roles.insert(
            0,
            tokio::spawn(async move {
                iroh_tunnel::host::testing::run_unowned(
                    iroh_tunnel::host::status::Role::Serve,
                    &path,
                    Some(&state),
                    async move {
                        let _ = rx.await;
                    },
                )
                .await
            }),
        );
        Ok(())
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
    // Reserve the target's address, but leave nothing listening on it yet. It
    // comes from the reserved low range: an ephemeral port freed here could be
    // handed to another test's client socket, which would then receive this
    // test's traffic.
    let target_addr = reserve_access_port().await?;

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

/// #89 diagnostics: both roles' status files report the live flow count, the
/// current datagram limit and a drop counter per reason. An oversized client
/// datagram is dropped at Access, counted as `oversize`, and never delivered.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn status_files_report_flows_limit_and_drops_by_reason() -> Result<()> {
    let tunnel = UdpTunnel::start().await?;
    let client = UdpSocket::bind("127.0.0.1:0").await?;
    tunnel.roundtrip(&client, b"warm").await?;
    drain(&client).await;
    let delivered = tunnel.target_sources.lock().unwrap().len();

    // Far above any QUIC datagram limit, but well inside what UDP loopback carries.
    client.send_to(&vec![7u8; 4000], tunnel.access_addr).await?;

    let access = UdpTunnel::status_when(tunnel.access_state.path(), "access-status.json", |s| {
        s["services"][0]["udp"]["drops"]["oversize"].as_u64() >= Some(1)
    })
    .await?;
    let udp = &access["services"][0]["udp"];
    assert!(udp["active_flows"].as_u64() >= Some(1), "{udp}");
    assert!(udp["datagram_limit"].as_u64() > Some(0), "{udp}");
    for reason in [
        "oversize",
        "malformed",
        "unknown_flow",
        "upstream_error",
        "cap",
        "queue_full",
        "limit_shrink",
        "quarantine_reply",
        "foreign_source",
    ] {
        assert!(
            udp["drops"][reason].is_u64(),
            "missing counter {reason}: {udp}"
        );
    }

    let serve = UdpTunnel::status_when(tunnel.serve_state.path(), "serve-status.json", |s| {
        s["services"][0]["udp"]["active_flows"].as_u64() >= Some(1)
    })
    .await?;
    assert_eq!(serve["services"][0]["udp"]["drops"]["oversize"], 0);

    // The oversized datagram never reached the target.
    assert_eq!(tunnel.target_sources.lock().unwrap().len(), delivered);
    tunnel.shutdown().await
}

/// #89 / UDP-06 admission: at the service cap Serve refuses a new flow, counts
/// it as `cap`, and never evicts a flow that is already running.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn at_the_service_cap_a_new_flow_is_refused_and_old_flows_keep_working() -> Result<()> {
    let tunnel = UdpTunnel::start_limited("max_flows = 2\nmax_flows_per_peer = 2").await?;
    let a = UdpSocket::bind("127.0.0.1:0").await?;
    let b = UdpSocket::bind("127.0.0.1:0").await?;
    let c = UdpSocket::bind("127.0.0.1:0").await?;

    tunnel.roundtrip(&a, b"a-1").await?;
    tunnel.roundtrip(&b, b"b-1").await?;
    drain(&a).await;
    drain(&b).await;

    // The third client's flow is over the cap: it gets nothing back.
    for _ in 0..3 {
        c.send_to(b"c-1", tunnel.access_addr).await?;
    }
    assert!(
        try_recv(&c, Duration::from_secs(2)).await?.is_none(),
        "a flow over the cap must not be served"
    );

    // The flows that were admitted first still work.
    assert_eq!(tunnel.roundtrip(&a, b"a-2").await?, b"a-2");
    assert_eq!(tunnel.roundtrip(&b, b"b-2").await?, b"b-2");

    // The status file is flushed every few seconds: wait for a snapshot that
    // already shows both facts, not just the first one.
    let serve = UdpTunnel::status_when(tunnel.serve_state.path(), "serve-status.json", |s| {
        let udp = &s["services"][0]["udp"];
        udp["drops"]["cap"].as_u64() >= Some(1) && udp["active_flows"] == 2
    })
    .await?;
    assert_eq!(serve["services"][0]["udp"]["active_flows"], 2);
    tunnel.shutdown().await
}

/// UDP-06 per-peer cap: one Access peer that has used its share is refused
/// further flows, while another peer is still served from the shared service
/// cap. A flooding peer cannot take every slot.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_peer_cannot_take_the_slots_of_another() -> Result<()> {
    let mut tunnel = UdpTunnel::start_limited("max_flows = 4\nmax_flows_per_peer = 1").await?;
    let second = tunnel.add_access_peer("second").await?;
    let first = tunnel.access_addr;
    let a1 = UdpSocket::bind("127.0.0.1:0").await?;
    let a2 = UdpSocket::bind("127.0.0.1:0").await?;
    let b1 = UdpSocket::bind("127.0.0.1:0").await?;

    // The first peer's only slot.
    roundtrip_via(first, &a1, b"a1").await?;
    drain(&a1).await;

    // Its second flow is over the per-peer cap.
    for _ in 0..3 {
        a2.send_to(b"a2", first).await?;
    }
    assert!(
        try_recv(&a2, Duration::from_secs(2)).await?.is_none(),
        "a second flow from the same peer must be refused"
    );

    // The other peer is not affected by the first peer's usage.
    assert_eq!(roundtrip_via(second, &b1, b"b1").await?, b"b1");

    // The status file is flushed every few seconds: wait for a snapshot that
    // already shows both facts, not just the first one.
    let serve = UdpTunnel::status_when(tunnel.serve_state.path(), "serve-status.json", |s| {
        let udp = &s["services"][0]["udp"];
        udp["drops"]["cap"].as_u64() >= Some(1) && udp["active_flows"] == 2
    })
    .await?;
    assert_eq!(serve["services"][0]["udp"]["active_flows"], 2);
    tunnel.shutdown().await
}

/// Echo target that answers after `delay`, so a reply can arrive after its
/// flow has gone idle.
async fn delayed_echo(sock: Arc<UdpSocket>, delay: Duration, sources: Arc<Mutex<Vec<SocketAddr>>>) {
    let mut buf = vec![0u8; 65535];
    while let Ok((n, from)) = sock.recv_from(&mut buf).await {
        sources.lock().unwrap().push(from);
        let reply = buf[..n].to_vec();
        let sock = sock.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = sock.send_to(&reply, from).await;
        });
    }
}

/// UDP-04 / UDP-06 quarantine: after a flow goes idle its upstream socket is
/// kept open as a sink. A late reply from the target lands there, is counted
/// as `quarantine_reply` and is never delivered to anyone. The flow is
/// reported as quarantined, not active.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_late_reply_to_an_expired_flow_is_counted_and_not_delivered() -> Result<()> {
    let target = Arc::new(UdpSocket::bind("127.0.0.1:0").await?);
    let target_addr = target.local_addr()?;
    let sources = Arc::new(Mutex::new(Vec::new()));
    let echo = tokio::spawn(delayed_echo(
        target,
        Duration::from_secs(4),
        sources.clone(),
    ));
    let tunnel =
        UdpTunnel::start_configured(target_addr, None, sources.clone(), "idle_timeout = 1").await?;
    let client = UdpSocket::bind("127.0.0.1:0").await?;

    // UDP gives no connect signal and Access may not be listening yet: send
    // until the target has seen a datagram, then stop so the flow can go idle.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while sources.lock().unwrap().is_empty() {
        anyhow::ensure!(
            tokio::time::Instant::now() < deadline,
            "the datagram never reached the target"
        );
        let _ = client.send_to(b"slow", tunnel.access_addr).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let serve = UdpTunnel::status_when(tunnel.serve_state.path(), "serve-status.json", |s| {
        let udp = &s["services"][0]["udp"];
        udp["drops"]["quarantine_reply"].as_u64() >= Some(1)
            && udp["quarantined_flows"] == 1
            && udp["active_flows"] == 0
    })
    .await?;
    assert!(serve["services"][0]["udp"]["drops"]["quarantine_reply"].as_u64() >= Some(1));
    assert!(
        try_recv(&client, Duration::from_millis(500))
            .await?
            .is_none(),
        "a reply that arrives after the flow expired must not reach any client"
    );

    echo.abort();
    tunnel.shutdown().await
}

/// UDP-06: a quarantined socket still counts against the caps, so a new flow
/// is refused while it lasts, and is admitted once quarantine ends (30 s).
/// Slow on purpose: it runs the real quarantine period.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn quarantine_holds_the_slot_and_then_releases_it() -> Result<()> {
    let tunnel =
        UdpTunnel::start_limited("idle_timeout = 1\nmax_flows = 1\nmax_flows_per_peer = 1").await?;
    let a = UdpSocket::bind("127.0.0.1:0").await?;
    let b = UdpSocket::bind("127.0.0.1:0").await?;
    let c = UdpSocket::bind("127.0.0.1:0").await?;

    tunnel.roundtrip(&a, b"a").await?;
    // a goes idle after 1 s and is quarantined; its socket still holds the slot.
    UdpTunnel::status_when(tunnel.serve_state.path(), "serve-status.json", |s| {
        s["services"][0]["udp"]["quarantined_flows"] == 1
    })
    .await?;

    for _ in 0..3 {
        b.send_to(b"b", tunnel.access_addr).await?;
    }
    assert!(
        try_recv(&b, Duration::from_secs(2)).await?.is_none(),
        "a quarantined socket must keep holding its slot"
    );

    // Quarantine ends after 30 s; the status file lags by up to one flush.
    UdpTunnel::status_within(
        tunnel.serve_state.path(),
        "serve-status.json",
        Duration::from_secs(60),
        |s| s["services"][0]["udp"]["quarantined_flows"] == 0,
    )
    .await?;
    assert_eq!(tunnel.roundtrip(&c, b"c").await?, b"c");
    tunnel.shutdown().await
}

/// UDP-04: no flow survives a reconnect. When Serve restarts, the Access
/// connection is dead; the next datagram must dial a fresh one and start a
/// new incarnation, instead of sending into the dead connection forever.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn forwarding_resumes_after_serve_restarts() -> Result<()> {
    let mut tunnel = UdpTunnel::start().await?;
    let client = UdpSocket::bind("127.0.0.1:0").await?;
    assert_eq!(tunnel.roundtrip(&client, b"before").await?, b"before");
    drain(&client).await;

    tunnel.restart_serve().await?;

    // `roundtrip` keeps resending while the new connection is dialed.
    let reply = tokio::time::timeout(Duration::from_secs(40), tunnel.roundtrip(&client, b"after"))
        .await
        .context("forwarding never resumed after Serve restarted")??;
    assert!(
        reply == b"after" || reply == b"before",
        "unexpected {reply:?}"
    );
    tunnel.shutdown().await
}

/// UDP-06 isolation: a client that floods its flow does not slow another
/// client's flow. A UDP send to a local target rarely blocks, so this cannot
/// force `queue_full` end to end (the unit tests in `src/udp.rs` cover the
/// bounded queue itself); it proves the flood stays inside its own flow.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_flooding_flow_does_not_stop_another_flow() -> Result<()> {
    let tunnel = UdpTunnel::start_limited("queue_depth = 1").await?;
    let flooder = UdpSocket::bind("127.0.0.1:0").await?;
    let calm = UdpSocket::bind("127.0.0.1:0").await?;
    tunnel.roundtrip(&flooder, b"hello").await?;
    tunnel.roundtrip(&calm, b"hello").await?;
    drain(&flooder).await;
    drain(&calm).await;

    let access = tunnel.access_addr;
    let flood = tokio::spawn(async move {
        for _ in 0..5000 {
            let _ = flooder.send_to(&[1u8; 512], access).await;
        }
        flooder
    });
    for i in 0..5u8 {
        let reply = tokio::time::timeout(
            Duration::from_secs(10),
            roundtrip_via(access, &calm, &[b'c', i]),
        )
        .await
        .context("the calm flow stalled while another flow flooded")??;
        assert!(reply == [b'c', i] || reply.len() == 512 || reply == b"hello");
    }
    let _ = flood.await;
    tunnel.shutdown().await
}

/// UDP-06 Access cap: Access holds at most `max_flows` flows per service. A
/// new client over the cap is dropped at Access, counted as `cap`, and never
/// reaches Serve; flows already running are not evicted.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn access_refuses_new_clients_over_its_own_cap() -> Result<()> {
    let sources = Arc::new(Mutex::new(Vec::new()));
    let target_sock = UdpSocket::bind("127.0.0.1:0").await?;
    let target_addr = target_sock.local_addr()?;
    let echo = tokio::spawn(udp_echo(target_sock, sources.clone()));
    let tunnel =
        UdpTunnel::start_with(target_addr, None, sources.clone(), "", "max_flows = 2").await?;
    let a = UdpSocket::bind("127.0.0.1:0").await?;
    let b = UdpSocket::bind("127.0.0.1:0").await?;
    let c = UdpSocket::bind("127.0.0.1:0").await?;

    tunnel.roundtrip(&a, b"a-1").await?;
    tunnel.roundtrip(&b, b"b-1").await?;
    drain(&a).await;
    drain(&b).await;
    let reached = sources.lock().unwrap().len();

    for _ in 0..3 {
        c.send_to(b"c-1", tunnel.access_addr).await?;
    }
    assert!(
        try_recv(&c, Duration::from_secs(2)).await?.is_none(),
        "a client over the Access cap must not be served"
    );
    assert_eq!(
        sources.lock().unwrap().len(),
        reached,
        "a refused client must not reach the target"
    );
    assert_eq!(tunnel.roundtrip(&a, b"a-2").await?, b"a-2");

    let access = UdpTunnel::status_when(tunnel.access_state.path(), "access-status.json", |s| {
        let udp = &s["services"][0]["udp"];
        udp["drops"]["cap"].as_u64() >= Some(3) && udp["active_flows"] == 2
    })
    .await?;
    assert_eq!(access["services"][0]["udp"]["active_flows"], 2);

    echo.abort();
    tunnel.shutdown().await
}

/// Transports the Access status file reports for the first service.
fn access_transports(status: &serde_json::Value) -> Vec<(String, bool)> {
    status["services"][0]["transports"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|t| {
                    (
                        t["kind"].as_str().unwrap_or_default().to_owned(),
                        t["active"].as_bool().unwrap_or(false),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// #89 path evidence: the same UDP round trip works on a relay-only path and
/// on a direct path, and the status file shows which one was used.
///
/// Relay-only is forced by building the endpoints without IP transports
/// (test-utils seam); direct is the default on loopback once hole punching
/// finishes. One sequential test because the seam is process-global. This is
/// loopback with an in-process relay: it proves the code runs on both path
/// kinds, not how either performs on a real network.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn udp_round_trips_on_a_relay_only_path_and_on_a_direct_path() -> Result<()> {
    // Phase 1: relay only.
    // `start` only spawns the roles; they build their endpoints a moment
    // later. Keep the switch on until the first round trip has proved both
    // endpoints exist, then turn it off so phase 2 gets default endpoints.
    iroh_tunnel_engine::testing::set_relay_only(true);
    let relay_only = UdpTunnel::start().await?;
    let client = UdpSocket::bind("127.0.0.1:0").await?;
    let first = relay_only.roundtrip(&client, b"over-relay").await;
    iroh_tunnel_engine::testing::set_relay_only(false);
    assert_eq!(first?, b"over-relay");
    let status =
        UdpTunnel::status_when(relay_only.access_state.path(), "access-status.json", |s| {
            !access_transports(s).is_empty()
        })
        .await?;
    let transports = access_transports(&status);
    assert!(
        transports.iter().all(|(kind, _)| kind == "relay"),
        "a relay-only endpoint must not report a direct path: {transports:?}"
    );
    assert!(
        transports.iter().any(|(_, active)| *active),
        "{transports:?}"
    );
    relay_only.shutdown().await?;

    // Phase 2: default endpoints; wait for hole punching to select a direct path.
    let direct = UdpTunnel::start().await?;
    assert_eq!(direct.roundtrip(&client, b"warm").await?, b"warm");
    UdpTunnel::status_within(
        direct.access_state.path(),
        "access-status.json",
        Duration::from_secs(60),
        |s| {
            access_transports(s)
                .iter()
                .any(|(kind, active)| kind == "direct" && *active)
        },
    )
    .await
    .context("loopback never selected a direct path")?;
    drain(&client).await;
    assert_eq!(
        direct.roundtrip(&client, b"over-direct").await?,
        b"over-direct"
    );
    direct.shutdown().await
}
