//! Serve role: publish local services into Iroh.
//!
//! Implements T-06. Loads the serve config, builds an [`Endpoint`] that
//! registers every service's ALPN (legacy + multiplex variants — see
//! [`crate::proto`]), then accepts incoming connections and pipes every
//! bidirectional stream on each to the matching local TCP service.
//!
//! ## Concurrency model
//!
//! - One accept loop task serves the whole endpoint (iroh 1.0 registers all
//!   ALPNs on a single endpoint, so we demultiplex by ALPN per connection).
//! - Each *connection* is supervised by its own task running an
//!   `accept_bi` loop; every accepted *stream* becomes its own pipe task,
//!   so a failure in one stream never affects another (NFR-08). One
//!   connection can therefore carry any number of concurrent channels.
//! - Connection errors are logged at WARN and the connection is dropped; the
//!   process never crashes on a per-connection error.
//!
//! Based on Page 04 v2 §1.1 (serve accept sequence) and Page 06 v5 §1.1
//! (serve run CLI behavior). Note: iroh 1.0's accept/ALPN API differs from the
//! earlier draft the spec was written against — see the API notes inline.

use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
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
        // Collect every service's ALPNs up front — iroh 1.0 registers ALPNs
        // on the endpoint at build time (not filtered per-accept). Both the
        // legacy and the multiplex variant map to the same service, so
        // pre-0.2.0 access peers keep working unchanged while 0.2.0 access
        // peers can negotiate multiplexing.
        let alpns: Vec<Vec<u8>> = cfg
            .services
            .iter()
            .flat_map(|s| [proto::alpn_for(&s.name), proto::multiplex_alpn_for(&s.name)])
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
        // Build the ALPN -> target lookup for demultiplexing accepted
        // streams. Both ALPN variants of a service share one target (same
        // local addr, same active-stream counter).
        let mut targets: HashMap<Vec<u8>, ServiceTarget> = HashMap::new();
        for svc in &cfg.services {
            let target = ServiceTarget::new(format!("{}:{}", svc.host, svc.port));
            targets.insert(proto::alpn_for(&svc.name), target.clone());
            targets.insert(proto::multiplex_alpn_for(&svc.name), target.clone());
        }

        // Operator-facing status snapshot (T-13), refreshed by the flush task
        // below as stream counts change. Best-effort: a failure to write
        // status is logged but does not stop the tunnel. Built before the
        // accept task takes ownership of `targets`.
        let status = StatusSnapshot {
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
                .map(|s| StatusServiceRow {
                    name: s.name.clone(),
                    protocol: crate::role_run::protocol_str(s.protocol).to_string(),
                    local_addr: crate::status::format_local_addr(&s.host, s.port),
                    active_streams: targets
                        .get(&proto::alpn_for(&s.name))
                        .map(|t| t.active_streams())
                        .unwrap_or_default(),
                })
                .collect(),
        };
        match status.render().save() {
            Ok(p) => tracing::info!(path = %p.display(), "wrote status file"),
            Err(e) => tracing::warn!("failed to write status file: {e}"),
        }

        tracing::info!("serve endpoint ready, accepting connections");
        let accept_ep = ep.clone();
        let accept = tokio::spawn(async move {
            accept_loop(&accept_ep, targets).await;
        });

        // Refresh status.json when any service's active-stream count changes,
        // at most once per STATUS_FLUSH_INTERVAL — avoids disk churn under
        // busy stream churn while keeping the file near-live for operators.
        let flush = tokio::spawn(status_flush_loop(status));

        // Wait for the injected shutdown signal, then drain in-flight streams
        // before closing the endpoint (T-08). The accept and status tasks are
        // aborted first so they stop handing new connections to the pipe.
        shutdown.await;
        accept.abort();
        flush.abort();
        ep.close().await;
        Ok(())
    }
}

/// How often the status flush task re-checks stream counters.
const STATUS_FLUSH_INTERVAL: Duration = Duration::from_secs(5);

/// One served service: where to dial locally + the live stream counter.
#[derive(Clone)]
struct ServiceTarget {
    local_addr: String,
    active_streams: Arc<AtomicU64>,
}

impl ServiceTarget {
    fn new(local_addr: String) -> Self {
        Self {
            local_addr,
            active_streams: Arc::new(AtomicU64::new(0)),
        }
    }

    fn active_streams(&self) -> Arc<AtomicU64> {
        self.active_streams.clone()
    }
}

/// Status-file template: everything immutable for the life of the process,
/// plus the live per-service counters rendered into `active_connections`.
///
/// Since 0.2.0 `active_connections` counts *active streams* (in-flight pipes),
/// not iroh connections — with multiplexing one connection carries many
/// channels, so streams are what an operator cares about.
struct StatusSnapshot {
    node_id: String,
    home_relay: Option<String>,
    pid: u32,
    started_at: u64,
    services: Vec<StatusServiceRow>,
}

