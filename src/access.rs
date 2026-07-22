//! Access role: consume remote services to local.
//!
//! Implements T-07. Loads the access config (ephemeral key), builds an
//! [`Endpoint`], and for each service opens a local TCP listener. When a local
//! client connects, access dials the remote serve peer, opens a bidirectional
//! QUIC stream, and pipes the client stream through it.
//!
//! ## Concurrency model
//!
//! - One listen-loop task per service (so each service has its own bound port).
//! - Each accepted local client becomes its own task, so a failure in one
//!   tunnel never affects another (NFR-08).
//! - `host = 0.0.0.0` binds all interfaces (share within the LAN); the default
//!   `127.0.0.1` keeps it local-only.
//!
//! Based on Page 04 v2 §1.2 (access dial sequence) and Page 06 v5 §1.2 (access
//! run CLI behavior). Note: iroh 1.0's connect/ALPN API differs from the
//! earlier draft the spec was written against — see the API notes inline.

use std::future::Future;
use std::path::Path;

use anyhow::{Context, Result};
use iroh::EndpointAddr;
use tokio::net::{TcpListener, TcpStream};

use crate::{config::AccessConfig, endpoint, proto};

/// Run the access role until interrupted (Ctrl-C).
///
/// Loads `config_path`, builds an ephemeral-key endpoint, prints the per-service
/// `Exposed:` lines, then spawns a listen loop per service.
pub async fn run(config_path: &Path) -> Result<()> {
    run_with_shutdown(config_path, crate::shutdown::wait_for_signal()).await
}

/// Run the access role until the caller-provided `shutdown` future resolves.
///
/// Same as [`run`], but the shutdown signal is injected. Production wires
/// `shutdown::wait_for_signal()` here; tests inject a `oneshot::Receiver` or
/// similar so the role can be driven end-to-end without sending real signals.
pub async fn run_with_shutdown(
    config_path: &Path,
    shutdown: impl Future<Output = ()>,
) -> Result<()> {
    let mut cfg = AccessConfig::load(config_path)?;
    cfg.resolve_and_save_key(config_path)?;
    let ep = endpoint::create_access_endpoint(&cfg.node).await?;

    println!("NodeId: {}", endpoint::node_id_string(&ep));

    if cfg.services.is_empty() {
        tracing::warn!("no services configured; nothing to expose");
    }

    // The relay URLs to attach to each dialed peer's EndpointAddr. An
    // EndpointAddr containing only a node id cannot be routed by iroh (the
    // Minimal preset registers no address-lookup service), so we MUST provide
    // relay URLs to dial through.
    //
    // Relays are OPTIONAL in config (IROHTUN-44): if the user does not set
    // `relay_urls`, we fall back to the n0 default relays — mirroring iroh's
    // own `RelayMode::Default`. If the user configures `relay_urls`, those
    // take precedence (custom/self-hosted relays for latency, privacy, etc.).
    let relay_urls: Vec<iroh::RelayUrl> = if cfg.node.relay_urls.is_empty() {
        let defaults = endpoint::n0_default_relay_urls();
        tracing::info!(
            count = defaults.len(),
            "no relay_urls configured, falling back to n0 default relays"
        );
        defaults
    } else {
        cfg.node
            .relay_urls
            .iter()
            .map(|s| {
                s.parse::<iroh::RelayUrl>()
                    .with_context(|| format!("invalid relay_url: {s}"))
            })
            .collect::<Result<Vec<_>>>()?
    };

    for svc in &cfg.services {
        let node_id = svc
            .node_id
            .parse::<iroh::EndpointId>()
            .with_context(|| format!("invalid node_id: {}", svc.node_id))?;
        let alpn = proto::alpn_for(&svc.name);
        let listen_addr = format!("{}:{}", svc.host, svc.port);
        println!(
            "Exposed: {} {listen_addr} -> peer {node_id} ({}://{listen_addr})",
            svc.name,
            protocol_str(svc.protocol)
        );

        let ep_clone = ep.clone();
        let svc_name = svc.name.clone();
        tokio::spawn(listen_loop(
            ep_clone,
            node_id,
            alpn,
            listen_addr,
            relay_urls.clone(),
            svc_name,
        ));
    }

    tracing::info!("access endpoint ready, listening for local clients");
    shutdown.await;
    crate::shutdown::drain_connections(std::time::Duration::from_secs(5)).await;
    ep.close().await;
    Ok(())
}

