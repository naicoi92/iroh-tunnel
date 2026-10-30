//! Access role: consume remote services to local.
//!
//! Implements T-07. Loads the access config, builds an [`Endpoint`], and for
//! each service opens a local TCP listener. When a local client connects,
//! access opens a bidirectional QUIC stream to the remote serve peer and
//! pipes the client stream through it.
//!
//! ## Multiplexing
//!
//! Every TCP service keeps one long-lived Iroh connection to the Serve peer
//! and opens one bidirectional stream per local TCP connection. Legacy
//! `multiplex` config values are ignored. A dead connection closes its channels;
//! the next local client dials a fresh connection. The ALPN is unchanged
//! (`iroh-tunnel/{name}`, see [`crate::proto`]); no negotiation is added.
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
//! service has no live connection.
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
use tokio::task::JoinSet;

use crate::shutdown;

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

/// Run an in-memory quick configuration without persisting its identity.
pub async fn run_quick(cfg: AccessConfig) -> Result<()> {
    cfg.validate()
        .map_err(|e| crate::error::CliError::Config(format!("access quick mode: {e}")))?;
    crate::role_run::run_config::<AccessStrategy, _, _>(cfg, |ep, cfg| {
        AccessStrategy::run_loop(ep, cfg, crate::shutdown::wait_for_signal())
    })
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

        let mut listeners = JoinSet::new();
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

        // Graceful-cleanup inputs captured BEFORE `status` moves into the
        // flush task: the state-dir choice (the ownership-checked removal
        // resolves its own path from it) and the cooperative stop channel.
        let cleanup_state_dir = status.state_dir.clone();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();

        tracing::info!("access endpoint ready, listening for local clients");
        let mut flush = tokio::spawn(access_status_flush_loop(
            status,
            dialers.clone(),
            seeded,
            stop_rx,
            crate::status::STATUS_FLUSH_INTERVAL,
            save_access_status,
        ));
        let (stop, stop_rx) = shutdown::channel();
        for dialer in &dialers {
            listeners.spawn(listen_loop(dialer.clone(), stop_rx.clone()));
        }
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                biased;
                _ = &mut shutdown => break,
                Some(result) = listeners.join_next(), if !listeners.is_empty() => shutdown::observe(result),
            }
        }
        shutdown::begin(&stop);
        // Listeners seal admission and join their own channels against the
        // shared deadline. Never abort a supervisor with live descendants.
        shutdown::join(&mut listeners).await;
        for dialer in &dialers {
            dialer.stop_auxiliary_tasks().await;
        }
        // Signal the flush task to stop (dropping the sender alone would
        // also close the channel, but only at scope end — far too late).
        let _ = stop_tx.send(());
        // Stop the flush task COOPERATIVELY and JOIN it: aborting would not
        // cancel an in-flight spawn_blocking save (write+fsync+rename runs
        // to completion on the blocking pool and could recreate the file
        // AFTER the cleanup removal). The task exits only at an await
        // point and every save is awaited inside its loop — a successful
        // join therefore proves none of its saves is still running. On a
        // pathological stall (>5 s for one fsync) the join is abandoned
        // and the removal is SKIPPED: a possibly-stale file (surfaced by
        // the reader-side pid warning) beats a resurrection race that
        // cannot be won once the save is in-flight.
        let flush_abort = flush.abort_handle();
        let flush_stopped = match tokio::time::timeout(Duration::from_secs(5), &mut flush).await {
            Ok(result) => {
                let clean = result.is_ok();
                shutdown::observe(result);
                clean
            }
            Err(_) => {
                tracing::warn!("status flush task did not stop within 5s; keeping status file");
                flush_abort.abort();
                shutdown::observe(flush.await);
                false
            }
        };
        ep.close().await;
        // Remove the status file only after a clean join AND only if it
        // still belongs to THIS process (its `pid` field decides — two
        // instances can share a state dir, and an idle peer would not
        // recreate its own file after a blind removal). A crash never runs
        // this path; the reader-side pid-liveness warning covers the stale
        // file it leaves behind.
        if flush_stopped {
            crate::status::remove_own_status_file(
                StatusWriter::access(),
                cleanup_state_dir.as_deref(),
            )
            .await;
        }
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
///
/// Stops COOPERATIVELY on `stop`: aborting this task would not cancel an
/// in-flight `spawn_blocking` save (write+fsync+rename runs to completion
/// on the blocking pool and could recreate the file after the shutdown
/// cleanup removed it). The run loop therefore signals, then JOINS this
/// task before removing the file — the loop here must exit on the signal
/// while letting the current save finish.
///
/// `interval` and `save` are seams for the unit test that drives a slow
/// save against the stop signal; production passes
/// [`crate::status::STATUS_FLUSH_INTERVAL`] and [`save_access_status`].
async fn access_status_flush_loop<S, F>(
    status: AccessStatusTemplate,
    dialers: Vec<Arc<ServiceDialer>>,
    mut last: Option<AccessStatusFile>,
    mut stop: tokio::sync::oneshot::Receiver<()>,
    interval: Duration,
    mut save: S,
) where
    S: FnMut(AccessStatusFile, Option<std::path::PathBuf>) -> F,
    F: Future<Output = Result<std::path::PathBuf>>,
{
    loop {
        // `biased` so a stop that arrives during a save is observed before
        // the next sleep even starts — no extra tick, no extra save.
        tokio::select! {
            biased;
            _ = &mut stop => break,
            _ = tokio::time::sleep(interval) => {}
        }
        let file = render_access_status(&status, &dialers).await;
        if last.as_ref() == Some(&file) {
            continue;
        }
        // Only record the file as written on success, so a failed write is
        // retried on the next tick rather than silently dropped until the
        // next change.
        match save(file.clone(), status.state_dir.clone()).await {
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
/// Owns the shared connection cache used by every local client.
struct ServiceDialer {
    ep: iroh::Endpoint,
    addr: iroh::EndpointAddr,
    /// The service ALPN — unchanged between modes (no negotiation).
    alpn: Vec<u8>,
    svc_name: String,
    listen_addr: String,
    /// The shared multiplexed connection and its poller, behind ONE lock:
    /// the one-connection-and-one-poller-per-service rule is structural —
    /// a redial replaces both atomically under the same guard, with no
    /// cross-lock ordering to reason about. Concurrent channels serialize
    /// the (re)dial here instead of racing N dials.
    state: Mutex<ConnState>,
    /// Serializes dialing without holding `state`; see `get_or_dial`.
    dial: Mutex<()>,
}

/// Cached connection and owned path-poller/disconnect-watcher tasks.
/// Replacement and cancellation retain ownership under the same lock.
struct ConnState {
    conn: Option<iroh::endpoint::Connection>,
    tasks: JoinSet<()>,
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
            state: Mutex::new(ConnState {
                conn: None,
                tasks: JoinSet::new(),
            }),
            dial: Mutex::new(()),
        }
    }
    /// Get the shared multiplexed connection, dialing it if there is none.
    ///
    /// `dial` is held across the retry loop on purpose: N concurrent
    /// channels produce exactly one connection, not N. The `state` lock is
    /// only ever held briefly (never across the dial or backoff), so status
    /// rendering and shutdown are not blocked by a slow dial.
    ///
    /// The connection is stored in the same synchronous step that follows a
    /// successful dial, so a caller deadline cannot cancel the future in
    /// between and discard a dial that already succeeded.
    async fn get_or_dial(&self) -> Result<iroh::endpoint::Connection> {
        let _dialing = self.dial.lock().await;
        if let Some(conn) = self.state.lock().await.conn.as_ref() {
            return Ok(conn.clone());
        }
        let conn = crate::role_run::connect_with_retry(&self.ep, &self.addr, &self.alpn).await?;
        let remote_id = conn.remote_id();
        {
            let mut state = self.state.lock().await;
            // Aborted tasks of a replaced connection are only reaped here;
            // the set stays owned by the state until `stop_auxiliary_tasks`.
            state.tasks.abort_all();
            while state.tasks.try_join_next().is_some() {}
            state.tasks.spawn(crate::role_run::disconnect_watcher(
                &conn,
                remote_id.to_string(),
                format!(
                    "disconnected from serve peer (service {}, multiplexed)",
                    self.svc_name
                ),
            ));
            state.conn = Some(conn.clone());
        }

        // One snapshot, two consumers: sharing it between the established
        // line and the poller baseline closes the blind window where a fast
        // hole punch (landing inside the first poll tick) would be swallowed
        // by the poller's silent seeding and never diffed or logged. A
        // `None` report (paths pending) degrades to an empty baseline — the
        // seed path handles exactly that case. The connection is already
        // stored, so cancellation here only loses the poller.
        let report = crate::conn_path::peer_path_report(&self.ep, remote_id).await;
        let baseline = report
            .as_ref()
            .map(|r| r.transports.clone())
            .unwrap_or_default();
        {
            let mut state = self.state.lock().await;
            // A concurrent invalidate/redial may have replaced this
            // connection while the report was pending; its poller belongs
            // to the replacement, not to us.
            if state
                .conn
                .as_ref()
                .is_some_and(|c| c.stable_id() == conn.stable_id())
            {
                let ConnState { tasks, .. } = &mut *state;
                spawn_path_change_poller(&self.ep, &conn, self.svc_name.clone(), baseline, tasks);
            }
        }
        log_connection_established(remote_id, &self.svc_name, report.as_ref());
        Ok(conn)
    }

    async fn stop_auxiliary_tasks(&self) {
        let mut state = self.state.lock().await;
        state.tasks.abort_all();
        shutdown::join(&mut state.tasks).await;
    }

    /// Drop the cached multiplexed connection (it died); the next channel
    /// dials a fresh one. The poller is left alone — it ends by itself when
    /// the connection closes, and the next `get_or_dial` replaces it.
    ///
    /// Only evicts `failed` itself: a channel holding a stale connection must
    /// not evict a healthy one another channel has since redialed.
    async fn invalidate(&self, failed: &iroh::endpoint::Connection) {
        let mut state = self.state.lock().await;
        if state
            .conn
            .as_ref()
            .is_some_and(|c| c.stable_id() == failed.stable_id())
        {
            state.conn = None;
        }
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
/// Stop seals admission before draining owned clients. Per-client errors are
/// logged, not propagated.
async fn listen_loop(dialer: Arc<ServiceDialer>, mut stop: shutdown::Stop) {
    let listen_addr = dialer.listen_addr.clone();
    let listener = tokio::select! {
        biased;
        _ = shutdown::stopped(&mut stop) => return,
        result = TcpListener::bind(&listen_addr) => match result {
            Ok(listener) => listener,
            Err(error) => {
                tracing::error!("failed to bind {listen_addr}: {error}");
                return;
            }
        }
    };
    tracing::info!(%listen_addr, "listening for local clients");
    let mut clients = JoinSet::new();
    let deadline = loop {
        tokio::select! {
            biased;
            deadline = shutdown::stopped(&mut stop) => break deadline,
            Some(result) = clients.join_next(), if !clients.is_empty() => shutdown::observe(result),
            result = listener.accept() => match result {
                Ok((local_stream, peer_addr)) => {
                    let dialer = dialer.clone();
                    let stop = stop.clone();
                    let deadline = tokio::time::Instant::now() + CLIENT_CONNECT_DEADLINE;
                    clients.spawn(async move {
                        match handle_local_connection(&dialer, local_stream, deadline, stop).await {
                            Ok(()) => tracing::debug!(%peer_addr, svc = %dialer.svc_name, "tunnel closed"),
                            Err(error) => tracing::warn!(%peer_addr, svc = %dialer.svc_name, "tunnel error: {error}"),
                        }
                    });
                }
                Err(error) => {
                    tracing::warn!("accept error on {listen_addr}: {error}");
                    tokio::select! {
                        deadline = shutdown::stopped(&mut stop) => break deadline,
                        _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                    }
                }
            }
        }
    };
    drop(listener);
    shutdown::drain(&mut clients, deadline).await;
}

// A single budget includes mutex wait, dial, backoff and opening the stream.
// Keep established streams outside this timeout. See issue-86-retry-evidence.md.
const CLIENT_CONNECT_DEADLINE: Duration = Duration::from_secs(30);

/// Tunnel one local client over a stream on the shared service connection.
/// Errors close only this local client's channel.
async fn handle_local_connection(
    dialer: &ServiceDialer,
    local: TcpStream,
    deadline: tokio::time::Instant,
    mut stop: shutdown::Stop,
) -> Result<()> {
    let streams = tokio::select! {
        biased;
        _ = shutdown::stopped(&mut stop) => anyhow::bail!("client connection cancelled on shutdown"),
        result = tokio::time::timeout_at(deadline, open_client_stream(dialer)) => {
            result.context("client connection deadline expired")??
        }
    };
    // QUIC shutdown queues FIN; pipe completion alone does not prove final
    // request bytes reached Serve. Keep this task owned until acknowledged.
    let delivered = streams.0.stopped();
    crate::pipe::pipe_tcp_bidirectional(local, (streams.1, streams.0)).await?;
    match delivered.await.context("client stream delivery failed")? {
        None => Ok(()),
        Some(code) => anyhow::bail!("peer stopped client stream: {code}"),
    }
}

async fn open_client_stream(
    dialer: &ServiceDialer,
) -> Result<(iroh::endpoint::SendStream, iroh::endpoint::RecvStream)> {
    // Multiplexed: one stream on the shared connection per channel. If
    // `open_bi` fails with a connection-level error, the cache is dropped and
    // exactly one redial + retry is attempted before giving up — a dead
    // connection surfaces as EOF-style errors on the active channels and the
    // next channel gets a fresh one.
    let conn = dialer.get_or_dial().await?;
    match conn.open_bi().await {
        // open_bi returns (SendStream, RecvStream) — send first. Our pipe
        // wants the remote pair as (read, write) = (recv, send), so swap.
        Ok(streams) => Ok(streams),
        Err(e) => {
            tracing::warn!(
                svc = %dialer.svc_name,
                "multiplexed open_bi failed: {e}, redialing once"
            );
            dialer.invalidate(&conn).await;
            let conn = dialer.get_or_dial().await?;
            conn.open_bi()
                .await
                .context("open bidirectional stream failed after redial")
        }
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

/// Delegate to the shared renderer in [`crate::conn_path`] — see its docs.
fn render_established_paths(report: Option<&crate::conn_path::PeerPathReport>) -> String {
    crate::conn_path::render_established_paths(report)
}

/// Emit the connection-established line: full
/// peer id in the `peer=` field, short id plus the active transports
/// (`relay=<url>` / `direct=<addr>`, comma-separated) in the message.
///
/// Pure — no endpoint query; callers hand in the snapshot they already
/// own (the multiplexed path shares one query with the poller baseline).
fn log_connection_established(
    remote_id: iroh::EndpointId,
    svc_name: &str,
    report: Option<&crate::conn_path::PeerPathReport>,
) {
    tracing::info!(
        peer = %remote_id,
        svc_name = %svc_name,
        "connected to serve peer (multiplexed, {}) via {}",
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
/// driven by a *different* service connection completing a hole punch, not only by
/// this connection. N multiplexed services dialing the same peer therefore
/// each log their own line per transition (differing only in `svc_name`)
/// and run N remote_info queries per interval; operators deduplicate by
/// the `peer=` field.
///
/// Lifetime: it holds only a *weak* connection handle (never keeps the
/// connection alive) and its loop is bounded by `closed()` — the task ends
/// with the connection. The owner aborts and joins it on redial or after
/// channel drain (one poller per service).
fn spawn_path_change_poller(
    ep: &iroh::Endpoint,
    conn: &iroh::endpoint::Connection,
    svc_name: String,
    initial: Vec<crate::conn_path::TransportStatus>,
    tasks: &mut JoinSet<()>,
) {
    let peer = conn.remote_id();
    let weak = conn.weak_handle();
    let ep = ep.clone();
    tasks.spawn(async move {
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
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// PR #62 run 4, acceptance A — the shutdown race, driven directly.
    ///
    /// `stop` arriving while a save is IN-FLIGHT must let that save finish
    /// (the run loop joins the task before removing the file), and must
    /// prevent any SECOND save: a post-stop tick is exactly the
    /// resurrection path (rename after removal) that cooperative stopping
    /// exists to close.
    ///
    /// The spy save always FAILS on purpose: a failed save leaves `last`
    /// unseeded, so every subsequent tick would save again if the loop
    /// wrongly kept ticking after `stop` — call count 1 is therefore a
    /// strong assertion, not a vacuous one.
    #[tokio::test]
    async fn flush_stop_mid_save_lets_it_finish_and_prevents_a_second_save() {
        let tmp = tempfile::tempdir().unwrap();
        let template = AccessStatusTemplate {
            node_id: "race-test".to_string(),
            pid: 1,
            started_at: 0,
            // No services → no dialers → the render is trivially stable;
            // the save seam below is the only observable.
            services: Vec::new(),
            state_dir: Some(tmp.path().to_path_buf()),
        };
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        // Signalled the moment the slow save STARTS, so the stop below is
        // provably sent while that save is in-flight.
        let (started_tx, mut started_rx) = tokio::sync::mpsc::channel::<()>(1);
        let calls = Arc::new(AtomicUsize::new(0));

        let calls_for_save = calls.clone();
        let started_for_save = started_tx.clone();
        let flush = tokio::spawn(access_status_flush_loop(
            template,
            Vec::new(),
            None, // unseeded `last`: the first tick always saves
            stop_rx,
            Duration::from_millis(20),
            move |_file, dir| {
                let calls = calls_for_save.clone();
                let started = started_for_save.clone();
                async move {
                    let n = calls.fetch_add(1, Ordering::SeqCst);
                    if n == 0 {
                        let _ = started.try_send(());
                        // The deliberately slow "blocking" save: its write
                        // lands AFTER the stop signal below is sent.
                        tokio::time::sleep(Duration::from_millis(300)).await;
                        let dir = dir.expect("test always injects a state dir");
                        std::fs::write(dir.join("access-status.json"), b"slow-save-landed")
                            .expect("spy save write");
                    }
                    Err(anyhow::anyhow!("spy save always fails"))
                }
            },
        ));

        // Synchronize on the slow save being in-flight, THEN stop.
        started_rx
            .recv()
            .await
            .expect("first save must start before the test stops the loop");
        let _ = stop_tx.send(());

        // The cooperative stop lets the in-flight save finish before the
        // loop exits — the join observes both.
        tokio::time::timeout(Duration::from_secs(2), flush)
            .await
            .expect("flush task must stop after the stop signal")
            .expect("flush task must not panic");

        // Past the in-flight window: with a 20 ms tick, a loop that
        // (wrongly) kept running would have saved again many times over.
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "no second save after stop — a post-stop tick is the file-resurrection path"
        );
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("access-status.json"))
                .expect("in-flight save must have landed"),
            "slow-save-landed",
            "the in-flight save completed and nothing overwrote it afterwards"
        );
    }

    /// A local serve-like endpoint (no relay) and a dialer pointed at it.
    async fn local_dialer() -> (iroh::Endpoint, ServiceDialer) {
        use iroh::endpoint::presets::Minimal;
        let alpn = proto::alpn_for("t");
        let server = iroh::Endpoint::builder(Minimal)
            .relay_mode(iroh::endpoint::RelayMode::Disabled)
            .alpns(vec![alpn.clone()])
            .bind()
            .await
            .unwrap();
        let client = iroh::Endpoint::builder(Minimal)
            .relay_mode(iroh::endpoint::RelayMode::Disabled)
            .bind()
            .await
            .unwrap();
        let mut addr = iroh::EndpointAddr::new(server.id());
        for sock in server.bound_sockets() {
            let ip = if sock.ip().is_unspecified() {
                std::net::Ipv4Addr::LOCALHOST.into()
            } else {
                sock.ip()
            };
            addr = addr.with_ip_addr((ip, sock.port()).into());
        }
        let dialer = ServiceDialer {
            ep: client,
            addr,
            alpn,
            svc_name: "t".to_string(),
            listen_addr: "127.0.0.1:0".to_string(),
            state: Mutex::new(ConnState {
                conn: None,
                tasks: JoinSet::new(),
            }),
            dial: Mutex::new(()),
        };
        // Accept and hold every connection so none closes on its own.
        let accept = server.clone();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Some(incoming) = accept.accept().await {
                if let Ok(conn) = incoming.await {
                    held.push(conn);
                }
            }
        });
        (server, dialer)
    }

    /// A channel holding a stale failed connection must not evict the
    /// healthy connection another channel redialed in the meantime.
    #[tokio::test]
    async fn invalidate_only_evicts_the_failed_connection() {
        let (_server, dialer) = local_dialer().await;
        let stale = dialer.get_or_dial().await.unwrap();
        dialer.invalidate(&stale).await;
        let fresh = dialer.get_or_dial().await.unwrap();
        assert_ne!(stale.stable_id(), fresh.stable_id());

        // A second channel still reporting the stale failure.
        dialer.invalidate(&stale).await;
        let again = dialer.get_or_dial().await.unwrap();
        assert_eq!(
            again.stable_id(),
            fresh.stable_id(),
            "healthy redialed connection must survive a stale invalidate"
        );
    }

    /// The status path must not wait for an in-progress dial, and a
    /// deadline cancelling a waiter must not wedge later dials.
    #[tokio::test]
    async fn path_report_does_not_block_on_a_stuck_dial() {
        let (_server, dialer) = local_dialer().await;
        let _dialing = dialer.dial.lock().await; // simulates a long dial
        let report = tokio::time::timeout(Duration::from_millis(500), dialer.path_report())
            .await
            .expect("path_report must not wait for the dial lock");
        assert!(report.is_none());
    }
}
