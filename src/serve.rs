//! Serve role: publish local services into Iroh.
//!
//! Implements T-06. Loads the serve config, builds an [`Endpoint`] that
//! registers every service's ALPN, then accepts incoming streams and pipes each
//! one to the matching local TCP service.
//!
//! ## Concurrency model
//!
//! - One accept loop task serves the whole endpoint (iroh 1.0 registers all
//!   ALPNs on a single endpoint, so we demultiplex by ALPN per connection).
//! - Each accepted stream becomes its own task, so a failure in one connection
//!   never affects another (NFR-08).
//! - Connection errors are logged at WARN and the connection is dropped; the
//!   process never crashes on a per-connection error.
//!
//! Based on Page 04 v2 §1.1 (serve accept sequence) and Page 06 v5 §1.1
//! (serve run CLI behavior). Note: iroh 1.0's accept/ALPN API differs from the
//! earlier draft the spec was written against — see the API notes inline.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use iroh::endpoint::Connection;
use tokio::net::TcpStream;

use crate::{config::ServeConfig, endpoint, proto};

/// Run the serve role until interrupted (Ctrl-C).
///
/// Loads `config_path`, resolves/persists the node secret key, builds the
/// endpoint with all service ALPNs registered, prints the operator-facing
/// status lines, then enters the accept loop.
pub async fn run(config_path: &Path) -> Result<()> {
    let mut cfg = ServeConfig::load(config_path)?;
    cfg.resolve_and_save_key(config_path)?;

    // Collect every service's ALPN and build an ALPN -> local-addr lookup for
    // demultiplexing accepted streams. iroh 1.0 registers ALPNs on the endpoint
    // at build time, so we need them all up front.
    let alpns: Vec<Vec<u8>> = cfg.services.iter().map(|s| proto::alpn_for(&s.name)).collect();
    let mut local_addrs: HashMap<Vec<u8>, String> = HashMap::new();
    for svc in &cfg.services {
        let alpn = proto::alpn_for(&svc.name);
        local_addrs.insert(alpn, format!("{}:{}", svc.host, svc.port));
    }

    let ep = endpoint::create_serve_endpoint(&cfg.node, &alpns).await?;

    let node_id = endpoint::node_id_string(&ep);
    println!("NodeId: {node_id}");
    if let Some(relay) = endpoint::home_relay(&ep) {
        println!("Home relay: {relay}");
    }

    for svc in &cfg.services {
        let local_addr = format!("{}:{}", svc.host, svc.port);
        println!(
            "Serving: {} {}://{local_addr}",
            svc.name,
            protocol_str(svc.protocol)
        );
    }

    if cfg.services.is_empty() {
        tracing::warn!("no services configured; nothing to serve");
    }

    tracing::info!("serve endpoint ready, accepting connections");
    let accept_ep = ep.clone();
    let accept = tokio::spawn(async move {
        accept_loop(&accept_ep, local_addrs).await;
    });

    // Wait for SIGINT/SIGTERM, then drain in-flight streams before closing
    // the endpoint (T-08).
    crate::shutdown::wait_for_signal().await;
    accept.abort();
    crate::shutdown::drain_connections(std::time::Duration::from_secs(5)).await;
    ep.close().await;
    Ok(())
}

/// Accept connections forever, demultiplexing each to its service by ALPN.
///
/// Returns only if the endpoint is closed (e.g. after Ctrl-C). Per-connection
/// errors are logged, not propagated.
async fn accept_loop(ep: &iroh::Endpoint, local_addrs: HashMap<Vec<u8>, String>) {
    loop {
        // ep.accept() is a Future yielding Option<Incoming>; None means the
        // endpoint was closed.
        let Some(incoming) = ep.accept().await else {
            tracing::info!("endpoint closed, accept loop exiting");
            return;
        };

        // Drive the handshake. The Connecting future resolves to a Connection.
        let conn = match incoming.await {
            Ok(conn) => conn,
            Err(e) => {
                tracing::warn!("incoming handshake failed: {e}");
                continue;
            }
        };

        // Demultiplex by the negotiated ALPN to find the local service address.
        let alpn = conn.alpn().to_vec();
        let Some(local_addr) = local_addrs.get(&alpn).cloned() else {
            let name = proto::name_from_alpn(&alpn)
                .map(String::from)
                .unwrap_or_else(|| format!("{alpn:02x?}"));
            tracing::warn!("connection with unknown ALPN for service '{name}', dropping");
            continue;
        };

        tokio::spawn(async move {
            match handle_connection(&conn, &local_addr).await {
                Ok(()) => tracing::debug!("connection closed normally"),
                Err(e) => tracing::warn!("connection error: {e}"),
            }
        });
    }
}

/// Accept a bidirectional stream on `conn`, connect the local service, and pipe
/// bytes both ways until either side closes.
async fn handle_connection(conn: &Connection, local_addr: &str) -> Result<()> {
    // accept_bi/open_bi return (SendStream, RecvStream) — send first. Our pipe
    // wants the remote pair as (read, write) = (recv, send), so swap.
    let (send, recv) = conn.accept_bi().await.context("accept_bidi failed")?;

    let local = TcpStream::connect(local_addr)
        .await
        .with_context(|| format!("failed to connect local service: {local_addr}"))?;

    // Pipe the local TCP stream against the QUIC stream halves.
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
