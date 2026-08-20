//! Access role: consume remote services to local.
//!
//! Implements T-07. Loads the access config, builds an [`Endpoint`], and for
//! each service opens a local TCP listener. When a local client connects,
//! access opens a bidirectional QUIC stream to the remote serve peer and
//! pipes the client stream through it.
//!
//! ## Multiplexing (0.2.0)
//!
//! Each service has a `multiplex` config flag (default `true`):
//!
//! - **`true`**: the service keeps ONE long-lived iroh connection to the
//!   serve peer (dialed via the usual retry/backoff loop) and opens one
//!   bidirectional stream per local TCP connection. Handshakes are paid
//!   once. A dead connection surfaces as EOF on its channels (correct
//!   port-forward semantics) and the next channel dials a fresh one.
//! - **`false`**: one iroh connection per local TCP connection — the
//!   pre-0.2.0 behavior verbatim.
//!
//! There is deliberately NO protocol negotiation: the ALPN is unchanged
//! (`iroh-tunnel/{name}`, see [`crate::proto`]). The rollout contract is
//! serve-first — multiplexing requires a 0.2.0+ serve peer, because a
//! pre-0.2.0 serve accepts exactly one stream per connection (a second
//! stream would hang until the connection closes). Upgrade serve nodes
//! before enabling multiplexing; if an access node must talk to an older
//! serve, set `multiplex = false` on that service.
//!
//! ## Concurrency model
//!
//! - One listen-loop task per service (so each service has its own bound port).
//! - Each accepted local client becomes its own task, so a failure in one
//!   tunnel never affects another (NFR-08).
//! - `host = 0.0.0.0` binds all interfaces (share within the LAN); the
//!   default `127.0.0.1` keeps it local-only.
//!
//! ## Status file (issue #59)
//!
//! `access run` writes `access-status.json` beside the serve file (same
//! atomic write, same 5 s change-detect flush): `node_id`, `pid`,
//! `started_at`, and one row per configured service — `name`,
//! `listen_addr`, the configured serve `peer`, the live `transports` of the
//! service's multiplexed connection, and the endpoint's local UDP
//! candidates. Transports are queried fresh from the cached connection at
//! each flush (never snapshotted in the cache), and are empty while the
//! service has no live connection — `multiplex = false` services therefore
//! always show an empty list (their connections are per-channel and
//! short-lived).
//!
//! Based on Page 04 v2 §1.2 (access dial sequence) and Page 06 v5 §1.2 (access
//! run CLI behavior). Note: iroh 1.0's connect/ALPN API differs from the
//! earlier draft the spec was written against — see the API notes inline.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

use crate::config::{AccessConfig, AccessService};
use crate::endpoint;
use crate::proto;
use crate::role_run::RoleStrategy;
use crate::status::{AccessServiceStatus, AccessStatusFile, StatusPayload, StatusWriter};

/// Run the access role until interrupted (Ctrl-C).
///
/// Thin wrapper over [`crate::role_run::run_with_shutdown`] that wires the
/// real signal handler and the [`AccessStrategy`] implementer.
pub async fn run(config_path: &Path) -> Result<()> {
    crate::role_run::run_with_shutdown::<AccessStrategy>(
        config_path,
        crate::shutdown::wait_for_signal(),
    )
    .await
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
    crate::role_run::run_with_shutdown::<AccessStrategy>(config_path, shutdown).await
}

/// Run the access role like [`run_with_shutdown`], but write its status
/// file into the explicitly given `state_dir` instead of the env-resolved
/// default.
///
/// Advanced/testing seam mirroring
/// [`crate::serve::run_with_shutdown_with_state_dir`]: integration tests
/// point an access instance at an isolated tempdir without touching the
/// process-global `IROH_TUNNEL_STATE_DIR` variable (which cannot be
/// mutated safely while other test threads exist). Production callers use
/// [`run_with_shutdown`]; operators relocate the file via the env variable
/// instead.
pub async fn run_with_shutdown_with_state_dir(
    state_dir: &Path,
    config_path: &Path,
    shutdown: impl Future<Output = ()>,
) -> Result<()> {
    let state_dir = Some(state_dir.to_path_buf());
    crate::role_run::run_skeleton::<AccessStrategy, _, _>(config_path, |ep, cfg| {
        AccessStrategy::run_loop_with_state_dir(ep, cfg, shutdown, state_dir)
    })
    .await
}

