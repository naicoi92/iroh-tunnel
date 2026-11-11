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
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::Mutex;
use tokio::task::JoinSet;

use crate::endpoint;
use crate::observation::{AccessObservation, AccessServiceObservation, STATUS_FLUSH_INTERVAL};
use crate::proto;
use crate::runtime::support;
use crate::runtime::{AccessReady, ReadyReport, ReadyServices};
use crate::runtime::{RoleHooks, RunCompletion, RunContext, StatusSink};
use crate::settings::{AccessServiceSettings, AccessSettings, Protocol, UdpLimits};
use crate::shutdown;
use crate::udp::{Incarnation, RedialBackoff};

/// Run the access role on a bound endpoint until `shutdown` resolves.
///
/// Order: emit `on_ready` -> build dialers -> initial status save ->
/// listeners. A failing `on_ready` closes the endpoint and returns its
/// error. The role owns admission, drain, task joins and the endpoint close;
/// the returned [`RunCompletion`] tells the caller whether the status writer
/// quiesced.
pub async fn run_access(
    ep: iroh::Endpoint,
    settings: AccessSettings,
    ctx: RunContext,
    hooks: RoleHooks<AccessObservation>,
    shutdown: impl Future<Output = ()>,
) -> RunCompletion {
    let RoleHooks {
        on_ready,
        status: sink,
    } = hooks;
    let report = ReadyReport {
        node_id: endpoint::node_id_string(&ep),
        home_relay: endpoint::home_relay(&ep).map(|relay| relay.to_string()),
        services: ready_services(&settings),
    };
    if let Err(error) = on_ready(&report) {
        ep.close().await;
        return RunCompletion {
            result: Err(error),
            status_quiesced: true,
        };
    }
    tracing::info!("endpoint ready");
    let cfg = settings;

    // Failures before the flush task spawns leave no writer to wait for.
    let (dialers, status_services) = match build_dialers(&ep, &cfg) {
        Ok(built) => built,
        Err(error) => {
            return RunCompletion {
                result: Err(error),
                status_quiesced: true,
            }
        }
    };
    let mut listeners = JoinSet::new();
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
        node_id: endpoint::node_id_string(&ep),
        pid: std::process::id(),
        started_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        services: status_services,
    };
    let initial = render_access_status(&status, &dialers).await;
    let mut sink = sink;
    let seeded = match sink.as_mut() {
        Some(sink) => match sink.save(initial.clone()).await {
            Ok(()) => Some(initial),
            Err(e) => {
                tracing::warn!("failed to write status file: {e}");
                None
            }
        },
        None => None,
    };

    // Cooperative stop channel for the flush task.
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();

    tracing::info!("access endpoint ready, listening for local clients");
    let flush = sink.map(|sink| {
        tokio::spawn(access_status_flush_loop(
            status,
            dialers.clone(),
            sink,
            seeded,
            stop_rx,
            STATUS_FLUSH_INTERVAL,
        ))
    });
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
    shutdown::begin(&stop, &ctx);
    // Listeners seal admission and join their own channels against the
    // shared deadline. Never abort a supervisor with live descendants.
    shutdown::join(&mut listeners).await;
    for dialer in &dialers {
        dialer.stop_auxiliary_tasks().await;
    }
    // Stop the flush task and bound its join; the run owner removes the
    // status file only when this reports the writer quiesced.
    let status_quiesced = match flush {
        Some(flush) => support::stop_flush(stop_tx, flush).await,
        None => true,
    };
    // Test-utils seam: simulate a shutdown stuck in the endpoint close.
    crate::testing::hang_point().await;
    ep.close().await;
    RunCompletion {
        result: Ok(()),
        status_quiesced,
    }
}

fn ready_services(cfg: &AccessSettings) -> ReadyServices {
    if cfg.services.is_empty() {
        tracing::warn!("no services configured; nothing to expose");
        return ReadyServices::Access(Vec::new());
    }
    // Resolve and parse everything up front so a bad config fails loudly
    // before we bind any ports. The results feed both the report and the
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
    let services = cfg
        .services
        .iter()
        .map(|svc| {
            // This is infallible: parse errors surface later when
            // listen_loop tries to dial; reporting the raw node_id
            // preserves the operator's intent so they can see what was
            // configured.
            let peer_display = match svc.node_id.parse::<iroh::EndpointId>() {
                Ok(id) => id.to_string(),
                Err(_) => svc.node_id.clone(),
            };
            AccessReady {
                name: svc.name.clone(),
                listen_addr: format!("{}:{}", svc.host, svc.port),
                peer_display,
                protocol: svc.protocol,
                relay_override_count: svc.relay_urls.len(),
            }
        })
        .collect();
    ReadyServices::Access(services)
}

