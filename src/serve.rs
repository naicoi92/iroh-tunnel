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
use std::future::Future;
use std::path::Path;

use anyhow::{Context, Result};
use iroh::endpoint::Connection;
use tokio::net::TcpStream;

use crate::config::ServeConfig;
use crate::endpoint;
use crate::proto;
use crate::role_run::RoleStrategy;

/// Run the serve role until interrupted (Ctrl-C).
///
/// Thin wrapper over [`crate::role_run::run_with_shutdown`] that wires the
/// real signal handler and the [`ServeStrategy`] implementer.
pub async fn run(config_path: &Path) -> Result<()> {
    crate::role_run::run_with_shutdown::<ServeStrategy>(
        config_path,
        crate::shutdown::wait_for_signal(),
    )
    .await
}

/// Run the serve role until the caller-provided `shutdown` future resolves.
///
/// Same as [`run`], but the shutdown signal is injected. Production wires
/// `shutdown::wait_for_signal()` here; tests inject a `oneshot::Receiver` or
/// similar so the role can be driven end-to-end without sending real signals.
pub async fn run_with_shutdown(
    config_path: &Path,
    shutdown: impl Future<Output = ()>,
) -> Result<()> {
    crate::role_run::run_with_shutdown::<ServeStrategy>(config_path, shutdown).await
}

/// Serve-role implementation of [`RoleStrategy`].
///
/// Owns the genuinely-serve-specific pieces: registers every service's ALPN
/// on the endpoint, writes the status snapshot, runs the accept loop that
/// demultiplexes incoming streams by ALPN.
pub(crate) struct ServeStrategy;

impl RoleStrategy for ServeStrategy {
    type Config = ServeConfig;

    async fn build_endpoint(cfg: &Self::Config) -> Result<iroh::Endpoint> {
        // Collect every service's ALPN up front — iroh 1.0 registers ALPNs on
        // the endpoint at build time (not filtered per-accept).
        let alpns: Vec<Vec<u8>> = cfg
            .services
            .iter()
            .map(|s| proto::alpn_for(&s.name))
            .collect();
        endpoint::create_serve_endpoint(&cfg.node, &alpns).await
    }

    fn print_services(cfg: &Self::Config) {
        if cfg.services.is_empty() {
            tracing::warn!("no services configured; nothing to serve");
            return;
        }
        for svc in &cfg.services {
            let local_addr = format!("{}:{}", svc.host, svc.port);
            println!(
                "Serving: {} {}://{local_addr}",
                svc.name,
                crate::role_run::protocol_str(svc.protocol)
            );
        }
    }

    async fn run_loop(
        ep: iroh::Endpoint,
        cfg: Self::Config,
        shutdown: impl Future<Output = ()>,
    ) -> Result<()> {
        // Build the ALPN -> local-addr lookup for demultiplexing accepted
        // streams.
        let mut local_addrs: HashMap<Vec<u8>, String> = HashMap::new();
        for svc in &cfg.services {
            let alpn = proto::alpn_for(&svc.name);
            local_addrs.insert(alpn, format!("{}:{}", svc.host, svc.port));
        }

        tracing::info!("serve endpoint ready, accepting connections");
        let accept_ep = ep.clone();
        let accept = tokio::spawn(async move {
            accept_loop(&accept_ep, local_addrs).await;
        });

        // Write the operator-facing status snapshot (T-13). Best-effort: a
        // failure to write status is logged but does not stop the tunnel.
        let status = crate::status::StatusFile {
            node_id: crate::endpoint::node_id_string(&ep),
            home_relay: endpoint::home_relay(&ep).map(|u| u.to_string()),
            pid: std::process::id(),
            started_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            services: cfg
                .services
                .iter()
                .map(|s| crate::status::ServiceStatus {
                    name: s.name.clone(),
                    protocol: crate::role_run::protocol_str(s.protocol).to_string(),
                    local_addr: crate::status::format_local_addr(&s.host, s.port),
                    active_connections: 0,
                })
                .collect(),
        };
        match status.save() {
            Ok(p) => tracing::info!(path = %p.display(), "wrote status file"),
            Err(e) => tracing::warn!("failed to write status file: {e}"),
        }

        // Wait for the injected shutdown signal, then drain in-flight streams
        // before closing the endpoint (T-08). The accept task is aborted
        // first so it stops handing new connections to the pipe.
        shutdown.await;
        accept.abort();
        ep.close().await;
        Ok(())
    }
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

        // The access peer's NodeId, from its TLS cert. Logged on connect and
        // (via the watcher below) on disconnect, so operators can see who is
        // tunneling in and correlate with access-side logs.
        let remote_id = conn.remote_id();
        let name = proto::name_from_alpn(&alpn)
            .map(String::from)
            .unwrap_or_else(|| format!("{alpn:02x?}"));
        tracing::info!(peer = %remote_id, service = %name, "peer connected");

        // Watcher: emit a disconnect line when the QUIC connection closes. The
        // weak handle is registered while `conn` is still alive (before the
        // stream-handling task takes it), so iroh guarantees the close event is
        // delivered even if the connection drops before this resolves.
        crate::role_run::spawn_disconnect_watcher(
            &conn,
            remote_id.to_string(),
            format!("peer disconnected (service {name})"),
        );

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
    crate::pipe::pipe_tcp_bidirectional(local, (recv, send)).await?;
    Ok(())
}