/// Access-role implementation of [`RoleStrategy`].
///
/// Owns the genuinely-access-specific pieces: resolves relay URLs (falling
/// back to the n0 defaults), parses each service's `node_id`, spawns one
/// TCP listener per service.
pub(crate) struct AccessStrategy;

impl RoleStrategy for AccessStrategy {
    type Config = AccessConfig;

    async fn build_endpoint(cfg: &Self::Config) -> Result<iroh::Endpoint> {
        // Access only dials out, so no ALPNs are registered. The endpoint
        // resolves the node's secret key (generating+persisting on first run)
        // via the shared RoleDoc::resolve_and_save_key path in the skeleton.
        //
        // The relay map is the UNION of node + per-service relay_urls: every
        // service dial needs a live relay transport for its URLs (a relay
        // only carries traffic for peers connected to it). Per-service dials
        // still attach only that service's own URLs — see run_loop.
        let extra_relays: Vec<String> = {
            let mut seen = cfg.node.relay_urls.clone();
            let mut extras = Vec::new();
            for svc in &cfg.services {
                for url in &svc.relay_urls {
                    if !seen.contains(url) {
                        seen.push(url.clone());
                        extras.push(url.clone());
                    }
                }
            }
            extras
        };
        endpoint::create_access_endpoint(&cfg.node, &extra_relays).await
    }

