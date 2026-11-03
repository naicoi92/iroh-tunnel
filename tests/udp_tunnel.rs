//! UDP forwarding through the tunnel, end-to-end (#88).
//!
//! Boots serve + access in-process against an in-process relay (no Internet,
//! CI-safe; same harness shape as `serve_status_conn_path.rs`) and drives
//! real UDP sockets: clients send to the Access listener, a real UDP target
//! sits behind Serve. Contract: `docs/design/udp-integration-contract.md`.

use std::net::SocketAddr;
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
        let target = tokio::spawn(udp_echo(target_sock));

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
            match tokio::time::timeout(Duration::from_millis(500), client.recv_from(&mut buf))
                .await
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
async fn udp_echo(sock: UdpSocket) {
    let mut buf = vec![0u8; 65535];
    while let Ok((n, from)) = sock.recv_from(&mut buf).await {
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