/// Resolve relays and parse every service's `node_id` before any listener
/// spawns, so a bad config fails fast. Each service resolves its own
/// effective relay list — its override when set, else the node's — so
/// dialers carry only their service's URLs (issue #54). One dialer per
/// service is kept alive alongside the listener tasks: the status flush task
/// reads each service's live connection path from its dialer (issue #59).
fn build_dialers(
    ep: &iroh::Endpoint,
    cfg: &AccessSettings,
) -> Result<(Vec<Arc<ServiceDialer>>, Vec<AccessStatusServiceRow>)> {
    let node_relay_urls = endpoint::resolve_relay_urls(&cfg.node.relay_urls)?;
    if cfg.node.relay_urls.is_empty() {
        tracing::info!(
            count = node_relay_urls.len(),
            "no relay_urls configured, falling back to n0 default relays"
        );
    }
    let mut dialers: Vec<Arc<ServiceDialer>> = Vec::new();
    let mut status_services: Vec<AccessStatusServiceRow> = Vec::new();
    for svc in &cfg.services {
        let node_id = svc
            .node_id
            .parse::<iroh::EndpointId>()
            .with_context(|| format!("invalid node_id: {}", svc.node_id))?;
        // Raw `host:port` is what TcpListener::bind consumes; the status row
        // renders the bracketed-IPv6 form via format_local_addr — the same
        // normalization as the serve schema's `local_addr`.
        let bind_addr = format!("{}:{}", svc.host, svc.port);
        let effective: Vec<iroh::RelayUrl> = if svc.relay_urls.is_empty() {
            node_relay_urls.clone()
        } else {
            endpoint::resolve_relay_urls(&svc.relay_urls)
                .with_context(|| format!("service '{}': invalid relay_urls", svc.name))?
        };
        let dialer = Arc::new(ServiceDialer::new(ep, node_id, svc, &effective, bind_addr));
        status_services.push(AccessStatusServiceRow {
            name: svc.name.clone(),
            listen_addr: support::format_local_addr(&svc.host, svc.port),
            // The parsed id, not the raw config string — the status row is
            // normalized exactly like every other rendered id.
            peer: node_id.to_string(),
        });
        dialers.push(dialer);
    }
    Ok((dialers, status_services))
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
) -> AccessObservation {
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
        services.push(AccessServiceObservation {
            name: row.name.clone(),
            listen_addr: row.listen_addr.clone(),
            peer: row.peer.clone(),
            transports,
            local_bound_addrs,
            udp: (dialer.protocol == Protocol::Udp).then(|| dialer.udp_stats.snapshot()),
        });
    }
    AccessObservation {
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
/// `interval` is a seam for the unit test that drives a slow save against
/// the stop signal; production passes [`STATUS_FLUSH_INTERVAL`]. `sink` is
/// the host's publisher.
async fn access_status_flush_loop(
    status: AccessStatusTemplate,
    dialers: Vec<Arc<ServiceDialer>>,
    mut sink: StatusSink<AccessObservation>,
    mut last: Option<AccessObservation>,
    mut stop: tokio::sync::oneshot::Receiver<()>,
    interval: Duration,
) {
    loop {
        // `biased` so a stop that arrives during a save is observed before
        // the next sleep even starts — no extra tick, no extra save.
        tokio::select! {
            biased;
            _ = &mut stop => break,
            _ = tokio::time::sleep(interval) => {}
        }
        let observation = render_access_status(&status, &dialers).await;
        if last.as_ref() == Some(&observation) {
            continue;
        }
        // Only record the observation as written on success, so a failed
        // write is retried on the next tick rather than silently dropped
        // until the next change.
        match sink.save(observation.clone()).await {
            Ok(()) => last = Some(observation),
            Err(e) => tracing::warn!("failed to write status file: {e}"),
        }
    }
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
    protocol: Protocol,
    udp_limits: UdpLimits,
    udp_stats: Arc<crate::udp::UdpStats>,
    listen_addr: String,
    /// The shared multiplexed connection and its poller, behind ONE lock:
    /// the one-connection-and-one-poller-per-service rule is structural —
    /// a redial replaces both atomically under the same guard. Lock order is
    /// `dial` -> `state`, and `state` is never held across a dial, backoff
    /// or endpoint query. Concurrent channels serialize the (re)dial on
    /// `dial` instead of racing N dials.
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

impl ConnState {
    /// Whether `conn` is the cached connection (not a replaced or evicted one).
    fn is_current(&self, conn: &iroh::endpoint::Connection) -> bool {
        self.conn
            .as_ref()
            .is_some_and(|cached| cached.stable_id() == conn.stable_id())
    }
}

impl ServiceDialer {
    fn new(
        ep: &iroh::Endpoint,
        node_id: iroh::EndpointId,
        svc: &AccessServiceSettings,
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
            protocol: svc.protocol,
            udp_limits: svc.udp,
            udp_stats: Arc::default(),
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
    /// `deadline` bounds the dial and backoff; the caller bounds the lock wait.
    ///
    /// The connection is stored in the same synchronous step that follows a
    /// successful dial, so a caller deadline cannot cancel the future in
    /// between and discard a dial that already succeeded.
    async fn get_or_dial(
        &self,
        deadline: tokio::time::Instant,
    ) -> Result<iroh::endpoint::Connection> {
        let _dialing = self.dial.lock().await;
        if let Some(conn) = self.state.lock().await.conn.as_ref() {
            return Ok(conn.clone());
        }
        let conn = support::connect_with_retry(&self.ep, &self.addr, &self.alpn, deadline).await?;
        let remote_id = conn.remote_id();
        {
            let mut state = self.state.lock().await;
            // Aborted tasks of a replaced connection are only reaped here;
            // the set stays owned by the state until `stop_auxiliary_tasks`.
            state.tasks.abort_all();
            while state.tasks.try_join_next().is_some() {}
            state.tasks.spawn(support::disconnect_watcher(
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
            if state.is_current(&conn) {
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
        if state.is_current(failed) {
            state.conn = None;
        }
    }

    /// Identity of the cached connection, for tests that observe eviction.
    #[cfg(test)]
    async fn cached_conn_id(&self) -> Option<usize> {
        self.state
            .lock()
            .await
            .conn
            .as_ref()
            .map(|conn| conn.stable_id())
    }

    /// Fresh connection-path report for the cached multiplexed connection,
    /// or `None` while the service has no live one (issue #59).
    ///
    /// The guard is dropped BEFORE the endpoint query so a status render
    /// never holds channels off their dial path, and the report is queried
    /// fresh on every call — never cached — so the status file can't serve
    /// a stale path after a migration.
    async fn path_report(&self) -> Option<crate::observation::PeerPathObservation> {
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
    if dialer.protocol == Protocol::Udp {
        return udp_listen_loop(dialer, stop).await;
    }
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

/// Serve UDP clients of one service over QUIC DATAGRAMs (#88).
///
/// Binds the service's listen address and runs [`udp_forward_loop`] on it
/// with the production redial back-off.
/// Contract: `docs/udp-integration-contract.md`.
async fn udp_listen_loop(dialer: Arc<ServiceDialer>, stop: shutdown::Stop) {
    let listen_addr = dialer.listen_addr.clone();
    let socket = match UdpSocket::bind(&listen_addr).await {
        Ok(socket) => Arc::new(socket),
        Err(error) => {
            tracing::error!("failed to bind udp {listen_addr}: {error}");
            return;
        }
    };
    tracing::info!(%listen_addr, "listening for local udp clients");
    udp_forward_loop(UdpForwardCtx::new(dialer), socket, stop).await;
}

/// Serializes dial admission with failure recording and eviction for one
/// UDP service: the forwarding loop holds it from the back-off check through
/// `get_or_dial` and the setup check, a reader from `record` through
/// `invalidate`.
type UdpDialGate = Arc<tokio::sync::Mutex<()>>;

/// Shared state of one UDP service's forwarding loop and its reply readers.
#[derive(Clone)]
struct UdpForwardCtx {
    dialer: Arc<ServiceDialer>,
    /// Never held across an `.await`.
    backoff: Arc<std::sync::Mutex<RedialBackoff>>,
    gate: UdpDialGate,
    #[cfg(test)]
    hooks: Option<Arc<UdpTestHooks>>,
}

/// Sequence counters tests wait on to order the loop against a reader.
#[cfg(test)]
#[derive(Default)]
struct UdpTestHooks {
    /// Bumped right before the loop awaits the dial gate.
    gate_waiting: tokio::sync::watch::Sender<u64>,
    /// Bumped right after a clear back-off check, before `get_or_dial`.
    admitted: tokio::sync::watch::Sender<u64>,
    /// Bumped when a reader matched the dedicated close, before the gate.
    reader_failed: tokio::sync::watch::Sender<u64>,
    /// Bumped when the back-off check dropped a datagram: proof the loop
    /// processed it (and did not dial) rather than merely received it.
    waiting_dropped: tokio::sync::watch::Sender<u64>,
    /// Number of live `relay_peer_replies` readers (up on start, down on drop,
    /// including when the reader is aborted).
    readers_alive: tokio::sync::watch::Sender<u64>,
}

/// Counts a reader in `UdpTestHooks::readers_alive` for as long as it lives.
#[cfg(test)]
struct ReaderAlive(Option<Arc<UdpTestHooks>>);

#[cfg(test)]
impl ReaderAlive {
    fn new(hooks: Option<Arc<UdpTestHooks>>) -> Self {
        if let Some(hooks) = &hooks {
            hooks.readers_alive.send_modify(|n| *n += 1);
        }
        Self(hooks)
    }
}

#[cfg(test)]
impl Drop for ReaderAlive {
    fn drop(&mut self) {
        if let Some(hooks) = &self.0 {
            hooks.readers_alive.send_modify(|n| *n -= 1);
        }
    }
}

impl UdpForwardCtx {
    fn new(dialer: Arc<ServiceDialer>) -> Self {
        Self {
            dialer,
            backoff: Arc::default(),
            gate: UdpDialGate::default(),
            #[cfg(test)]
            hooks: None,
        }
    }
}

/// The adopted session incarnation of one UDP service: its flows, its reply
/// readers and the connection they belong to.
struct UdpSession {
    flows: Arc<std::sync::Mutex<crate::udp::ClientFlows>>,
    readers: JoinSet<()>,
    /// Holding the adopted connection keeps its allocation alive, so its
    /// `stable_id()` cannot be reused by a later connection while compared.
    current: Option<(iroh::endpoint::Connection, Incarnation)>,
}

/// Forward datagrams from local clients on `socket` to the service's Serve
/// peer until `stop`.
///
/// Each client address is a flow (UDP-04). All flows share the service's one
/// connection; when that connection is replaced the flows end with it
/// (session incarnation). Idle flows expire after the service's idle timeout.
/// New clients are refused at `max_flows`, and spent flow identifiers end the
/// incarnation by evicting the connection.
async fn udp_forward_loop(ctx: UdpForwardCtx, socket: Arc<UdpSocket>, mut stop: shutdown::Stop) {
    let idle = ctx.dialer.udp_limits.idle_timeout;
    let listen_addr = ctx.dialer.listen_addr.clone();
    let mut session = UdpSession {
        flows: Arc::default(),
        readers: JoinSet::new(),
        current: None,
    };
    let mut buf = vec![0u8; 65535];
    let mut sweep = tokio::time::interval(crate::udp::sweep_interval(idle));
    sweep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        let (n, client) = tokio::select! {
            biased;
            _ = shutdown::stopped(&mut stop) => break,
            _ = sweep.tick() => {
                session.expire_idle(&ctx);
                continue;
            }
            Some(result) = session.readers.join_next(), if !session.readers.is_empty() => {
                shutdown::observe(result);
                continue;
            }
            result = socket.recv_from(&mut buf) => match result {
                Ok(received) => received,
                // Windows reports a client's ICMP port-unreachable here after a
                // reply to a client that has gone away. Nothing is wrong.
                Err(error) if crate::udp::is_transient_recv_error(&error) => {
                    tracing::debug!("udp recv on {listen_addr} reported a lost datagram: {error}");
                    continue;
                }
                Err(error) => {
                    tracing::warn!("udp recv error on {listen_addr}: {error}");
                    // A persistent error must not spin the loop.
                    tokio::select! {
                        _ = shutdown::stopped(&mut stop) => break,
                        _ = tokio::time::sleep(Duration::from_millis(100)) => {}
                    }
                    continue;
                }
            }
        };
        let deadline = tokio::time::Instant::now() + CLIENT_CONNECT_DEADLINE;
        // Gate wait, dialing and the setup check can take up to the client
        // deadline; shutdown must not wait for any of them.
        let admitted = tokio::select! {
            biased;
            _ = shutdown::stopped(&mut stop) => break,
            admitted = session.admit(&ctx, &socket, deadline) => admitted,
        };
        let Some((conn, incarnation)) = admitted else {
            continue;
        };
        session
            .forward(&ctx, &conn, incarnation, client, &buf[..n])
            .await;
    }
    session.readers.abort_all();
    shutdown::join(&mut session.readers).await;
}

impl UdpSession {
    fn expire_idle(&self, ctx: &UdpForwardCtx) {
        let mut flows = self.flows.lock().unwrap_or_else(|p| p.into_inner());
        let ended = flows.expire(
            std::time::Instant::now(),
            ctx.dialer.udp_limits.idle_timeout,
        );
        ctx.dialer.udp_stats.set_active_flows(flows.len());
        if ended > 0 {
            tracing::debug!(svc = %ctx.dialer.svc_name, ended, "udp flows expired");
        }
    }

    /// The connection a client datagram goes out on, or `None` to drop it.
    ///
    /// Holds the dial gate from the back-off check through the setup check,
    /// so a reader cannot record a failure between them; it is released
    /// before the datagram is sent.
    async fn admit(
        &mut self,
        ctx: &UdpForwardCtx,
        socket: &Arc<UdpSocket>,
        deadline: tokio::time::Instant,
    ) -> Option<(iroh::endpoint::Connection, Incarnation)> {
        #[cfg(test)]
        ctx.bump_hook(|hooks| &hooks.gate_waiting);
        let _gate = ctx.gate.lock().await;
        if !ctx.dial_allowed() {
            #[cfg(test)]
            ctx.bump_hook(|hooks| &hooks.waiting_dropped);
            return None;
        }
        #[cfg(test)]
        ctx.bump_hook(|hooks| &hooks.admitted);
        let conn = match ctx.dialer.get_or_dial(deadline).await {
            Ok(conn) => conn,
            Err(error) => {
                tracing::warn!(svc = %ctx.dialer.svc_name, "udp datagram dropped, no connection: {error}");
                return None;
            }
        };
        self.adopt(ctx, socket, conn).await
    }

    /// Keep the current incarnation, or start a new one for a newly dialed
    /// connection.
    async fn adopt(
        &mut self,
        ctx: &UdpForwardCtx,
        socket: &Arc<UdpSocket>,
        conn: iroh::endpoint::Connection,
    ) -> Option<(iroh::endpoint::Connection, Incarnation)> {
        if let Some((current, incarnation)) = &self.current {
            if current.stable_id() == conn.stable_id() {
                return Some((conn, *incarnation));
            }
        }
        self.retire().await;
        ctx.dialer.udp_stats.set_active_flows(0);
        // Begun after retiring, so a late record from an old reader is stale.
        let incarnation = ctx.lock_backoff().begin();
        if !crate::udp::carries_datagrams(conn.max_datagram_size()) {
            reject_unusable(ctx, &conn, incarnation).await;
            return None;
        }
        self.readers.spawn(relay_peer_replies(
            conn.clone(),
            incarnation,
            socket.clone(),
            self.flows.clone(),
            ctx.clone(),
        ));
        self.current = Some((conn.clone(), incarnation));
        Some((conn, incarnation))
    }

    /// End the current incarnation: no flow or reader survives it.
    async fn retire(&mut self) {
        self.readers.abort_all();
        while self.readers.join_next().await.is_some() {}
        *self.flows.lock().unwrap_or_else(|p| p.into_inner()) = crate::udp::ClientFlows::default();
        self.current = None;
    }

    /// Send one client payload on `conn`, applying the datagram limit and
    /// the flow cap.
    async fn forward(
        &mut self,
        ctx: &UdpForwardCtx,
        conn: &iroh::endpoint::Connection,
        incarnation: Incarnation,
        client: std::net::SocketAddr,
        payload: &[u8],
    ) {
        use crate::udp::{send_decision, DropReason, SendDecision};
        let dialer = &ctx.dialer;
        let stats = &dialer.udp_stats;
        let max = conn.max_datagram_size();
        let limit = crate::udp::payload_limit(max);
        stats.set_datagram_limit(limit);
        let n = payload.len();
        match send_decision(max, n) {
            SendDecision::Send => {}
            SendDecision::Drop(DropReason::LimitShrink) => {
                stats.record_drop(DropReason::LimitShrink);
                tracing::warn!(svc = %dialer.svc_name, "datagrams unusable on this connection, dropping");
                return;
            }
            SendDecision::Drop(reason) => {
                // Only an oversize decision reaches here, which needs a usable limit.
                stats.record_drop(reason);
                tracing::warn!(
                    svc = %dialer.svc_name,
                    size = n,
                    limit = limit.unwrap_or_default(),
                    "dropping oversize udp datagram"
                );
                return;
            }
        }
        let flow = self.flows.lock().unwrap_or_else(|p| p.into_inner()).touch(
            client,
            std::time::Instant::now(),
            dialer.udp_limits.max_flows,
        );
        let flow = match flow {
            Ok(flow) => flow,
            Err(crate::udp::Refused::Cap) => {
                // Over the Access cap: drop before it costs Serve anything.
                stats.record_drop(DropReason::Cap);
                tracing::debug!(svc = %dialer.svc_name, %client, "udp flow refused: access flow cap reached");
                return;
            }
            Err(crate::udp::Refused::Exhausted) => {
                // Identifiers are never reused within an incarnation, so the
                // only way on is a new one: drop the connection, and the next
                // datagram dials afresh with a clean flow table (contract).
                stats.record_drop(DropReason::Cap);
                tracing::warn!(
                    svc = %dialer.svc_name,
                    "udp flow identifiers exhausted, ending the session incarnation"
                );
                dialer.invalidate(conn).await;
                return;
            }
        };
        stats.set_active_flows(self.flows.lock().unwrap_or_else(|p| p.into_inner()).len());
        if let Err(error) = conn.send_datagram(crate::udp::encode(flow, payload)) {
            tracing::debug!(svc = %dialer.svc_name, "udp datagram not sent: {error}");
            // A lost connection never recovers. Evict it so the next datagram
            // dials a fresh one and starts a new incarnation (UDP-04); the
            // cached dead connection would otherwise swallow traffic forever.
            if matches!(error, iroh::endpoint::SendDatagramError::ConnectionLost(_)) {
                // The reader logs this close; here it only races to record
                // it, and recording the same incarnation twice is a no-op.
                if conn
                    .close_reason()
                    .is_some_and(|reason| crate::udp::closed_for_unsupported_datagrams(&reason))
                {
                    ctx.lock_backoff()
                        .record(incarnation, std::time::Instant::now());
                }
                dialer.invalidate(conn).await;
            }
        }
    }
}

/// Close a newly dialed connection that cannot carry datagrams and start the
/// redial back-off for it.
async fn reject_unusable(
    ctx: &UdpForwardCtx,
    conn: &iroh::endpoint::Connection,
    incarnation: Incarnation,
) {
    tracing::error!(
        peer = %conn.remote_id(),
        svc = %ctx.dialer.svc_name,
        "udp datagrams unsupported by serve peer, closing the connection"
    );
    conn.close(
        iroh::endpoint::VarInt::from_u32(crate::udp::DATAGRAMS_UNSUPPORTED_CODE),
        crate::udp::DATAGRAMS_UNSUPPORTED_REASON,
    );
    ctx.lock_backoff()
        .record(incarnation, std::time::Instant::now());
    ctx.dialer.invalidate(conn).await;
}

impl UdpForwardCtx {
    fn lock_backoff(&self) -> std::sync::MutexGuard<'_, RedialBackoff> {
        self.backoff.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Whether the back-off lets this datagram dial; logs the drop if not.
    /// Drops during the back-off record no drop counter.
    fn dial_allowed(&self) -> bool {
        let check = self.lock_backoff().check(std::time::Instant::now());
        let svc = &self.dialer.svc_name;
        match check {
            crate::udp::BackoffCheck::Clear => true,
            crate::udp::BackoffCheck::Waiting {
                first: true,
                resumes_in,
            } => {
                tracing::warn!(
                    svc = %svc,
                    "udp datagram dropped: serve peer cannot carry datagrams, dialing resumes in {}s",
                    resumes_in.as_secs()
                );
                false
            }
            crate::udp::BackoffCheck::Waiting { resumes_in, .. } => {
                tracing::debug!(
                    svc = %svc,
                    resumes_in_ms = resumes_in.as_millis(),
                    "udp datagram dropped during the redial back-off"
                );
                false
            }
        }
    }

    /// Record the failure of `conn`'s incarnation and evict it, under the
    /// dial gate so no datagram is admitted between the two.
    async fn fail_incarnation(&self, conn: &iroh::endpoint::Connection, incarnation: Incarnation) {
        let _gate = self.gate.lock().await;
        self.lock_backoff()
            .record(incarnation, std::time::Instant::now());
        self.dialer.invalidate(conn).await;
    }

    /// Advance a test hook counter; compiled out of production builds.
    #[cfg(test)]
    fn bump_hook(&self, counter: impl FnOnce(&UdpTestHooks) -> &tokio::sync::watch::Sender<u64>) {
        if let Some(hooks) = &self.hooks {
            counter(hooks).send_modify(|n| *n += 1);
        }
    }
}

/// Deliver each datagram from the peer to the client of its flow.
async fn relay_peer_replies(
    conn: iroh::endpoint::Connection,
    incarnation: Incarnation,
    socket: Arc<UdpSocket>,
    flows: Arc<std::sync::Mutex<crate::udp::ClientFlows>>,
    ctx: UdpForwardCtx,
) {
    use crate::udp::DropReason;
    #[cfg(test)]
    let _alive = ReaderAlive::new(ctx.hooks.clone());
    let stats = &ctx.dialer.udp_stats;
    loop {
        let datagram = match conn.read_datagram().await {
            Ok(datagram) => datagram,
            Err(error) => {
                if crate::udp::closed_for_unsupported_datagrams(&error) {
                    tracing::error!(
                        peer = %conn.remote_id(),
                        svc = %ctx.dialer.svc_name,
                        "serve peer closed the udp connection: datagrams unsupported"
                    );
                    #[cfg(test)]
                    ctx.bump_hook(|hooks| &hooks.reader_failed);
                    ctx.fail_incarnation(&conn, incarnation).await;
                }
                return;
            }
        };
        let Ok((flow, payload)) = crate::udp::decode(&datagram) else {
            stats.record_drop(DropReason::Malformed);
            tracing::debug!("dropping malformed udp datagram from peer");
            continue;
        };
        let client = flows
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .client_of(flow, std::time::Instant::now());
        match client {
            Some(client) => {
                if let Err(error) = socket.send_to(&payload, client).await {
                    tracing::debug!(flow = flow.0, %client, "udp reply to client failed: {error}");
                }
            }
            None => {
                stats.record_drop(DropReason::UnknownFlow);
                tracing::debug!(flow = flow.0, "dropping reply for unknown udp flow");
            }
        }
    }
}

// A single budget includes mutex wait, dial, backoff and opening the stream.
// Keep established streams outside this timeout.
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
        result = tokio::time::timeout_at(deadline, open_client_stream(dialer, deadline)) => match result {
            Ok(Ok(streams)) => streams,
            // The dial loop stops at the same deadline; report one message.
            Ok(Err(_)) if tokio::time::Instant::now() >= deadline => {
                anyhow::bail!("client connection deadline expired")
            }
            Ok(Err(error)) => return Err(error),
            Err(_) => anyhow::bail!("client connection deadline expired"),
        }
    };
    // QUIC shutdown queues FIN; pipe completion alone does not prove final
    // request bytes reached Serve. Keep this task owned until acknowledged.
    let delivered = streams.0.stopped();
    crate::tcp::pipe_tcp_bidirectional(local, (streams.1, streams.0)).await?;
    // Bounded: a peer that never acknowledges must not pin this task (and its
    // status/drain accounting) forever. A missing acknowledgement is NOT
    // success: it may hide a lost delivery, so it is reported as peer loss
    // (AC-TCP-06). Nothing is replayed.
    match crate::tcp::await_delivery(delivered, crate::tcp::DELIVERY_ACK_GRACE).await {
        crate::tcp::Delivery::Acknowledged => Ok(()),
        crate::tcp::Delivery::PeerStopped(code) => {
            anyhow::bail!("peer stopped client stream: {code}")
        }
        crate::tcp::Delivery::Unconfirmed => {
            tracing::warn!(svc = %dialer.svc_name, "peer lost: no delivery acknowledgement within grace");
            anyhow::bail!("peer lost: delivery unconfirmed (no acknowledgement within grace)")
        }
        crate::tcp::Delivery::Lost(error) => {
            Err(anyhow::Error::new(error).context("client stream delivery failed"))
        }
    }
}

async fn open_client_stream(
    dialer: &ServiceDialer,
    deadline: tokio::time::Instant,
) -> Result<(iroh::endpoint::SendStream, iroh::endpoint::RecvStream)> {
    // Multiplexed: one stream on the shared connection per channel. If
    // `open_bi` fails with a connection-level error, the cache is dropped and
    // exactly one redial + retry is attempted before giving up — a dead
    // connection surfaces as EOF-style errors on the active channels and the
    // next channel gets a fresh one.
    let conn = dialer.get_or_dial(deadline).await?;
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
            let conn = dialer.get_or_dial(deadline).await?;
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
/// Interval between connection-path polls of the Access role.
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
fn render_established_paths(report: Option<&crate::observation::PeerPathObservation>) -> String {
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
    report: Option<&crate::observation::PeerPathObservation>,
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
    initial: Vec<crate::observation::TransportObservation>,
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
        };
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        // Signalled the moment the slow save STARTS, so the stop below is
        // provably sent while that save is in-flight.
        let (started_tx, mut started_rx) = tokio::sync::mpsc::channel::<()>(1);
        let calls = Arc::new(AtomicUsize::new(0));

        let calls_for_save = calls.clone();
        let started_for_save = started_tx.clone();
        let dir = tmp.path().to_path_buf();
        let sink = StatusSink::new(move |_observation| {
            let calls = calls_for_save.clone();
            let started = started_for_save.clone();
            let dir = dir.clone();
            Box::pin(async move {
                let n = calls.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    let _ = started.try_send(());
                    // The deliberately slow "blocking" save: its write
                    // lands AFTER the stop signal below is sent.
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    std::fs::write(dir.join("access-status.json"), b"slow-save-landed")
                        .expect("spy save write");
                }
                Err(anyhow::anyhow!("spy save always fails"))
            })
        });
        let flush = tokio::spawn(access_status_flush_loop(
            template,
            Vec::new(),
            sink,
            None, // unseeded `last`: the first tick always saves
            stop_rx,
            Duration::from_millis(20),
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

    fn far_deadline() -> tokio::time::Instant {
        tokio::time::Instant::now() + Duration::from_secs(30)
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
            protocol: Protocol::Tcp,
            udp_limits: UdpLimits::default(),
            udp_stats: Arc::default(),
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
        let stale = dialer.get_or_dial(far_deadline()).await.unwrap();
        dialer.invalidate(&stale).await;
        let fresh = dialer.get_or_dial(far_deadline()).await.unwrap();
        assert_ne!(stale.stable_id(), fresh.stable_id());

        // A second channel still reporting the stale failure.
        dialer.invalidate(&stale).await;
        let again = dialer.get_or_dial(far_deadline()).await.unwrap();
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

    // -----------------------------------------------------------------------
    // UDP forwarding orchestration (#152): setup check, close report, back-off
    // -----------------------------------------------------------------------

    use iroh::endpoint::{Connection, ConnectionError, VarInt};

    const TEST_WINDOW: Duration = Duration::from_millis(1500);
    const POLL_STEP: Duration = Duration::from_millis(10);
    const POLL_LIMIT: Duration = Duration::from_secs(5);
    const SEND_STEP: Duration = Duration::from_millis(50);
    /// Sending stops this long before expiry so the loop can process every
    /// datagram on a loaded machine (RS-14: a wide, explicit margin).
    const PROCESS_MARGIN: Duration = Duration::from_millis(350);
    /// Window left when it is captured; leaves room for a >= 200 ms
    /// observation plus the processing margin.
    const MIN_WINDOW_LEFT: Duration = Duration::from_millis(600);
    /// The shortest negative observation that counts as suppression coverage.
    const MIN_OBSERVED: Duration = Duration::from_millis(200);

    /// Dialable loopback address of a relay-less endpoint.
    fn loopback_addr(ep: &iroh::Endpoint) -> iroh::EndpointAddr {
        let mut addr = iroh::EndpointAddr::new(ep.id());
        for sock in ep.bound_sockets() {
            let ip = if sock.ip().is_unspecified() {
                std::net::Ipv4Addr::LOCALHOST.into()
            } else {
                sock.ip()
            };
            addr = addr.with_ip_addr((ip, sock.port()).into());
        }
        addr
    }

    /// A loopback Serve-like peer for the service `t` that hands every
    /// accepted connection to the test and echoes its datagrams.
    struct TestPeer {
        ep: iroh::Endpoint,
        accepted: tokio::sync::mpsc::UnboundedReceiver<Connection>,
        stop: tokio::sync::oneshot::Sender<()>,
        task: OwnedTask,
        children: ChildrenGuard,
    }

    /// A task owned by a fixture: aborted on drop so a panicking test cannot
    /// leak it; `shutdown` takes it back to join.
    struct OwnedTask(Option<tokio::task::JoinHandle<()>>);

    impl OwnedTask {
        fn take(&mut self) -> tokio::task::JoinHandle<()> {
            self.0.take().expect("task already taken")
        }
    }

    impl Drop for OwnedTask {
        fn drop(&mut self) {
            if let Some(task) = &self.0 {
                task.abort();
            }
        }
    }

    /// Aborts every registered child on drop (panic path); the normal path
    /// drains them through `drain_children`.
    struct ChildrenGuard(PeerChildren);

    impl Drop for ChildrenGuard {
        fn drop(&mut self) {
            for handle in self.0.lock().unwrap_or_else(|p| p.into_inner()).iter() {
                handle.abort();
            }
        }
    }

    /// Per-connection child tasks, shared so a fallback can still drain them
    /// when the supervisor had to be aborted.
    type PeerChildren = Arc<std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>>;

    /// Abort every child task, then join them one at a time. Handles are
    /// popped before they are awaited: the remaining handles stay registered
    /// while the current one is joined, so an interrupted drain leaves them
    /// for the next caller. Only the harness (never the
    /// supervisor) drains, so the supervisor's cancellation cannot lose it.
    async fn drain_children(children: &PeerChildren) {
        for handle in children.lock().unwrap_or_else(|p| p.into_inner()).iter() {
            handle.abort();
        }
        loop {
            let next = children.lock().unwrap_or_else(|p| p.into_inner()).pop();
            let Some(handle) = next else { break };
            let _ = handle.await;
        }
    }

    /// Join `task` within `bound`; on expiry abort and await it, then
    /// report the failure so the caller still finishes its cleanup.
    async fn join_or_abort(
        mut task: tokio::task::JoinHandle<()>,
        bound: Duration,
        what: &str,
        failures: &mut Vec<String>,
    ) {
        match tokio::time::timeout(bound, &mut task).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => failures.push(format!("{what} failed: {error}")),
            Err(_) => {
                task.abort();
                let _ = task.await;
                failures.push(format!("{what} did not stop within {bound:?} (aborted)"));
            }
        }
    }

    /// Await `fut` within `bound`, recording a failure instead of panicking.
    async fn finish_within(
        bound: Duration,
        what: &str,
        failures: &mut Vec<String>,
        fut: impl Future<Output = ()>,
    ) {
        if tokio::time::timeout(bound, fut).await.is_err() {
            failures.push(format!("{what} did not finish within {bound:?}"));
        }
    }

    fn assert_cleanup(failures: Vec<String>) {
        assert!(failures.is_empty(), "cleanup failures: {failures:?}");
    }

    async fn test_peer(datagrams: bool) -> TestPeer {
        use iroh::endpoint::presets::Minimal;
        use iroh::endpoint::QuicTransportConfig;
        let mut builder = iroh::Endpoint::builder(Minimal)
            .relay_mode(iroh::endpoint::RelayMode::Disabled)
            .alpns(vec![proto::alpn_for("t")]);
        if !datagrams {
            builder = builder.transport_config(
                QuicTransportConfig::builder()
                    .datagram_receive_buffer_size(None)
                    .build(),
            );
        }
        let ep = builder.bind().await.expect("bind test peer");
        let (accepted_tx, accepted) = tokio::sync::mpsc::unbounded_channel();
        let (stop, stop_rx) = tokio::sync::oneshot::channel();
        let children = PeerChildren::default();
        let task = tokio::spawn(peer_supervisor(
            ep.clone(),
            accepted_tx,
            stop_rx,
            children.clone(),
        ));
        TestPeer {
            ep,
            accepted,
            stop,
            task: OwnedTask(Some(task)),
            children: ChildrenGuard(children),
        }
    }

    /// Accept until stopped; each connection's handshake, hand-off and echo
    /// run as children registered in `children`, which the harness owner
    /// aborts and joins after the supervisor has stopped.
    async fn peer_supervisor(
        ep: iroh::Endpoint,
        accepted: tokio::sync::mpsc::UnboundedSender<Connection>,
        mut stop: tokio::sync::oneshot::Receiver<()>,
        children: PeerChildren,
    ) {
        loop {
            tokio::select! {
                biased;
                _ = &mut stop => break,
                incoming = ep.accept() => {
                    let Some(incoming) = incoming else { break };
                    let accepted = accepted.clone();
                    let child = tokio::spawn(async move {
                        let Ok(conn) = incoming.await else { return };
                        let _ = accepted.send(conn.clone());
                        while let Ok(datagram) = conn.read_datagram().await {
                            let _ = conn.send_datagram(datagram);
                        }
                    });
                    children.lock().unwrap_or_else(|p| p.into_inner()).push(child);
                }
            }
        }
        // Descendants are aborted and joined by the owner of `children`
        // (`TestPeer::shutdown`), which the supervisor's cancellation cannot
        // reach; the supervisor only stops accepting.
    }

    impl TestPeer {
        async fn next_conn(&mut self, within: Duration, what: &str) -> Connection {
            tokio::time::timeout(within, self.accepted.recv())
                .await
                .unwrap_or_else(|_| panic!("{what}: no connection within {within:?}"))
                .expect("peer supervisor alive")
        }

        fn assert_no_conn(&mut self, what: &str) {
            match self.accepted.try_recv() {
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
                Ok(conn) => panic!("{what}: unexpected connection {}", conn.stable_id()),
                Err(error) => panic!("{what}: peer channel {error:?}"),
            }
        }

        async fn shutdown(mut self) {
            let mut failures = Vec::new();
            let _ = self.stop.send(());
            let task = self.task.take();
            join_or_abort(task, POLL_LIMIT, "peer supervisor", &mut failures).await;
            // The owner drains the children after the supervisor is gone.
            drain_children(&self.children.0).await;
            finish_within(
                POLL_LIMIT,
                "peer endpoint close",
                &mut failures,
                self.ep.close(),
            )
            .await;
            assert_cleanup(failures);
        }
    }

    fn close_unsupported(conn: &Connection) {
        conn.close(
            VarInt::from_u32(crate::udp::DATAGRAMS_UNSUPPORTED_CODE),
            crate::udp::DATAGRAMS_UNSUPPORTED_REASON,
        );
    }

    /// A UDP-service dialer pointed at `peer` (no accept task of its own).
    async fn udp_dialer(peer: &TestPeer) -> Arc<ServiceDialer> {
        use iroh::endpoint::presets::Minimal;
        let client = iroh::Endpoint::builder(Minimal)
            .relay_mode(iroh::endpoint::RelayMode::Disabled)
            .bind()
            .await
            .expect("bind access endpoint");
        Arc::new(ServiceDialer {
            ep: client,
            addr: loopback_addr(&peer.ep),
            alpn: proto::alpn_for("t"),
            svc_name: "t".to_string(),
            protocol: Protocol::Udp,
            udp_limits: UdpLimits::default(),
            udp_stats: Arc::default(),
            listen_addr: "127.0.0.1:0".to_string(),
            state: Mutex::new(ConnState {
                conn: None,
                tasks: JoinSet::new(),
            }),
            dial: Mutex::new(()),
        })
    }

    /// `udp_forward_loop` on a known socket, with a short back-off window
    /// and the test hooks on.
    struct ForwardHarness {
        client: UdpSocket,
        access_addr: std::net::SocketAddr,
        dialer: Arc<ServiceDialer>,
        backoff: Arc<std::sync::Mutex<RedialBackoff>>,
        gate: UdpDialGate,
        hooks: Arc<UdpTestHooks>,
        stop: tokio::sync::watch::Sender<Option<tokio::time::Instant>>,
        task: OwnedTask,
    }

    async fn forward(dialer: Arc<ServiceDialer>, window: Duration) -> ForwardHarness {
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("bind access"));
        let access_addr = socket.local_addr().expect("access addr");
        let client = UdpSocket::bind("127.0.0.1:0").await.expect("bind client");
        let hooks = Arc::new(UdpTestHooks::default());
        let ctx = UdpForwardCtx {
            dialer: dialer.clone(),
            backoff: Arc::new(std::sync::Mutex::new(RedialBackoff::with_window(window))),
            gate: UdpDialGate::default(),
            hooks: Some(hooks.clone()),
        };
        let (backoff, gate) = (ctx.backoff.clone(), ctx.gate.clone());
        let (stop, stop_rx) = shutdown::channel();
        let task = tokio::spawn(udp_forward_loop(ctx, socket, stop_rx));
        ForwardHarness {
            client,
            access_addr,
            dialer,
            backoff,
            gate,
            hooks,
            stop,
            task: OwnedTask(Some(task)),
        }
    }

    impl ForwardHarness {
        async fn send(&self, payload: &[u8]) {
            self.client
                .send_to(payload, self.access_addr)
                .await
                .expect("client send");
        }

        async fn expect_echo(&self, payload: &[u8]) {
            let mut buf = [0u8; 1500];
            let (n, _) = tokio::time::timeout(POLL_LIMIT, self.client.recv_from(&mut buf))
                .await
                .expect("echo within 5 s")
                .expect("client recv");
            assert_eq!(&buf[..n], payload, "echoed payload");
        }

        async fn round_trip(&self, payload: &[u8]) {
            self.send(payload).await;
            self.expect_echo(payload).await;
        }

        fn waiting_until(&self) -> Option<std::time::Instant> {
            self.backoff
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .waiting_until()
        }

        /// The open window's end, asserting enough of it is left to observe.
        async fn window_end(&self) -> std::time::Instant {
            eventually("back-off window opens", || async {
                self.waiting_until().is_some()
            })
            .await;
            let end = self.waiting_until().expect("window open");
            let left = end.saturating_duration_since(std::time::Instant::now());
            assert!(
                left >= MIN_WINDOW_LEFT,
                "timing: only {left:?} of the window left, need {MIN_WINDOW_LEFT:?}"
            );
            end
        }

        /// Observe the window from a baseline: keep sending until just before
        /// `end`, require the loop to have processed (dropped) every datagram
        /// inside the window, and require that no dial was admitted. The
        /// observation must span at least `MIN_OBSERVED`.
        async fn send_through_window(&self, peer: &mut TestPeer, end: std::time::Instant) {
            let started = std::time::Instant::now();
            let budget = end.saturating_duration_since(started);
            assert!(
                budget >= MIN_OBSERVED + PROCESS_MARGIN,
                "timing: only {budget:?} of the window left at the start of the observation"
            );
            let admitted = *self.hooks.admitted.subscribe().borrow();
            let dropped = *self.hooks.waiting_dropped.subscribe().borrow();
            let stop_at = end - PROCESS_MARGIN;
            let mut sent = 0u64;
            while sent == 0 || std::time::Instant::now() < stop_at {
                self.send(b"during-window").await;
                sent += 1;
                tokio::time::sleep(SEND_STEP).await;
            }
            // Every datagram sent must be processed before the window expires.
            let left = end.saturating_duration_since(std::time::Instant::now());
            let processed = tokio::time::timeout(
                left,
                wait_past(
                    &self.hooks.waiting_dropped,
                    dropped + sent - 1,
                    "window datagrams processed",
                ),
            )
            .await;
            let observed = started.elapsed();
            assert!(
                processed.is_ok() && std::time::Instant::now() < end,
                "timing: {sent} datagrams not all processed before expiry (observed {observed:?}, window {budget:?})"
            );
            assert!(
                observed >= MIN_OBSERVED,
                "timing: observed only {observed:?}, need {MIN_OBSERVED:?}"
            );
            assert_eq!(
                *self.hooks.admitted.subscribe().borrow(),
                admitted,
                "a datagram was admitted to dial during the window ({sent} sent, observed {observed:?})"
            );
            peer.assert_no_conn(&format!("no redial during the window ({sent} sent)"));
        }

        /// After `end`, one datagram dials exactly one new connection.
        async fn resume_after(&self, peer: &mut TestPeer, end: std::time::Instant) -> Connection {
            tokio::time::sleep_until(tokio::time::Instant::from_std(end)).await;
            self.send(b"after-window").await;
            let conn = peer.next_conn(POLL_LIMIT, "redial after the window").await;
            tokio::time::sleep(Duration::from_millis(100)).await;
            peer.assert_no_conn("exactly one redial after the window");
            conn
        }

        async fn shutdown(self) {
            self.shutdown_within(POLL_LIMIT).await;
        }

        async fn shutdown_within(mut self, loop_bound: Duration) {
            let mut failures = Vec::new();
            self.stop.send_replace(Some(tokio::time::Instant::now()));
            let task = self.task.take();
            join_or_abort(task, loop_bound, "forward loop", &mut failures).await;
            let dialer = &self.dialer;
            // The loop's reader JoinSet is aborted when the loop ends (or is
            // aborted: dropping a JoinSet aborts its tasks); wait until they
            // are really gone.
            let mut readers = self.hooks.readers_alive.subscribe();
            let gone = tokio::time::timeout(POLL_LIMIT, readers.wait_for(|n| *n == 0)).await;
            if !matches!(gone, Ok(Ok(_))) {
                failures.push("forward readers still alive after the loop ended".into());
            }
            finish_within(
                POLL_LIMIT,
                "dialer auxiliary tasks",
                &mut failures,
                dialer.stop_auxiliary_tasks(),
            )
            .await;
            finish_within(
                POLL_LIMIT,
                "access endpoint close",
                &mut failures,
                dialer.ep.close(),
            )
            .await;
            assert_cleanup(failures);
        }
    }

    /// Poll `condition` every 10 ms for up to 5 s.
    async fn eventually<F, Fut>(what: &str, mut condition: F)
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = bool>,
    {
        let polled = tokio::time::timeout(POLL_LIMIT, async {
            while !condition().await {
                tokio::time::sleep(POLL_STEP).await;
            }
        })
        .await;
        assert!(polled.is_ok(), "{what}: not within {POLL_LIMIT:?}");
    }

    /// Wait until a hook counter passes `base`.
    async fn wait_past(counter: &tokio::sync::watch::Sender<u64>, base: u64, what: &str) {
        let mut rx = counter.subscribe();
        let passed = tokio::time::timeout(POLL_LIMIT, rx.wait_for(|n| *n > base))
            .await
            .map(|seen| seen.map(|_| ()));
        assert!(
            matches!(passed, Ok(Ok(()))),
            "{what}: counter did not pass {base}"
        );
    }

    fn assert_closed_unsupported(reason: ConnectionError) {
        match reason {
            ConnectionError::ApplicationClosed(close) => {
                // Wire-contract literals, independent of the production constants.
                assert_eq!(close.error_code, VarInt::from_u32(0x1d47), "close code");
                assert_eq!(
                    close.reason.as_ref(),
                    b"udp datagrams unsupported",
                    "close reason"
                );
            }
            other => panic!("expected the dedicated application close, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn adoption_check_closes_and_backs_off() {
        let mut peer = test_peer(false).await;
        let h = forward(udp_dialer(&peer).await, TEST_WINDOW).await;
        h.send(b"first").await;
        let a = peer.next_conn(POLL_LIMIT, "first dial").await;
        let reason = tokio::time::timeout(POLL_LIMIT, a.closed())
            .await
            .expect("access closes the unusable connection");
        assert_closed_unsupported(reason);

        let end = h.window_end().await;
        h.send_through_window(&mut peer, end).await;
        assert_eq!(
            h.dialer.udp_stats.snapshot().drops,
            crate::observation::UdpDropCounts::default(),
            "neither the setup failure nor drops during the window are counted"
        );
        h.resume_after(&mut peer, end).await;

        h.shutdown().await;
        peer.shutdown().await;
    }

    #[tokio::test]
    async fn reader_close_evicts_and_resumes_after_one_window() {
        let mut peer = test_peer(true).await;
        let h = forward(udp_dialer(&peer).await, TEST_WINDOW).await;
        h.round_trip(b"a").await;
        let a = peer.next_conn(POLL_LIMIT, "first dial").await;

        close_unsupported(&a);
        // No client traffic since the close: only the reader can record and evict.
        eventually("reader records and evicts", || async {
            h.waiting_until().is_some() && h.dialer.cached_conn_id().await.is_none()
        })
        .await;
        let end = h.window_end().await;
        h.send_through_window(&mut peer, end).await;
        let b = h.resume_after(&mut peer, end).await;
        h.expect_echo(b"after-window").await;

        h.shutdown().await;
        drop(b);
        peer.shutdown().await;
    }

    #[tokio::test]
    async fn stale_incarnation_close_does_not_start_a_window() {
        let mut peer = test_peer(true).await;
        let h = forward(udp_dialer(&peer).await, TEST_WINDOW).await;
        h.round_trip(b"a").await;
        let a = peer.next_conn(POLL_LIMIT, "first dial").await;
        let cached = h
            .dialer
            .get_or_dial(far_deadline())
            .await
            .expect("cached connection");
        h.dialer.invalidate(&cached).await;
        h.round_trip(b"b").await;
        let _b = peer.next_conn(POLL_LIMIT, "second dial").await;

        close_unsupported(&a);
        let closed_at = std::time::Instant::now();
        let observe = Duration::from_millis(300);
        // A stall long enough to let the (erroneous) window expire would let
        // traffic erase the evidence, so every step is held to this budget.
        let budget = TEST_WINDOW - Duration::from_millis(100);
        let mut rounds = 0u32;
        while rounds == 0 || closed_at.elapsed() < observe {
            // Check the window right before sending, and the budget before that.
            let before = closed_at.elapsed();
            assert!(
                before < budget,
                "timing: {before:?} since the close before round {} exceeds the {budget:?} budget",
                rounds + 1
            );
            assert_eq!(
                h.waiting_until(),
                None,
                "stale close must not open a window (before round {})",
                rounds + 1
            );
            h.round_trip(b"through-b").await;
            rounds += 1;
            let after = closed_at.elapsed();
            assert!(
                after < budget,
                "timing: round {rounds} ended {after:?} after the close, over the {budget:?} budget"
            );
            assert_eq!(
                h.waiting_until(),
                None,
                "stale close must not open a window (after round {rounds})"
            );
        }
        let total = closed_at.elapsed();
        assert!(
            rounds >= 1 && total >= observe,
            "timing: {rounds} rounds in {total:?}, need >= {observe:?}"
        );

        h.shutdown().await;
        peer.shutdown().await;
    }

    #[tokio::test]
    async fn reader_close_racing_a_send_cannot_redial() {
        let mut peer = test_peer(true).await;
        let h = forward(udp_dialer(&peer).await, TEST_WINDOW).await;
        h.round_trip(b"a").await;
        let a = peer.next_conn(POLL_LIMIT, "first dial").await;

        let dialing = h.dialer.dial.lock().await;
        let admitted = *h.hooks.admitted.subscribe().borrow();
        h.send(b"racing").await;
        wait_past(&h.hooks.admitted, admitted, "datagram admitted").await;
        let failed = *h.hooks.reader_failed.subscribe().borrow();
        close_unsupported(&a);
        wait_past(&h.hooks.reader_failed, failed, "reader matched the close").await;
        drop(dialing);

        let end = h.window_end().await;
        h.send_through_window(&mut peer, end).await;
        h.resume_after(&mut peer, end).await;

        h.shutdown().await;
        peer.shutdown().await;
    }

    #[tokio::test]
    async fn reader_close_before_admission_cannot_redial() {
        let mut peer = test_peer(true).await;
        let h = forward(udp_dialer(&peer).await, TEST_WINDOW).await;
        h.round_trip(b"a").await;
        let a = peer.next_conn(POLL_LIMIT, "first dial").await;

        let failed = *h.hooks.reader_failed.subscribe().borrow();
        close_unsupported(&a);
        wait_past(&h.hooks.reader_failed, failed, "reader matched the close").await;
        eventually("reader records the failure", || async {
            h.waiting_until().is_some()
        })
        .await;

        let end = h.window_end().await;
        h.send_through_window(&mut peer, end).await;
        h.resume_after(&mut peer, end).await;

        h.shutdown().await;
        peer.shutdown().await;
    }

    #[tokio::test]
    async fn shutdown_while_gate_held_is_prompt() {
        let peer = test_peer(true).await;
        let h = forward(udp_dialer(&peer).await, TEST_WINDOW).await;
        let _held = h.gate.clone().lock_owned().await;
        let waiting = *h.hooks.gate_waiting.subscribe().borrow();
        h.send(b"blocked").await;
        wait_past(&h.hooks.gate_waiting, waiting, "loop waits on the gate").await;

        h.shutdown_within(Duration::from_secs(1)).await;
        peer.shutdown().await;
    }
}
