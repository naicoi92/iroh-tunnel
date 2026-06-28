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
    let cfg = AccessConfig::load(config_path)?;
    let ep = endpoint::create_access_endpoint(&cfg.node).await?;

    if cfg.services.is_empty() {
        tracing::warn!("no services configured; nothing to expose");
    }

    // The global relay_urls serve as the connectivity fallback for dialing
    // peers: we attach the first relay URL to each peer's EndpointAddr so the
    // remote serve node is reachable through the shared relay. (n0 relays are
    // stateless and will forward to any peer registered with them.) If no
    // relay_urls are configured, dialing falls back to whatever address lookup
    // the endpoint has — which for the Minimal preset may be none.
    let relay_url = cfg
        .node
        .relay_urls
        .first()
        .and_then(|s: &String| s.parse::<iroh::RelayUrl>().ok());

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
        tokio::spawn(listen_loop(
            ep_clone,
            node_id,
            alpn,
            listen_addr,
            relay_url.clone(),
        ));
    }

    tracing::info!("access endpoint ready, listening for local clients");
    tokio::signal::ctrl_c().await?;
    tracing::info!("shutdown signal received");
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
    relay_url: Option<iroh::RelayUrl>,
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
                let relay_url = relay_url.clone();
                tokio::spawn(async move {
                    match handle_local_connection(&ep, node_id, &alpn, relay_url, local_stream)
                        .await
                    {
                        Ok(()) => tracing::debug!(%peer_addr, "tunnel closed"),
                        Err(e) => tracing::warn!(%peer_addr, "tunnel error: {e}"),
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
    relay_url: Option<iroh::RelayUrl>,
    local: TcpStream,
) -> Result<()> {
    // Build the peer address. Endpoint::connect() is idempotent — it reuses an
    // existing QUIC connection to the peer if one is already open, so a pool of
    // local clients multiplexes streams over a single QUIC connection (Page 04
    // v2 §5).
    let mut addr = EndpointAddr::new(node_id);
    if let Some(relay) = relay_url {
        addr = addr.with_relay_url(relay);
    }

    let conn = ep.connect(addr, alpn).await.context("dial peer failed")?;

    // open_bi returns (SendStream, RecvStream) — send first. Our pipe wants the
    // remote pair as (read, write) = (recv, send), so we swap.
    let (send, recv) = conn
        .open_bi()
        .await
        .context("open bidirectional stream failed")?;

    crate::pipe::pipe_bidirectional(local, (recv, send)).await?;
    Ok(())
}

/// Lowercase protocol name for display (matches the serde form in `config`).
fn protocol_str(p: crate::config::Protocol) -> &'static str {
    match p {
        crate::config::Protocol::Tcp => "tcp",
        crate::config::Protocol::Udp => "udp",
    }
}