    fn print_services(cfg: &Self::Config) {
        if cfg.services.is_empty() {
            tracing::warn!("no services configured; nothing to expose");
            return;
        }
        // Resolve and parse everything up front so a bad config fails loudly
        // before we bind any ports. The results feed both the printout and the
        // spawned listeners.
        let relay_urls: Vec<iroh::RelayUrl> = endpoint::resolve_relay_urls(&cfg.node.relay_urls)
            .unwrap_or_else(|e| {
                tracing::warn!("failed to resolve relay_urls, will fail at dial time: {e}");
                Vec::new()
            });
        if cfg.node.relay_urls.is_empty() {
            tracing::info!(
                count = relay_urls.len(),
                "no relay_urls configured, falling back to n0 default relays"
            );
        }
        for svc in &cfg.services {
            // We don't fail here — print_services is infallible in the trait
            // signature. parse errors surface later when listen_loop tries
            // to dial; printing the raw node_id preserves the operator's
            // intent so they can see what was configured.
            let node_id_display = match svc.node_id.parse::<iroh::EndpointId>() {
                Ok(id) => id.to_string(),
                Err(_) => svc.node_id.clone(),
            };
            let listen_addr = format!("{}:{}", svc.host, svc.port);
            let relay_note = if svc.relay_urls.is_empty() {
                String::new()
            } else {
                format!(" [relay override: {}]", svc.relay_urls.len())
            };
            println!(
                "Exposed: {} {listen_addr} -> peer {node_id_display} ({}://{listen_addr}){relay_note}",
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
        Self::run_loop_with_state_dir(ep, cfg, shutdown, None).await
    }
}

impl AccessStrategy {
    /// [`RoleStrategy::run_loop`] with an optional injected state dir for
    /// the status file (see [`run_with_shutdown_with_state_dir`]).
    async fn run_loop_with_state_dir(
        ep: iroh::Endpoint,
        cfg: AccessConfig,
        shutdown: impl Future<Output = ()>,
        state_dir: Option<PathBuf>,
    ) -> Result<()> {
        // Resolve the node-level relays once (the per-service fallback) and
        // parse every service's node_id before spawning listeners, so a bad
        // config fails fast. Each service resolves its own effective relay
        // list — its override when set, else the node's — so dialers carry
        // only their service's URLs (issue #54).
        let node_relay_urls = endpoint::resolve_relay_urls(&cfg.node.relay_urls)?;
        if cfg.node.relay_urls.is_empty() {
            tracing::info!(
                count = node_relay_urls.len(),
                "no relay_urls configured, falling back to n0 default relays"
            );
        }

        let mut handles = Vec::new();
        // One dialer per service, kept alive alongside the listener tasks:
        // the status flush task reads each service's live connection path
        // from its dialer (issue #59).
        let mut dialers: Vec<Arc<ServiceDialer>> = Vec::new();
        let mut status_services: Vec<AccessStatusServiceRow> = Vec::new();
        for svc in &cfg.services {
            let node_id = svc
                .node_id
                .parse::<iroh::EndpointId>()
                .with_context(|| format!("invalid node_id: {}", svc.node_id))?;
            // Raw `host:port` is what TcpListener::bind consumes; the
            // status row renders the bracketed-IPv6 form via
            // format_local_addr — the same normalization as the serve
            // schema's `local_addr`.
            let bind_addr = format!("{}:{}", svc.host, svc.port);
            let effective: Vec<iroh::RelayUrl> = if svc.relay_urls.is_empty() {
                node_relay_urls.clone()
            } else {
                endpoint::resolve_relay_urls(&svc.relay_urls)
                    .with_context(|| format!("service '{}': invalid relay_urls", svc.name))?
            };
            let dialer = Arc::new(ServiceDialer::new(&ep, node_id, svc, &effective, bind_addr));
            status_services.push(AccessStatusServiceRow {
                name: svc.name.clone(),
                listen_addr: crate::status::format_local_addr(&svc.host, svc.port),
                // The parsed id, not the raw config string — the status row
                // is normalized exactly like every other rendered id.
                peer: node_id.to_string(),
            });
            dialers.push(dialer);
        }
        // The initial status write happens BEFORE any listener spawns (the
        // same ordering as serve): no local client can have connected yet,
        // so the first snapshot deterministically carries every service's
        // configured peer with empty transports.

        // Operator-facing status snapshot (issue #59), refreshed by the
        // flush task below as connections come and go. Best-effort: a
        // failure to write status is logged but does not stop the tunnel.
        // Same seeding contract as serve's: the initial write seeds the
        // change detection when it succeeds; a failure seeds `None` so the
        // first tick retries it.
        let status = AccessStatusTemplate {
            node_id: crate::endpoint::node_id_string(&ep),
            pid: std::process::id(),
            started_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            services: status_services,
            state_dir,
        };
        let initial = render_access_status(&status, &dialers).await;
        let seeded = match save_access_status(initial.clone(), status.state_dir.clone()).await {
            Ok(p) => {
                tracing::info!(path = %p.display(), "wrote status file");
                Some(initial)
            }
            Err(e) => {
                tracing::warn!("failed to write status file: {e}");
                None
            }
        };

        tracing::info!("access endpoint ready, listening for local clients");
        let flush = tokio::spawn(access_status_flush_loop(status, dialers.clone(), seeded));
        for dialer in dialers {
            handles.push(tokio::spawn(listen_loop(dialer)));
        }
        shutdown.await;
        // Abort each per-service listener so they stop accepting new local
        // clients before the endpoint close tears down the in-flight dials;
        // the flush task with it.
        for h in handles {
            h.abort();
        }
        flush.abort();
        ep.close().await;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Status file (issue #59)
// ---------------------------------------------------------------------------

/// Access-status template: everything immutable for the life of the
/// process. The live per-service transports are rendered into each snapshot
/// at flush time from the dialers' cached connections.
struct AccessStatusTemplate {
    node_id: String,
    pid: u32,
    started_at: u64,
    services: Vec<AccessStatusServiceRow>,
    /// Injected state dir (testing seam); `None` → env-resolved default.
    state_dir: Option<PathBuf>,
}

/// One immutable per-service row of [`AccessStatusTemplate`].
struct AccessStatusServiceRow {
    name: String,
    listen_addr: String,
    /// Configured serve peer (full node id) — shown even before the first
    /// connection, so operators can tell "misconfigured peer" apart from
    /// "not connected yet".
    peer: String,
}

/// Render the template into the status file: per service, a fresh
/// connection-path report of its multiplexed connection (queried NOW, never
/// a cached snapshot — path migrations can't go stale between flushes), or
/// the configured peer with empty transports while disconnected.
async fn render_access_status(
    status: &AccessStatusTemplate,
    dialers: &[Arc<ServiceDialer>],
) -> AccessStatusFile {
    debug_assert_eq!(
        status.services.len(),
        dialers.len(),
        "one dialer per status row"
    );
    let mut services = Vec::with_capacity(status.services.len());
    for (row, dialer) in status.services.iter().zip(dialers) {
        let (transports, local_bound_addrs) = match dialer.path_report().await {
            Some(report) => (report.transports, report.local_bound_addrs),
            None => (Vec::new(), Vec::new()),
        };
        services.push(AccessServiceStatus {
            name: row.name.clone(),
            listen_addr: row.listen_addr.clone(),
            peer: row.peer.clone(),
            transports,
            local_bound_addrs,
        });
    }
    AccessStatusFile {
        node_id: status.node_id.clone(),
        pid: status.pid,
        started_at: status.started_at,
        services,
    }
}

/// Periodically rewrite access-status.json, but only when the rendered
/// snapshot changed (a service connected or disconnected, or its transports
/// migrated) — the access twin of serve's `status_flush_loop`.
async fn access_status_flush_loop(
    status: AccessStatusTemplate,
    dialers: Vec<Arc<ServiceDialer>>,
    mut last: Option<AccessStatusFile>,
) {
    loop {
        tokio::time::sleep(crate::status::STATUS_FLUSH_INTERVAL).await;
        let file = render_access_status(&status, &dialers).await;
        if last.as_ref() == Some(&file) {
            continue;
        }
        // Only record the file as written on success, so a failed write is
        // retried on the next tick rather than silently dropped until the
        // next change.
        match save_access_status(file.clone(), status.state_dir.clone()).await {
            Ok(_) => last = Some(file),
            Err(e) => tracing::warn!("failed to write status file: {e}"),
        }
    }
}

/// Persist the rendered access status file via the shared writer.
///
/// Runs on the blocking pool: the atomic save fsyncs, and a stalled disk
/// must never stall an async worker (the listen loops share this runtime).
async fn save_access_status(
    file: AccessStatusFile,
    state_dir: Option<std::path::PathBuf>,
) -> Result<std::path::PathBuf> {
    let writer = StatusWriter::access();
    let payload = StatusPayload::Access(file);
    tokio::task::spawn_blocking(move || writer.save_with_state_dir(state_dir.as_deref(), &payload))
        .await
        .context("status save task failed")?
}

/// Per-service dialing state shared by all local-client tasks.
///
/// Owns the multiplexed-connection cache. The `multiplex = false` path is
/// stateless here — every channel dials its own connection.
struct ServiceDialer {
    ep: iroh::Endpoint,
    addr: iroh::EndpointAddr,
    /// The service ALPN — unchanged between modes (no negotiation).
    alpn: Vec<u8>,
    svc_name: String,
    listen_addr: String,
    multiplex: bool,
    /// The shared multiplexed connection and its poller, behind ONE lock:
    /// the one-connection-and-one-poller-per-service rule is structural —
    /// a redial replaces both atomically under the same guard, with no
    /// cross-lock ordering to reason about. Concurrent channels serialize
    /// the (re)dial here instead of racing N dials.
    state: Mutex<ConnState>,
}

/// The cached multiplexed connection plus its path-change poller —
/// the pair `get_or_dial` swaps atomically on every (re)dial.
struct ConnState {
    conn: Option<iroh::endpoint::Connection>,
    poller: Option<tokio::task::AbortHandle>,
}

impl ServiceDialer {
    fn new(
        ep: &iroh::Endpoint,
        node_id: iroh::EndpointId,
        svc: &AccessService,
        relay_urls: &[iroh::RelayUrl],
        listen_addr: String,
    ) -> Self {
        // build_dial_addr attaches every relay URL so iroh can try each in
        // turn; resolve_relay_urls guarantees a non-empty list (n0 defaults
        // fallback, IROHTUN-44).
        Self {
            ep: ep.clone(),
            addr: endpoint::build_dial_addr(node_id, relay_urls),
            alpn: proto::alpn_for(&svc.name),
            svc_name: svc.name.clone(),
            listen_addr,
            multiplex: svc.multiplex,
            state: Mutex::new(ConnState {
                conn: None,
                poller: None,
            }),
        }
    }
    /// Get the shared multiplexed connection, dialing it if there is none.
    ///
    /// The lock is held across the retry loop on purpose: N concurrent
    /// channels produce exactly one connection, not N.
    async fn get_or_dial(&self) -> Result<iroh::endpoint::Connection> {
        let mut guard = self.state.lock().await;
        if let Some(conn) = guard.conn.as_ref() {
            return Ok(conn.clone());
        }
        let conn = crate::role_run::connect_with_retry(&self.ep, &self.addr, &self.alpn).await?;
        let remote_id = conn.remote_id();

        // One snapshot, two consumers. The query awaits UNDER the guard —
        // correctness over lock-holder courtesy: sharing this snapshot
        // between the established line and the poller baseline closes the
        // blind window where a fast hole punch (landing inside the first
        // poll tick) would be swallowed by the poller's silent seeding and
        // never diffed or logged. A `None` report (paths pending) degrades
        // to an empty baseline — the seed path handles exactly that case.
        let report = crate::conn_path::peer_path_report(&self.ep, remote_id).await;
        let baseline = report
            .as_ref()
            .map(|r| r.transports.clone())
            .unwrap_or_default();

        // Swap poller + connection atomically under the same guard, so a
        // racing invalidate+redial can never end with the OLD poller as the
        // survivor. The old poller also ends by itself once its connection
        // closes — the abort is just eager cleanup.
        if let Some(old) = guard.poller.take() {
            old.abort();
        }
        let poller = spawn_path_change_poller(&self.ep, &conn, self.svc_name.clone(), baseline);
        *guard = ConnState {
            conn: Some(conn.clone()),
            poller: Some(poller),
        };
        drop(guard);

        // Emit after publish (no guard needed — the snapshot is in hand).
        log_connection_established(remote_id, &self.svc_name, "multiplexed", report.as_ref());
        crate::role_run::spawn_disconnect_watcher(
            &conn,
            remote_id.to_string(),
            format!(
                "disconnected from serve peer (service {}, multiplexed)",
                self.svc_name
            ),
        );
        Ok(conn)
    }

    /// Drop the cached multiplexed connection (it died); the next channel
    /// dials a fresh one. The poller is left alone — it ends by itself when
    /// the connection closes, and the next `get_or_dial` replaces it.
    async fn invalidate(&self) {
        self.state.lock().await.conn = None;
    }

    /// Fresh connection-path report for the cached multiplexed connection,
    /// or `None` while the service has no live one (issue #59).
    ///
    /// The guard is dropped BEFORE the endpoint query so a status render
    /// never holds channels off their dial path, and the report is queried
    /// fresh on every call — never cached — so the status file can't serve
    /// a stale path after a migration.
    async fn path_report(&self) -> Option<crate::conn_path::PeerPathReport> {
        let peer = {
            let guard = self.state.lock().await;
            guard.conn.as_ref().map(|conn| conn.remote_id())?
        };
        crate::conn_path::peer_path_report(&self.ep, peer).await
    }
}

/// Bind the service's `listen_addr` and, for each local client, tunnel it.
///
/// Returns only if the listener errors fatally (e.g. the bound socket closes).
/// Per-client errors are logged, not propagated.
async fn listen_loop(dialer: Arc<ServiceDialer>) {
    let listen_addr = dialer.listen_addr.clone();
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
                let dialer = dialer.clone();
                tokio::spawn(async move {
                    match handle_local_connection(&dialer, local_stream).await {
                        Ok(()) => {
                            tracing::debug!(%peer_addr, svc = %dialer.svc_name, "tunnel closed")
                        }
                        Err(e) => {
                            tracing::warn!(%peer_addr, svc = %dialer.svc_name, "tunnel error: {e}")
                        }
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

/// Tunnel one local client according to the service's `multiplex` flag.
///
/// See the module docs for the rollout contract. Errors close only this
/// local client's channel.
async fn handle_local_connection(dialer: &ServiceDialer, local: TcpStream) -> Result<()> {
    if !dialer.multiplex {
        return dialer.pipe_per_channel(local).await;
    }
    // Multiplexed: one stream on the shared connection per channel. If
    // `open_bi` fails with a connection-level error, the cache is dropped and
    // exactly one redial + retry is attempted before giving up — a dead
    // connection surfaces as EOF-style errors on the active channels and the
    // next channel gets a fresh one.
    let conn = dialer.get_or_dial().await?;
    match conn.open_bi().await {
        // open_bi returns (SendStream, RecvStream) — send first. Our pipe
        // wants the remote pair as (read, write) = (recv, send), so swap.
        Ok((send, recv)) => crate::pipe::pipe_tcp_bidirectional(local, (recv, send)).await,
        Err(e) => {
            tracing::warn!(
                svc = %dialer.svc_name,
                "multiplexed open_bi failed: {e}, redialing once"
            );
            dialer.invalidate().await;
            let conn = dialer.get_or_dial().await?;
            let (send, recv) = conn
                .open_bi()
                .await
                .context("open bidirectional stream failed after redial")?;
            crate::pipe::pipe_tcp_bidirectional(local, (recv, send)).await
        }
    }
}

impl ServiceDialer {
    /// `multiplex = false`: exactly the pre-0.2.0 behavior — one iroh
    /// connection per local client, retried with backoff until it connects.
    async fn pipe_per_channel(&self, local: TcpStream) -> Result<()> {
        // iroh 1.0's Endpoint::connect does NOT reuse an existing connection
        // — every call is a fresh QUIC connection (relay session + TLS
        // handshake). That is precisely the cost multiplexing removes; this
        // path keeps the pre-0.2.0 one-connection-per-channel semantics.
        let conn = crate::role_run::connect_with_retry(&self.ep, &self.addr, &self.alpn).await?;

        let remote_id = conn.remote_id();
        // No poller here: a per-channel connection lives for exactly one
        // channel (see module docs), so its established line is all the
        // path context it will ever get.
        let report = crate::conn_path::peer_path_report(&self.ep, remote_id).await;
        log_connection_established(remote_id, &self.svc_name, "per-channel", report.as_ref());
        crate::role_run::spawn_disconnect_watcher(
            &conn,
            remote_id.to_string(),
            format!("disconnected from serve peer (service {})", self.svc_name),
        );

        let (send, recv) = conn
            .open_bi()
            .await
            .context("open bidirectional stream failed")?;
        crate::pipe::pipe_tcp_bidirectional(local, (recv, send)).await?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Connection-path logging (issue #58)
// ---------------------------------------------------------------------------

/// How often the multiplexed path-change poller re-snapshots the peer's
/// transports.
///
/// Same rationale as serve's `STATUS_FLUSH_INTERVAL`: 5 s is near-live for
/// operators while keeping the endpoint query off the hot path — the poller
/// runs only while its connection lives.
///
/// Exposed as `pub` only under the `test-utils` feature so integration
/// tests derive their wait windows from the real interval instead of a
/// drifting copy.
/// Single source of the poller cadence value — both cfg variants of
/// [`PATH_POLL_INTERVAL`] below derive from it, so the value can never
/// drift between the pub (test-utils) and private twins.
const PATH_POLL_INTERVAL_SECS: u64 = 5;

#[cfg(feature = "test-utils")]
pub const PATH_POLL_INTERVAL: Duration = Duration::from_secs(PATH_POLL_INTERVAL_SECS);

/// Private twin of the [`PATH_POLL_INTERVAL`] definition above for builds
/// without `test-utils`.
#[cfg(not(feature = "test-utils"))]
const PATH_POLL_INTERVAL: Duration = Duration::from_secs(PATH_POLL_INTERVAL_SECS);

/// First 8 chars of the peer id + `…`, for log-message readability.
///
/// Delegates to the shared [`crate::conn_path::short_peer_id`] — the same
/// shape the `<role> status` tables render — so a short id means one thing
/// everywhere. The full id stays in the `peer=` field so both hosts can be
/// correlated by grepping the same string.
fn short_peer_id(peer: &iroh::EndpointId) -> String {
    crate::conn_path::short_peer_id(&peer.to_string())
}

/// Render a fresh connection's active transports for its established log
/// line — or `paths pending` when iroh has no active-path snapshot yet
/// (queried immediately after the handshake).
fn render_established_paths(report: Option<&crate::conn_path::PeerPathReport>) -> String {
    report
        .map(|r| crate::conn_path::render_active_transports(&r.transports))
        .filter(|rendered| !rendered.is_empty())
        .unwrap_or_else(|| "paths pending".to_string())
}

/// Emit the connection-established line shared by both dial paths: full
/// peer id in the `peer=` field, short id plus the active transports
/// (`relay=<url>` / `direct=<addr>`, comma-separated) in the message.
/// `mode` is `"multiplexed"` or `"per-channel"`.
///
/// Pure — no endpoint query; callers hand in the snapshot they already
/// own (the multiplexed path shares one query with the poller baseline).
fn log_connection_established(
    remote_id: iroh::EndpointId,
    svc_name: &str,
    mode: &str,
    report: Option<&crate::conn_path::PeerPathReport>,
) {
    tracing::info!(
        peer = %remote_id,
        svc_name = %svc_name,
        "connected to serve peer ({}, {}) via {}",
        mode,
        short_peer_id(&remote_id),
        render_established_paths(report),
    );
}

/// Spawn the path-change poller for one live multiplexed connection.
///
/// iroh migrates between relay and direct paths silently — a hole punch
/// succeeds, a direct path dies and traffic falls back to relay — while the
/// multiplexed connection outlives all of them. The poller snapshots the
/// peer's transports every [`PATH_POLL_INTERVAL`] and logs exactly one line
/// per real change (what counts as "real" is
/// [`crate::conn_path::diff_transports`]).
///
/// Semantics: it watches the PEER-level remote map, which iroh shares
/// across every connection to the same peer — a logged transition can be
/// driven by a *different* connection's traffic (e.g. the per-channel
/// service to the same serve peer completing a hole punch), not only by
/// this connection. N multiplexed services dialing the same peer therefore
/// each log their own line per transition (differing only in `svc_name`)
/// and run N remote_info queries per interval; operators deduplicate by
/// the `peer=` field.
///
/// Lifetime: it holds only a *weak* connection handle (never keeps the
/// connection alive) and its loop is bounded by `closed()` — the task ends
/// with the connection. The returned [`tokio::task::AbortHandle`] lets the
/// owner end it earlier on redial (one poller per service).
fn spawn_path_change_poller(
    ep: &iroh::Endpoint,
    conn: &iroh::endpoint::Connection,
    svc_name: String,
    initial: Vec<crate::conn_path::TransportStatus>,
) -> tokio::task::AbortHandle {
    let peer = conn.remote_id();
    let weak = conn.weak_handle();
    let ep = ep.clone();
    tokio::spawn(async move {
        let mut last = initial;
        loop {
            // `biased` so a dead connection always wins the race against a
            // simultaneously-elapsed sleep: exit without one last useless
            // snapshot.
            tokio::select! {
                biased;
                _ = weak.closed() => break,
                _ = tokio::time::sleep(PATH_POLL_INTERVAL) => {}
            }
            // The peer's remote-map entry can briefly disappear while iroh
            // re-negotiates paths; nothing to diff then — the next tick
            // re-checks.
            let Some(report) = crate::conn_path::peer_path_report(&ep, peer).await else {
                continue;
            };
            // One pure tick (see `conn_path::poller_step`): silent seed
            // while the baseline has never seen an active path, baseline
            // kept through teardown blips, otherwise a single line per
            // real migration.
            let (line, next) = crate::conn_path::poller_step(&last, report.transports);
            if let Some(line) = line {
                tracing::info!(peer = %peer, svc_name = %svc_name, "{}", line);
            }
            last = next;
        }
    })
    .abort_handle()
}