/// Bind `listen_addr` and, for each local client, dial the peer and pipe bytes.
///
/// Returns only if the listener errors fatally (e.g. the bound socket closes).
/// Per-client errors are logged, not propagated.
async fn listen_loop(
    ep: iroh::Endpoint,
    node_id: iroh::EndpointId,
    alpn: Vec<u8>,
    listen_addr: String,
    relay_urls: Vec<iroh::RelayUrl>,
    svc_name: String,
) {
    let listener = match TcpListener::bind(&listen_addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("failed to bind {listen_addr}: {e}");
            return;
        }
    };
    tracing::info!(%listen_addr, "listening for local clients");

    loop {
        match listener.accept().await {
            Ok((local_stream, peer_addr)) => {
                let ep = ep.clone();
                let alpn = alpn.clone();
                let relay_urls = relay_urls.clone();
                let svc_name = svc_name.clone();
                tokio::spawn(async move {
                    match handle_local_connection(
                        &ep,
                        node_id,
                        &alpn,
                        &relay_urls,
                        &svc_name,
                        local_stream,
                    )
                    .await
                    {
                        Ok(()) => tracing::debug!(%peer_addr, %svc_name, "tunnel closed"),
                        Err(e) => tracing::warn!(%peer_addr, %svc_name, "tunnel error: {e}"),
                    }
                });
            }
            Err(e) => {
                tracing::warn!("accept error on {listen_addr}: {e}");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
    }
}

/// Dial the peer, open a bidirectional stream, and pipe the local client
/// through it until either side closes.
async fn handle_local_connection(
    ep: &iroh::Endpoint,
    node_id: iroh::EndpointId,
    alpn: &[u8],
    relay_urls: &[iroh::RelayUrl],
    svc_name: &str,
    local: TcpStream,
) -> Result<()> {
    // Build the peer address. Endpoint::connect() is idempotent — it reuses an
    // existing QUIC connection to the peer if one is already open, so a pool of
    // local clients multiplexes streams over a single QUIC connection (Page 04
    // v2 §5).
    //
    // Attach every available relay URL so iroh can try each in turn. With no
    // relay URLs and no address-lookup service (Minimal preset), iroh returns
    // "No addressing information available" — so an empty `relay_urls` is a
    // programming error here. The caller (run()) guarantees a non-empty list
    // by falling back to the n0 defaults when the config is empty (IROHTUN-44).
    let mut addr = EndpointAddr::new(node_id);
    for url in relay_urls {
        addr = addr.with_relay_url(url.clone());
    }

    let conn = connect_with_retry(ep, addr, alpn).await?;

    // The serve peer's NodeId, from its TLS cert. Logged on connect/disconnect
    // so operators can correlate access activity with serve-side logs.
    let remote_id = conn.remote_id();
    tracing::info!(peer = %remote_id, %svc_name, "connected to serve peer");

    // Watcher: emit a disconnect line when the QUIC connection closes. The weak
    // handle is registered while `conn` is still alive, so iroh guarantees the
    // close event is delivered even if `conn` drops before this resolves.
    let weak = conn.weak_handle();
    let rid = remote_id;
    let sname = svc_name.to_string();
    tokio::spawn(async move {
        let _ = weak.closed().await;
        tracing::info!(peer = %rid, %sname, "disconnected from serve peer");
    });

    // open_bi returns (SendStream, RecvStream) — send first. Our pipe wants the
    // remote pair as (read, write) = (recv, send), so we swap.
    let (send, recv) = conn
        .open_bi()
        .await
        .context("open bidirectional stream failed")?;

    crate::pipe::pipe_tcp_bidirectional(local, (recv, send)).await?;
    Ok(())
}

/// Dial the peer with exponential backoff.
///
/// Retries the `Endpoint::connect` call on failure with backoff 1s → 2s → 4s →
/// 8s → 16s → 30s (cap), looping. On the first success after one or more
/// failures, logs `reconnected after N attempts`. Per-service independent: each
/// local-client task runs its own retry, so one unreachable peer never affects
/// another service (Page 04 v2 §1.3).
///
/// Note: the spec sample shows `ep.connect(node_id, alpn)` with a bare
/// [`iroh::NodeId`], but iroh 1.0's connect takes an [`iroh::EndpointAddr`] —
/// the same reality T-07's `handle_local_connection` already built the address
/// for. We accept the prepared address and apply the retry/backoff contract on
/// top of it unchanged.
async fn connect_with_retry(
    ep: &iroh::Endpoint,
    addr: iroh::EndpointAddr,
    alpn: &[u8],
) -> Result<iroh::endpoint::Connection> {
    let mut backoff_ms = 1000u64;
    let max_backoff_ms = 30_000u64;
    let mut attempt = 1u32;
    loop {
        match ep.connect(addr.clone(), alpn).await {
            Ok(conn) => {
                if attempt > 1 {
                    tracing::info!("reconnected after {attempt} attempts");
                }
                return Ok(conn);
            }
            Err(e) => {
                tracing::warn!("connect attempt {attempt} failed: {e}, retrying in {backoff_ms}ms");
                tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                backoff_ms = (backoff_ms * 2).min(max_backoff_ms);
                attempt += 1;
            }
        }
    }
}

/// Lowercase protocol name for display (matches the serde form in `config`).
fn protocol_str(p: crate::config::Protocol) -> &'static str {
    match p {
        crate::config::Protocol::Tcp => "tcp",
        crate::config::Protocol::Udp => "udp",
    }
}