struct StatusServiceRow {
    name: String,
    protocol: String,
    local_addr: String,
    active_streams: Arc<AtomicU64>,
}

impl StatusSnapshot {
    fn render(&self) -> crate::status::StatusFile {
        crate::status::StatusFile {
            node_id: self.node_id.clone(),
            home_relay: self.home_relay.clone(),
            pid: self.pid,
            started_at: self.started_at,
            services: self
                .services
                .iter()
                .map(|s| crate::status::ServiceStatus {
                    name: s.name.clone(),
                    protocol: s.protocol.clone(),
                    local_addr: s.local_addr.clone(),
                    active_connections: s.active_streams.load(Ordering::Relaxed),
                })
                .collect(),
        }
    }
}

/// Periodically rewrite status.json, but only when a counter changed.
async fn status_flush_loop(status: StatusSnapshot) {
    let mut last: Vec<u64> = status
        .services
        .iter()
        .map(|s| s.active_streams.load(Ordering::Relaxed))
        .collect();
    loop {
        tokio::time::sleep(STATUS_FLUSH_INTERVAL).await;
        let now: Vec<u64> = status
            .services
            .iter()
            .map(|s| s.active_streams.load(Ordering::Relaxed))
            .collect();
        if now == last {
            continue;
        }
        last = now;
        if let Err(e) = status.render().save() {
            tracing::warn!("failed to write status file: {e}");
        }
    }
}

/// Accept connections forever, demultiplexing each to its service by ALPN.
///
/// Returns only if the endpoint is closed (e.g. after Ctrl-C). Per-connection
/// errors are logged, not propagated.
async fn accept_loop(ep: &iroh::Endpoint, targets: HashMap<Vec<u8>, ServiceTarget>) {
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

        // Demultiplex by the negotiated ALPN to find the service target.
        let alpn = conn.alpn().to_vec();
        let Some(target) = targets.get(&alpn).cloned() else {
            let name = proto::name_from_alpn(&alpn)
                .map(String::from)
                .unwrap_or_else(|| format!("{alpn:02x?}"));
            tracing::warn!("connection with unknown ALPN for service '{name}', dropping");
            continue;
        };

        // The access peer's NodeId, from its TLS cert. Logged on connect and
        // (via the watcher below) on disconnect, so operators can see who is
        // tunneling in and correlate with access-side logs. `mode` records
        // which ALPN variant negotiated this connection.
        let remote_id = conn.remote_id();
        let name = proto::name_from_alpn(&alpn)
            .map(String::from)
            .unwrap_or_else(|| format!("{alpn:02x?}"));
        let mode = if proto::is_multiplex_alpn(&alpn) {
            "multi"
        } else {
            "legacy"
        };
        tracing::info!(peer = %remote_id, service = %name, %mode, "peer connected");

        // Watcher: emit a disconnect line when the QUIC connection closes. The
        // weak handle is registered while `conn` is still alive (before the
        // stream-handling task takes it), so iroh guarantees the close event is
        // delivered even if the connection drops before this resolves.
        crate::role_run::spawn_disconnect_watcher(
            &conn,
            remote_id.to_string(),
            format!("peer disconnected (service {name}, {mode})"),
        );

        tokio::spawn(async move {
            handle_connection(&conn, target).await;
        });
    }
}

/// Supervise one connection: accept bidirectional streams in a loop, connect
/// the local service for each, and pipe bytes both ways until either side
/// closes.
///
/// Returns when the connection closes (`accept_bi` errors). Per-stream
/// failures — including a refused local dial — reset only that stream; the
/// connection and its other streams are unaffected.
async fn handle_connection(conn: &Connection, target: ServiceTarget) {
    loop {
        // accept_bi returns (SendStream, RecvStream) — send first. Our pipe
        // wants the remote pair as (read, write) = (recv, send), so swap.
        let (send, recv) = match conn.accept_bi().await {
            Ok(pair) => pair,
            Err(e) => {
                tracing::debug!("connection closed: {e}");
                return;
            }
        };

        let local = match TcpStream::connect(&target.local_addr).await {
            Ok(local) => local,
            Err(e) => {
                // Dropping the halves resets this one stream (RESET_STREAM /
                // STOP_SENDING); the access side surfaces it as a failed
                // channel while the connection stays usable.
                tracing::warn!(
                    local_addr = %target.local_addr,
                    "failed to connect local service, resetting stream: {e}"
                );
                continue;
            }
        };

        let counter = target.active_streams();
        counter.fetch_add(1, Ordering::Relaxed);
        tokio::spawn(async move {
            match crate::pipe::pipe_tcp_bidirectional(local, (recv, send)).await {
                Ok(()) => tracing::debug!("stream closed normally"),
                Err(e) => tracing::warn!("stream error: {e}"),
            }
            counter.fetch_sub(1, Ordering::Relaxed);
        });
    }
}
