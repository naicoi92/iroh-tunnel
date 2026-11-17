//! Serve role loop: publish local services into Iroh.
//!
//! Accepts incoming connections on the bound endpoint and pipes every
//! bidirectional stream on each to the matching local TCP/UDP service.
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

use anyhow::{Context, Result};
use iroh::endpoint::{Connection, VarInt};
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::watch;
use tokio::task::JoinSet;
use tokio::time::Instant;

use crate::endpoint;
use crate::fault::{self, Fault};
use crate::locked::Locked;
use crate::observation::{
    PeerConnectionObservation, ServeObservation, ServiceObservation, STATUS_FLUSH_INTERVAL,
};
use crate::proto;
use crate::runtime::support;
use crate::runtime::{ReadyServices, ServeReady};
use crate::runtime::{RoleHooks, RunCompletion, RunContext, StatusSink};
use crate::settings::{Protocol, ServeSettings, UdpLimits};
use crate::shutdown;

/// Run the serve role on a bound endpoint until `shutdown` resolves.
///
/// Order: emit `on_ready` -> initial status save -> admission. A failing
/// `on_ready` closes the endpoint and returns its error. The role owns
/// admission, drain, task joins and the endpoint close; the returned
/// [`RunCompletion`] tells the caller whether the status writer quiesced.
pub async fn run_serve(
    ep: iroh::Endpoint,
    settings: ServeSettings,
    ctx: RunContext,
    hooks: RoleHooks<ServeObservation>,
    shutdown: impl Future<Output = ()>,
) -> RunCompletion {
    let RoleHooks { on_ready, status } = hooks;
    if let Err(error) = support::announce_ready(&ep, ready_services(&settings), on_ready).await {
        return RunCompletion {
            result: Err(error),
            status_quiesced: true,
        };
    }
    let (fault, mut faults) = fault::channel();
    let mut role = ServeRole::start(&ep, &settings, status, fault).await;
    let result = support::run_until(&mut role.accept, shutdown, &mut faults).await;
    let status_quiesced = role.shutdown(&ctx).await;
    ep.close().await;
    support::completion(result, &mut faults, status_quiesced)
}

/// One running Serve role: its admission supervisor, the quarantine sinks
/// that outlive connections, and the status writer.
struct ServeRole {
    accept: JoinSet<()>,
    stop: watch::Sender<Option<Instant>>,
    sinks: Arc<shutdown::QuarantineSinks>,
    flush: support::StatusFlush,
}

impl ServeRole {
    /// Save the initial status, then open admission: nothing is accepted
    /// before the first snapshot is published.
    async fn start(
        ep: &iroh::Endpoint,
        settings: &ServeSettings,
        mut sink: Option<StatusSink<ServeObservation>>,
        fault: Fault,
    ) -> Self {
        let targets = service_targets(settings, &fault);
        let status = StatusSnapshot::new(ep, settings, &targets);
        let initial = status.render(current_home_relay(ep), Vec::new());
        let seeded = support::save_initial(sink.as_mut(), initial).await;

        // Live registry of connected peers, fed by the accept loop and read
        // by the status flush task (issue #57).
        let peers = PeerTracker::new(fault.clone());
        tracing::info!("serve endpoint ready, accepting connections");
        let (stop, stop_rx) = shutdown::channel();
        let sinks = Arc::new(shutdown::QuarantineSinks::new(fault));
        let mut accept = JoinSet::new();
        accept.spawn({
            let (ep, peers, sinks) = (ep.clone(), peers.clone(), sinks.clone());
            async move { accept_loop(&ep, targets, peers, sinks, stop_rx).await }
        });
        // Refresh serve-status.json when the rendered snapshot changes, at
        // most once per STATUS_FLUSH_INTERVAL: no disk churn under busy
        // stream churn, still near-live for operators.
        let flush = support::StatusFlush::spawn(sink, |sink, stop| {
            status_flush_loop(
                status,
                ep.clone(),
                peers,
                sink,
                seeded,
                stop,
                STATUS_FLUSH_INTERVAL,
            )
        });
        Self {
            accept,
            stop,
            sinks,
            flush,
        }
    }

    /// Stop admission, drain connections and sinks against one deadline,
    /// then stop the status writer; returns whether the writer quiesced.
    async fn shutdown(mut self, ctx: &RunContext) -> bool {
        shutdown::begin(&self.stop, ctx);
        // The accept supervisor seals handshakes and connection admission;
        // its connection children seal stream admission and join pipe leaves.
        shutdown::join(&mut self.accept).await;
        // Quarantine sinks outlive their connections but not the role.
        let drain_deadline = (*self.stop.borrow()).unwrap_or_else(Instant::now);
        self.sinks.shutdown(drain_deadline).await;
        // The run owner removes the status file only when the writer
        // quiesced.
        self.flush.stop().await
    }
}

/// The ALPN -> target lookup that demultiplexes accepted connections.
fn service_targets(settings: &ServeSettings, fault: &Fault) -> HashMap<Vec<u8>, ServiceTarget> {
    settings
        .services
        .iter()
        .map(|svc| {
            let local_addr = format!("{}:{}", svc.host, svc.port);
            let target = ServiceTarget::new(local_addr, svc.protocol, svc.udp, fault.clone());
            (proto::alpn_for(&svc.name), target)
        })
        .collect()
}

/// A service's name from its ALPN, or the raw bytes when it is not ours.
fn service_name(alpn: &[u8]) -> String {
    proto::name_from_alpn(alpn)
        .map(String::from)
        .unwrap_or_else(|| format!("{alpn:02x?}"))
}

fn ready_services(settings: &ServeSettings) -> ReadyServices {
    if settings.services.is_empty() {
        tracing::warn!("no services configured; nothing to serve");
    }
    ReadyServices::Serve(
        settings
            .services
            .iter()
            .map(|svc| ServeReady {
                name: svc.name.clone(),
                protocol: svc.protocol,
                local_addr: format!("{}:{}", svc.host, svc.port),
            })
            .collect(),
    )
}

/// One served service: where to dial locally + the live stream counter.
#[derive(Clone)]
struct ServiceTarget {
    local_addr: String,
    protocol: Protocol,
    active_streams: Arc<AtomicU64>,
    /// Resolved UDP limits and live counters; used only by a UDP service.
    udp_limits: UdpLimits,
    udp_stats: Arc<crate::udp::UdpStats>,
    /// Live flows of each peer across all its connections (per-peer cap).
    udp_peer_slots: Arc<crate::udp::PeerFlowSlots>,
    /// How a UDP flow's target address is resolved when the flow opens.
    udp_resolver: UpstreamResolver,
}

impl ServiceTarget {
    /// `fault` ends the role when the service's shared per-peer counts are
    /// poisoned.
    fn new(local_addr: String, protocol: Protocol, udp_limits: UdpLimits, fault: Fault) -> Self {
        Self {
            local_addr,
            protocol,
            udp_limits,
            udp_stats: Arc::default(),
            udp_peer_slots: Arc::new(crate::udp::PeerFlowSlots::new(fault)),
            udp_resolver: UpstreamResolver::default(),
            active_streams: Arc::new(AtomicU64::new(0)),
        }
    }

    fn active_streams(&self) -> Arc<AtomicU64> {
        self.active_streams.clone()
    }
}

/// Observation template: everything immutable for the life of the process,
/// plus the live per-service counters rendered into `active_connections`.
/// The home relay is not stored here: iroh assigns it after bind and may
/// change it on failover, so each render takes the current value (#177).
///
/// Since 0.2.0 `active_connections` counts *active streams* (in-flight pipes),
/// not iroh connections — with multiplexing one connection carries many
/// channels, so streams are what an operator cares about.
struct StatusSnapshot {
    node_id: String,
    pid: u32,
    started_at: u64,
    services: Vec<StatusServiceRow>,
}
struct StatusServiceRow {
    name: String,
    protocol: String,
    local_addr: String,
    active_streams: Arc<AtomicU64>,
    /// Present for a UDP service.
    udp_stats: Option<Arc<crate::udp::UdpStats>>,
}

impl StatusSnapshot {
    /// The immutable rows of `settings`' services, wired to their targets'
    /// live counters.
    fn new(
        ep: &iroh::Endpoint,
        settings: &ServeSettings,
        targets: &HashMap<Vec<u8>, ServiceTarget>,
    ) -> Self {
        let services = settings
            .services
            .iter()
            .map(|s| {
                let target = targets.get(&proto::alpn_for(&s.name));
                StatusServiceRow {
                    name: s.name.clone(),
                    protocol: s.protocol.as_str().to_string(),
                    local_addr: support::format_local_addr(&s.host, s.port),
                    active_streams: target
                        .map(ServiceTarget::active_streams)
                        .unwrap_or_default(),
                    udp_stats: target
                        .filter(|_| s.protocol == Protocol::Udp)
                        .map(|t| t.udp_stats.clone()),
                }
            })
            .collect();
        Self {
            node_id: endpoint::node_id_string(ep),
            pid: std::process::id(),
            started_at: support::unix_now_secs(),
            services,
        }
    }

    fn render(
        &self,
        home_relay: Option<String>,
        connections: Vec<PeerConnectionObservation>,
    ) -> ServeObservation {
        ServeObservation {
            node_id: self.node_id.clone(),
            home_relay,
            pid: self.pid,
            started_at: self.started_at,
            services: self
                .services
                .iter()
                .map(|s| ServiceObservation {
                    name: s.name.clone(),
                    protocol: s.protocol.clone(),
                    local_addr: s.local_addr.clone(),
                    active_connections: s.active_streams.load(Ordering::Relaxed),
                    udp: s.udp_stats.as_ref().map(|stats| stats.snapshot()),
                })
                .collect(),
            connections,
        }
    }
}

/// Live registry of which peers are connected over which services (issue #57).
///
/// The accept loop tracks `(peer, service)` when a connection is accepted and
/// untracks it when the connection task ends; the status flush task renders
/// the registry into `connections`. A peer with several connections to the
/// same service (e.g. a legacy access peer) is
/// refcounted, and services are merged across a peer's connections.
///
/// The registry is role-wide: a poisoned lock (a panic mid-section may have
/// left the refcounts inconsistent) raises a role fault (#216, #226).
#[derive(Clone)]
struct PeerTracker {
    registry: Arc<Locked<PeerRegistry>>,
    fault: Fault,
}

/// Connection refcount per service, per peer.
type PeerRegistry = HashMap<iroh::EndpointId, HashMap<String, u64>>;

impl PeerTracker {
    fn new(fault: Fault) -> Self {
        Self {
            registry: Arc::default(),
            fault,
        }
    }

    /// Run `section` on the registry; a poisoned registry raises the role
    /// fault and fails the caller.
    fn with<R>(&self, section: impl FnOnce(&mut PeerRegistry) -> R) -> Result<R> {
        self.registry.with(section).map_err(|poisoned| {
            self.fault
                .raise(anyhow::Error::new(poisoned).context("peer tracker"));
            anyhow::Error::new(poisoned).context("peer tracker")
        })
    }

    /// Record one accepted connection from `peer` for `service`, returning
    /// the RAII handle that untracks it.
    ///
    /// The guard's [`Drop`] decrements the refcount, so a connection task
    /// that returns *or panics* cannot leak the entry — the unwind drops the
    /// guard like any local.
    ///
    /// # Errors
    ///
    /// The registry lock is poisoned.
    fn track(&self, peer: iroh::EndpointId, service: &str) -> Result<TrackedPeer> {
        self.with(|registry| {
            *registry
                .entry(peer)
                .or_default()
                .entry(service.to_string())
                .or_default() += 1;
        })?;
        Ok(TrackedPeer {
            peers: self.clone(),
            peer,
            service: service.to_string(),
        })
    }

    /// Drop one connection from `peer` for `service`; removes the service
    /// (and with it the peer) when its refcount hits zero.
    ///
    /// # Errors
    ///
    /// The registry lock is poisoned.
    fn untrack(&self, peer: iroh::EndpointId, service: &str) -> Result<()> {
        self.with(|registry| {
            let Some(services) = registry.get_mut(&peer) else {
                return;
            };
            let Some(count) = services.get_mut(service) else {
                return;
            };
            *count = count.saturating_sub(1);
            if *count == 0 {
                services.remove(service);
                if services.is_empty() {
                    registry.remove(&peer);
                }
            }
        })
    }

    /// Point-in-time copy of the registry: `(peer, sorted service names)`,
    /// sorted by peer id for deterministic output.
    ///
    /// # Errors
    ///
    /// The registry lock is poisoned.
    fn snapshot(&self) -> Result<Vec<(iroh::EndpointId, Vec<String>)>> {
        let mut peers: Vec<_> = self.with(|registry| {
            registry
                .iter()
                .map(|(peer, services)| {
                    let mut names: Vec<String> = services.keys().cloned().collect();
                    names.sort();
                    (*peer, names)
                })
                .collect()
        })?;
        // Cached key: the node-id string is computed once per peer instead
        // of once per comparison.
        peers.sort_by_cached_key(|(peer, _)| peer.to_string());
        Ok(peers)
    }
}

/// RAII untrack handle returned by [`PeerTracker::track`].
struct TrackedPeer {
    peers: PeerTracker,
    peer: iroh::EndpointId,
    service: String,
}

impl Drop for TrackedPeer {
    fn drop(&mut self) {
        // Drop cannot propagate; a poisoned registry already raised the
        // role fault.
        let _ = self.peers.untrack(self.peer, &self.service);
    }
}

/// Render the tracker into the status file's `connections` array: one row
/// per tracked peer with a fresh [`crate::conn_path::peer_path_report`]
/// snapshot.
///
/// Peers whose iroh remote-map entry has already expired (`remote_info` →
/// `None`) are skipped — the flush cadence picks them back up if they
/// reconnect.
///
/// # Errors
///
/// The tracker's lock is poisoned.
async fn render_connections(
    ep: &iroh::Endpoint,
    peers: &PeerTracker,
) -> Result<Vec<PeerConnectionObservation>> {
    let mut out = Vec::new();
    for (peer, services) in peers.snapshot()? {
        let Some(report) = crate::conn_path::peer_path_report(ep, peer).await else {
            continue;
        };
        out.push(PeerConnectionObservation {
            path: report,
            services,
        });
    }
    Ok(out)
}

/// The endpoint's home relay as rendered into the status file, read live:
/// `None` until iroh assigns one after bind (issue #177).
fn current_home_relay(ep: &iroh::Endpoint) -> Option<String> {
    endpoint::home_relay(ep).map(|relay| relay.to_string())
}

/// Periodically rewrite serve-status.json, but only when the rendered
/// snapshot changed (stream counters, peer set, home relay, or transport
/// states).
///
/// Stops COOPERATIVELY on `stop`: aborting this task would not cancel an
/// in-flight `spawn_blocking` save (write+fsync+rename runs to completion
/// on the blocking pool and could recreate the file after the shutdown
/// cleanup removed it). The run loop therefore signals, then JOINS this
/// task before removing the file — the loop here must exit on the signal
/// while letting the current save finish.
///
/// `interval` is a seam for unit-testing the stop handshake; production
/// passes [`STATUS_FLUSH_INTERVAL`]. `sink` is the host's publisher.
async fn status_flush_loop(
    status: StatusSnapshot,
    ep: iroh::Endpoint,
    peers: PeerTracker,
    mut sink: StatusSink<ServeObservation>,
    mut last: Option<ServeObservation>,
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
        let connections = match render_connections(&ep, &peers).await {
            Ok(connections) => connections,
            Err(error) => {
                // A poisoned tracker is fatal for this task (#216).
                tracing::error!("status flush stopped: {error:#}");
                break;
            }
        };
        let observation = status.render(current_home_relay(&ep), connections);
        if last.as_ref() == Some(&observation) {
            continue;
        }
        // Only record the observation as written on success, so a failed
        // write is retried on the next tick rather than silently dropped
        // until the next change.
        match sink.save(observation.clone()).await {
            Ok(()) => last = Some(observation),
            Err(e) => tracing::warn!("failed to write status file: {e:#}"),
        }
    }
}

/// Accept connections forever, demultiplexing each to its service by ALPN.
///
/// Stop cancels admission and pending handshake, then joins connection owners.
/// Per-connection errors are logged, not propagated.
async fn accept_loop(
    ep: &iroh::Endpoint,
    targets: HashMap<Vec<u8>, ServiceTarget>,
    peers: PeerTracker,
    sinks: Arc<shutdown::QuarantineSinks>,
    mut stop: shutdown::Stop,
) {
    let mut admission = ServeAdmission {
        targets,
        peers,
        sinks,
        connections: JoinSet::new(),
        watchers: JoinSet::new(),
    };
    loop {
        let incoming = tokio::select! {
            biased;
            _ = shutdown::stopped(&mut stop) => break,
            Some(result) = admission.connections.join_next(), if !admission.connections.is_empty() => {
                shutdown::observe(result);
                continue;
            }
            Some(result) = admission.watchers.join_next(), if !admission.watchers.is_empty() => {
                shutdown::observe(result);
                continue;
            }
            incoming = ep.accept() => match incoming {
                Some(incoming) => incoming,
                None => break,
            }
        };
        // Dropping a pending handshake cancels it; established connections
        // remain open until their owned pipes have drained.
        let conn = tokio::select! {
            biased;
            _ = shutdown::stopped(&mut stop) => break,
            result = incoming => match result {
                Ok(conn) => conn,
                Err(error) => {
                    tracing::warn!("incoming handshake failed: {error}");
                    continue;
                }
            }
        };
        let Some(accepted) = admission.demultiplex(conn) else {
            continue;
        };
        // Query the fresh path so the connect line carries it — symmetric
        // with the access role's established lines (`via relay=…`). Right
        // after the handshake iroh may not have an active-path snapshot
        // yet; that renders as `paths pending` and the status file's
        // `connections` array catches up on its next flush.
        let report = tokio::select! {
            biased;
            _ = shutdown::stopped(&mut stop) => break,
            report = crate::conn_path::peer_path_report(ep, accepted.conn.remote_id()) => report,
        };
        admission.admit(accepted, report.as_ref(), stop.clone());
    }
    admission.close().await;
}

/// The connections the accept loop owns and what it hands each of them.
struct ServeAdmission {
    targets: HashMap<Vec<u8>, ServiceTarget>,
    peers: PeerTracker,
    sinks: Arc<shutdown::QuarantineSinks>,
    connections: JoinSet<()>,
    /// Disconnect loggers, one per admitted connection.
    watchers: JoinSet<()>,
}

/// A handshaken connection matched to its service.
struct AcceptedConnection {
    conn: Connection,
    target: ServiceTarget,
    service: String,
}

impl ServeAdmission {
    /// Match `conn` to its service by the negotiated ALPN; an unknown ALPN
    /// is logged and the connection dropped.
    fn demultiplex(&self, conn: Connection) -> Option<AcceptedConnection> {
        let alpn = conn.alpn();
        let service = service_name(alpn);
        let Some(target) = self.targets.get(alpn).cloned() else {
            tracing::warn!("connection with unknown ALPN for service '{service}', dropping");
            return None;
        };
        Some(AcceptedConnection {
            conn,
            target,
            service,
        })
    }

    /// Log the connection and spawn its owner and disconnect watcher.
    fn admit(
        &mut self,
        accepted: AcceptedConnection,
        report: Option<&crate::observation::PeerPathObservation>,
        stop: shutdown::Stop,
    ) {
        let AcceptedConnection {
            conn,
            target,
            service,
        } = accepted;
        // The access peer's NodeId, from its cert. Logged on connect and
        // (via the watcher below) on disconnect, so operators can see who is
        // tunneling in and correlate with access-side logs.
        let remote_id = conn.remote_id();
        tracing::info!(
            peer = %remote_id,
            service = %service,
            "peer connected via {}",
            crate::conn_path::render_established_paths(report)
        );
        // Watcher: emit a disconnect line when the QUIC connection closes. The
        // weak handle is registered while `conn` is still alive (before the
        // stream-handling task takes it), so iroh guarantees the close event is
        // delivered even if the connection drops before this resolves.
        self.watchers.spawn(support::disconnect_watcher(
            &conn,
            remote_id.to_string(),
            format!("peer disconnected (service {service})"),
        ));
        let (peers, sinks) = (self.peers.clone(), self.sinks.clone());
        self.connections.spawn(async move {
            // Track the peer for the status file's `connections` array for
            // as long as this connection lives (issue #57). The guard
            // untracks on drop — normal return or panic unwind alike — so
            // the refcount can never leak.
            let _tracked = match peers.track(remote_id, &service) {
                Ok(tracked) => tracked,
                Err(error) => {
                    // A poisoned tracker raised the role fault (#226), which
                    // logs it; this connection ends.
                    tracing::debug!(peer = %remote_id, "connection refused: {error:#}");
                    return;
                }
            };
            handle_connection(&conn, target, &sinks, stop).await;
        });
    }

    /// Join every connection owner, then end the watchers.
    async fn close(mut self) {
        // Supervisors are not aborted: each joins its own stream children
        // using the same absolute deadline. Idle connections do not consume
        // grace.
        shutdown::join(&mut self.connections).await;
        self.watchers.abort_all();
        shutdown::join(&mut self.watchers).await;
    }
}

/// Bound on one stream's local target connect; see `handle_connection`.
const LOCAL_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Connect one stream's local target within `limit`.
///
/// A failure or timeout is logged and yields `None`: the caller drops the
/// stream halves, which resets only this channel; the shared connection and
/// sibling streams are untouched.
async fn connect_local<F, T>(
    connect: F,
    limit: Duration,
    local_addr: &str,
    stream_no: u64,
) -> Option<T>
where
    F: Future<Output = std::io::Result<T>>,
{
    match tokio::time::timeout(limit, connect).await {
        Ok(Ok(local)) => Some(local),
        Ok(Err(error)) => {
            tracing::warn!(%local_addr, stream = stream_no,
                "failed to connect local service, resetting stream: {error}");
            None
        }
        Err(_) => {
            tracing::warn!(%local_addr, stream = stream_no,
                "timed out connecting local service, resetting stream");
            None
        }
    }
}

/// Own stream admission and pipe tasks for one multiplexed connection.
async fn handle_connection(
    conn: &Connection,
    target: ServiceTarget,
    sinks: &shutdown::QuarantineSinks,
    mut stop: shutdown::Stop,
) {
    if target.protocol == Protocol::Udp {
        return handle_udp_connection(conn, target, sinks, stop).await;
    }
    let mut streams = JoinSet::new();
    let mut stream_no: u64 = 0;
    let deadline = loop {
        let (send, recv) = tokio::select! {
            biased;
            deadline = shutdown::stopped(&mut stop) => break Some(deadline),
            Some(result) = streams.join_next(), if !streams.is_empty() => {
                shutdown::observe(result);
                continue;
            }
            result = conn.accept_bi() => match result {
                Ok(pair) => pair,
                Err(error) => {
                    tracing::debug!("connection closed: {error}");
                    break None;
                }
            }
        };
        stream_no += 1;
        tracing::debug!(stream = stream_no, "accepted stream on connection");
        let counter = ActiveStream::new(target.active_streams());
        streams.spawn(serve_stream(
            (send, recv),
            target.local_addr.clone(),
            stream_no,
            counter,
        ));
    };
    if let Some(deadline) = deadline {
        shutdown::drain(&mut streams, deadline).await;
    } else {
        // Peer loss cannot drain and must never replay partially copied data.
        streams.abort_all();
        shutdown::join(&mut streams).await;
    }
}

/// Pipe one accepted stream to a fresh connection to the local target, then
/// wait (bounded) for the peer to acknowledge the final bytes.
async fn serve_stream(
    (send, recv): (iroh::endpoint::SendStream, iroh::endpoint::RecvStream),
    local_addr: String,
    stream_no: u64,
    _counter: ActiveStream,
) {
    // Connect inside the stream's own task: a slow target must not hold up
    // accepting other streams on this connection. A failed or timed-out
    // connect closes only this channel (dropping the stream halves resets
    // it).
    let Some(local) = connect_local(
        TcpStream::connect(&local_addr),
        LOCAL_CONNECT_TIMEOUT,
        &local_addr,
        stream_no,
    )
    .await
    else {
        return;
    };
    // AsyncWrite::shutdown only queues QUIC FIN. Keep channel owned until
    // final bytes are acknowledged, or last-pipe completion could close the
    // endpoint and discard its queued response. The wait is bounded: a
    // vanished peer must not pin this task and the active_streams counter.
    let delivered = send.stopped();
    if let Err(error) = crate::tcp::pipe_tcp_bidirectional(local, (recv, send)).await {
        tracing::warn!("stream error: {error:#}");
        return;
    }
    match crate::tcp::await_delivery(delivered, crate::tcp::DELIVERY_ACK_GRACE).await {
        crate::tcp::Delivery::Acknowledged => tracing::debug!("stream closed normally"),
        crate::tcp::Delivery::PeerStopped(code) => {
            tracing::debug!(%code, "peer stopped completed stream");
        }
        crate::tcp::Delivery::Unconfirmed => tracing::warn!(
            stream = stream_no,
            "peer lost: no delivery acknowledgement within grace"
        ),
        crate::tcp::Delivery::Lost(error) => tracing::warn!("stream delivery error: {error}"),
    }
}

/// One flow's slot under its service's cap, given back on drop unless it is
/// handed to a quarantine sink, so every way an opening or live flow ends
/// (refusal, failed open, abort, expiry) releases it exactly once.
struct ServiceFlowSlot {
    stats: Arc<crate::udp::UdpStats>,
    held: bool,
}

impl ServiceFlowSlot {
    fn try_reserve(stats: &Arc<crate::udp::UdpStats>, max: usize) -> Option<Self> {
        stats.try_open_flow(max).then(|| Self {
            stats: stats.clone(),
            held: true,
        })
    }

    /// Keep the slot for a quarantined socket; the sink's [`QuarantineSlot`]
    /// releases it when the sink ends.
    fn into_quarantine(mut self) -> QuarantineSlot {
        self.held = false;
        self.stats.quarantine_started();
        QuarantineSlot(self.stats.clone())
    }
}

impl Drop for ServiceFlowSlot {
    fn drop(&mut self) {
        if self.held {
            self.stats.flows_closed(1);
        }
    }
}

/// The cap slots a flow holds from admission until it stops being live.
struct FlowReservation {
    /// Released when the flow stops being live; a quarantined socket holds
    /// none.
    peer: crate::udp::PeerFlowSlot,
    service: ServiceFlowSlot,
}

/// Why a flow got no reservation.
enum ReserveRefused {
    /// The named cap (`peer` or `service`) is full.
    Cap(&'static str),
    /// The per-peer counts are poisoned; the role fault is already raised.
    Poisoned,
}

impl FlowReservation {
    /// Reserve the peer's slot (its live flows across all its connections),
    /// then the service's (shared by every peer).
    fn try_new(target: &ServiceTarget, peer: iroh::EndpointId) -> Result<Self, ReserveRefused> {
        let limits = target.udp_limits;
        let peer = target
            .udp_peer_slots
            .try_reserve(peer, limits.max_flows_per_peer)
            .map_err(|_| ReserveRefused::Poisoned)?
            .ok_or(ReserveRefused::Cap("peer"))?;
        let service = ServiceFlowSlot::try_reserve(&target.udp_stats, limits.max_flows)
            .ok_or(ReserveRefused::Cap("service"))?;
        Ok(Self { peer, service })
    }
}

/// One live flow's upstream: its socket, the bounded queue feeding the writer
/// task, the handles that end the reply relay and the writer, and its slots.
struct UdpUpstream {
    socket: Arc<UdpSocket>,
    queue: crate::udp::FlowQueue,
    relay: tokio::task::AbortHandle,
    writer: tokio::task::AbortHandle,
    reservation: FlowReservation,
}

/// A flow of one peer connection on Serve.
enum UdpFlow {
    /// Its upstream is still being resolved and bound by an opener task,
    /// which owns the flow's reservation; datagrams wait in the queue.
    Opening {
        queue: crate::udp::FlowQueue,
        opener: tokio::task::AbortHandle,
    },
    Live(UdpUpstream),
}

impl UdpFlow {
    fn queue(&self) -> &crate::udp::FlowQueue {
        match self {
            Self::Opening { queue, .. } | Self::Live(UdpUpstream { queue, .. }) => queue,
        }
    }
}

/// One peer connection's UDP flows, shared with the reply relays. A poisoned
/// table ends its connection only (#226).
type UdpFlowTable = Arc<Locked<crate::udp::PeerFlows<UdpFlow>>>;

/// What an opener task hands back to its connection: the flow's queued
/// datagrams and, when the upstream opened, its socket and reservation.
struct OpenAttempt {
    flow: crate::udp::FlowId,
    rx: tokio::sync::mpsc::Receiver<bytes::Bytes>,
    opened: Result<(UdpSocket, FlowReservation)>,
}

/// Open one flow's upstream away from the receive loop. A failed open
/// releases the reservation before reporting back; an aborted one releases
/// it when the task is dropped.
async fn open_flow(
    flow: crate::udp::FlowId,
    target: ServiceTarget,
    reservation: FlowReservation,
    rx: tokio::sync::mpsc::Receiver<bytes::Bytes>,
) -> OpenAttempt {
    let opened = open_upstream(&target.local_addr, &target.udp_resolver).await;
    OpenAttempt {
        flow,
        rx,
        opened: opened.map(|socket| (socket, reservation)),
    }
}

/// The UDP flows of one peer connection and the tasks that serve them, owned
/// by the connection's receive loop.
struct UdpConnectionFlows<'conn> {
    conn: &'conn Connection,
    target: ServiceTarget,
    table: UdpFlowTable,
    /// Ends this connection when its table is poisoned.
    fault: Fault,
    /// Reply relays and queue writers of live flows.
    readers: JoinSet<()>,
    /// Upstreams still being opened.
    openers: JoinSet<OpenAttempt>,
}

impl<'conn> UdpConnectionFlows<'conn> {
    fn new(conn: &'conn Connection, target: ServiceTarget, fault: Fault) -> Self {
        Self {
            conn,
            target,
            table: UdpFlowTable::default(),
            fault,
            readers: JoinSet::new(),
            openers: JoinSet::new(),
        }
    }

    /// Run `section` on the flow table; `None` once the table is poisoned,
    /// which raises the connection's fault.
    fn flows<R>(
        &self,
        section: impl FnOnce(&mut crate::udp::PeerFlows<UdpFlow>) -> R,
    ) -> Option<R> {
        flow_table_section(&self.table, &self.fault, section)
    }

    fn stats(&self) -> &Arc<crate::udp::UdpStats> {
        &self.target.udp_stats
    }

    /// Route one tunnel datagram to its flow's queue, admitting a new flow
    /// if needed. Never waits: opening runs in its own task.
    fn route(&mut self, datagram: &bytes::Bytes) {
        use crate::udp::{Admission, DropReason, Push};
        let Ok((flow, payload)) = crate::udp::decode(datagram) else {
            self.stats().record_drop(DropReason::Malformed);
            tracing::debug!("dropping malformed udp datagram");
            return;
        };
        let now = std::time::Instant::now();
        let Some(admission) = self.flows(|flows| flows.admit(flow, now)) else {
            return;
        };
        let queue = match admission {
            Admission::Unknown => {
                self.stats().record_drop(DropReason::UnknownFlow);
                tracing::debug!(flow = flow.0, "dropping datagram for unknown udp flow");
                return;
            }
            Admission::Live => {
                match self.flows(|flows| flows.get(flow).map(|entry| entry.queue().clone())) {
                    Some(Some(queue)) => queue,
                    _ => return,
                }
            }
            Admission::New => match self.start_open(flow, now) {
                Some(queue) => queue,
                None => return,
            },
        };
        match queue.push(payload) {
            Push::Queued => {}
            Push::Full => {
                self.stats().record_drop(DropReason::QueueFull);
                tracing::debug!(flow = flow.0, "udp flow queue full, dropping newest");
            }
            Push::Closed => {
                self.stats().record_drop(DropReason::UpstreamError);
                tracing::debug!(flow = flow.0, "udp flow writer ended, dropping datagram");
            }
        }
    }

    /// Reserve a new flow's cap slots and start opening its upstream; the
    /// flow's queue takes datagrams meanwhile. A refused flow is counted and
    /// its identifier spent; nothing running is evicted.
    fn start_open(
        &mut self,
        flow: crate::udp::FlowId,
        now: std::time::Instant,
    ) -> Option<crate::udp::FlowQueue> {
        let reservation = match FlowReservation::try_new(&self.target, self.conn.remote_id()) {
            Ok(reservation) => reservation,
            Err(ReserveRefused::Cap(cap)) => {
                self.flows(|flows| flows.refuse(flow))?;
                self.stats().record_drop(crate::udp::DropReason::Cap);
                tracing::debug!(flow = flow.0, cap, "udp flow refused: flow cap reached");
                return None;
            }
            // The role is stopping; the datagram is dropped uncounted.
            Err(ReserveRefused::Poisoned) => return None,
        };
        let (queue, rx) = crate::udp::FlowQueue::new(self.target.udp_limits.queue_depth);
        let opener = self
            .openers
            .spawn(open_flow(flow, self.target.clone(), reservation, rx));
        let entry = UdpFlow::Opening {
            queue: queue.clone(),
            opener,
        };
        // A poisoned table ends the connection, whose close aborts the opener.
        self.flows(|flows| flows.insert(flow, entry, now))?;
        Some(queue)
    }

    /// Take an opener's result: make the flow live, or refuse it when its
    /// upstream could not be opened.
    fn finish_open(&mut self, attempt: OpenAttempt) {
        let OpenAttempt { flow, rx, opened } = attempt;
        let opening = self.flows(|flows| match flows.get(flow) {
            Some(UdpFlow::Opening { queue, .. }) => Some(queue.clone()),
            _ => None,
        });
        // The flow ended while its open finished (or the table is poisoned):
        // drop the result, releasing its reservation.
        let Some(Some(queue)) = opening else {
            return;
        };
        match opened {
            Ok((socket, reservation)) => {
                let upstream = self.spawn_live(flow, Arc::new(socket), queue, rx, reservation);
                // Only this loop ends flows, so the flow is still opening.
                // The finished opener's handle is all the old entry holds; a
                // poisoned table drops the upstream, whose tasks the
                // connection's close aborts.
                drop(self.flows(|flows| flows.replace(flow, UdpFlow::Live(upstream))));
            }
            Err(error) => {
                // Resolve, bind or descriptor failure refuses the flow and
                // drops what it queued; the process keeps running (contract:
                // counter `cap`).
                drop(self.flows(|flows| flows.remove(flow)));
                // The queue holds at least the datagram that admitted it.
                for _ in 0..rx.len() {
                    self.stats().record_drop(crate::udp::DropReason::Cap);
                }
                tracing::warn!(flow = flow.0, "cannot open udp upstream: {error:#}");
            }
        }
    }

    /// Start a flow's reply relay and its queue writer.
    fn spawn_live(
        &mut self,
        flow: crate::udp::FlowId,
        socket: Arc<UdpSocket>,
        queue: crate::udp::FlowQueue,
        rx: tokio::sync::mpsc::Receiver<bytes::Bytes>,
        reservation: FlowReservation,
    ) -> UdpUpstream {
        let relay = self.readers.spawn(relay_upstream_replies(
            self.conn.clone(),
            flow,
            socket.clone(),
            (self.table.clone(), self.fault.clone()),
            self.stats().clone(),
        ));
        // A writer per flow: a slow or blocked target stalls only this
        // flow's bounded queue, never the receive loop.
        let writer = self.readers.spawn(crate::udp::drain_queue(rx, {
            let socket = socket.clone();
            let stats = self.stats().clone();
            move |datagram: bytes::Bytes| {
                let socket = socket.clone();
                let stats = stats.clone();
                async move {
                    if let Err(error) = socket.send(&datagram).await {
                        stats.record_drop(crate::udp::DropReason::UpstreamError);
                        tracing::debug!(flow = flow.0, "udp upstream send failed: {error}");
                    }
                }
            }
        }));
        UdpUpstream {
            socket,
            queue,
            relay,
            writer,
            reservation,
        }
    }

    /// End every flow idle for the service's idle timeout.
    fn expire_idle(&self, stop: &shutdown::Stop, sinks: &shutdown::QuarantineSinks) {
        let idle = self.target.udp_limits.idle_timeout;
        let now = std::time::Instant::now();
        let Some(ended) = self.flows(|flows| flows.expire(now, idle)) else {
            return;
        };
        if !ended.is_empty() {
            tracing::debug!(count = ended.len(), "udp upstream flows expired");
        }
        for flow in ended {
            end_flow(flow, self.stats(), stop, sinks);
        }
    }

    /// End the connection's flows and join their tasks.
    async fn close(mut self, stop: &shutdown::Stop, sinks: &shutdown::QuarantineSinks) {
        // The connection's flows end with it, but their sockets are
        // quarantined like any expired flow: they outlive the incarnation
        // as sinks only. A poisoned table already ended the connection with
        // its fault; its flows are dropped, releasing their slots, when the
        // last table handle goes (this owner, after the relays are joined).
        let ended = self
            .table
            .with(crate::udp::PeerFlows::drain_all)
            .unwrap_or_default();
        for flow in ended {
            end_flow(flow, self.stats(), stop, sinks);
        }
        self.openers.abort_all();
        while let Some(result) = self.openers.join_next().await {
            shutdown::observe(result.map(drop));
        }
        self.readers.abort_all();
        shutdown::join(&mut self.readers).await;
    }
}

/// End one flow: quarantine a live flow's socket; cancel an opening flow,
/// whose opener releases its reservation when dropped.
fn end_flow(
    flow: UdpFlow,
    stats: &Arc<crate::udp::UdpStats>,
    stop: &shutdown::Stop,
    sinks: &shutdown::QuarantineSinks,
) {
    match flow {
        // An opening flow idle past its timeout (only when the idle timeout
        // is shorter than the resolve timeout) ends like a live one: its
        // queued datagrams are discarded uncounted.
        UdpFlow::Opening { opener, .. } => opener.abort(),
        UdpFlow::Live(upstream) => quarantine(upstream, stats, stop, sinks),
    }
}

/// Run `section` on a connection's flow table; `None` once it is poisoned,
/// which raises `fault` to end the connection.
fn flow_table_section<R>(
    table: &UdpFlowTable,
    fault: &Fault,
    section: impl FnOnce(&mut crate::udp::PeerFlows<UdpFlow>) -> R,
) -> Option<R> {
    table
        .with(section)
        .map_err(|poisoned| fault.raise(anyhow::Error::new(poisoned).context("udp flow table")))
        .ok()
}

/// Carry one peer's UDP flows over QUIC DATAGRAMs (#88).
///
/// Each flow gets its own connected upstream socket, so a reply is attributed
/// to a flow by the socket it arrives on, never by inspecting the payload.
/// Contract: `docs/udp-integration-contract.md`. Flows are capped per
/// peer and per service, expire after the service's idle timeout, and their
/// sockets are then quarantined as sinks; each flow has a bounded queue.
/// Upstreams open in their own tasks, so a slow open never stalls the
/// receive loop or another flow (#219).
async fn handle_udp_connection(
    conn: &Connection,
    target: ServiceTarget,
    sinks: &shutdown::QuarantineSinks,
    mut stop: shutdown::Stop,
) {
    let peer = conn.remote_id();
    let service = service_name(conn.alpn());
    if !accepts_datagrams(conn, peer, &service) {
        return;
    }
    let idle = target.udp_limits.idle_timeout;
    let (fault, mut faults) = fault::channel();
    let mut flows = UdpConnectionFlows::new(conn, target, fault);
    let mut sweep = tokio::time::interval(crate::udp::sweep_interval(idle));
    sweep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            biased;
            _ = shutdown::stopped(&mut stop) => break,
            // Already logged where it was raised; it ends this connection.
            _ = faults.first() => break,
            _ = sweep.tick() => flows.expire_idle(&stop, sinks),
            Some(result) = flows.readers.join_next(), if !flows.readers.is_empty() => {
                shutdown::observe(result);
            }
            Some(result) = flows.openers.join_next(), if !flows.openers.is_empty() => {
                match result {
                    Ok(attempt) => flows.finish_open(attempt),
                    // A failed opener released its reservation on unwind;
                    // its flow stays `Opening` until it expires.
                    Err(error) => shutdown::observe(Err(error)),
                }
            }
            result = conn.read_datagram() => match result {
                Ok(datagram) => flows.route(&datagram),
                Err(error) => {
                    log_udp_connection_end(&error, peer, &service);
                    break;
                }
            }
        }
    }
    flows.close(&stop, sinks).await;
}

/// Whether `conn` can carry datagrams. One that cannot never gets a flow
/// table, sweep or reader: it is closed with the dedicated code.
fn accepts_datagrams(conn: &Connection, peer: iroh::EndpointId, service: &str) -> bool {
    use crate::udp::{carries_datagrams, DATAGRAMS_UNSUPPORTED_CODE, DATAGRAMS_UNSUPPORTED_REASON};
    if carries_datagrams(conn.max_datagram_size()) {
        return true;
    }
    tracing::error!(
        peer = %peer,
        service = %service,
        "udp datagrams unsupported by access peer, closing the connection"
    );
    conn.close(
        VarInt::from_u32(DATAGRAMS_UNSUPPORTED_CODE),
        DATAGRAMS_UNSUPPORTED_REASON,
    );
    false
}

fn log_udp_connection_end(
    error: &iroh::endpoint::ConnectionError,
    peer: iroh::EndpointId,
    service: &str,
) {
    if crate::udp::closed_for_unsupported_datagrams(error) {
        tracing::error!(
            peer = %peer,
            service = %service,
            "access peer closed the udp connection: datagrams unsupported"
        );
    } else {
        tracing::debug!("udp connection closed: {error}");
    }
}

/// End a flow: stop relaying its replies and keep its socket open as a sink
/// for [`crate::udp::QUARANTINE`], so a late reply cannot reach a new flow
/// that the OS would otherwise give the same local port.
///
/// The socket keeps its service-cap slot (not its peer slot) until the sink
/// closes. The sink
/// is owned by the role run's `sinks`: it outlives the connection, ends early
/// on shutdown, and is joined or aborted within the stop deadline.
fn quarantine(
    upstream: UdpUpstream,
    stats: &Arc<crate::udp::UdpStats>,
    stop: &shutdown::Stop,
    sinks: &shutdown::QuarantineSinks,
) {
    let UdpUpstream {
        socket,
        relay,
        writer,
        reservation: FlowReservation { peer, service },
        ..
    } = upstream;
    // The flow is no longer live: its peer slot is free, while the socket
    // keeps its service slot until the sink ends.
    drop(peer);
    let slot = service.into_quarantine();
    // The relay and the writer hold clones of the socket; ending them leaves
    // the sink as the only reader. Datagrams still queued are discarded: the
    // flow is over.
    relay.abort();
    writer.abort();
    let stats = stats.clone();
    let mut stop = stop.clone();
    sinks.spawn(async move {
        // Released on every exit path, even if this task is aborted.
        let _slot = slot;
        let deadline = tokio::time::Instant::now() + crate::udp::QUARANTINE;
        // Replies are discarded; a small buffer is enough (the kernel drops
        // the excess of a datagram larger than it).
        let mut buf = vec![0u8; 2048];
        loop {
            tokio::select! {
                biased;
                _ = shutdown::stopped(&mut stop) => break,
                _ = tokio::time::sleep_until(deadline) => break,
                result = socket.recv(&mut buf) => match result {
                    Ok(_) => stats.record_drop(crate::udp::DropReason::QuarantineReply),
                    Err(error) if crate::udp::is_transient_recv_error(&error) => {}
                    // A socket that cannot be read again is done; release it.
                    Err(_) => break,
                },
            }
        }
    });
}

/// Longest wait for a target hostname to resolve when a flow opens.
const UPSTREAM_RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);

/// Gives a quarantined flow's slot back when the sink task ends for any
/// reason, including being aborted or unwinding from a panic.
struct QuarantineSlot(Arc<crate::udp::UdpStats>);

impl Drop for QuarantineSlot {
    fn drop(&mut self) {
        self.0.quarantine_ended();
    }
}

/// A pending resolution of a UDP target address.
type ResolveFuture =
    std::pin::Pin<Box<dyn Future<Output = std::io::Result<Vec<std::net::SocketAddr>>> + Send>>;

/// Resolves a UDP service's target address when one of its flows opens; the
/// system resolver by default, replaceable so tests can model a slow one.
#[derive(Clone)]
struct UpstreamResolver(Arc<dyn Fn(String) -> ResolveFuture + Send + Sync>);

impl Default for UpstreamResolver {
    fn default() -> Self {
        Self(Arc::new(|local_addr: String| {
            Box::pin(async move {
                let addrs = tokio::net::lookup_host(local_addr).await?;
                Ok(addrs.collect())
            })
        }))
    }
}

impl UpstreamResolver {
    fn resolve(&self, local_addr: &str) -> ResolveFuture {
        (self.0)(local_addr.to_string())
    }
}

/// Bind a fresh socket of the target's address family and connect it, so the
/// kernel drops replies from any other source.
async fn open_upstream(local_addr: &str, resolver: &UpstreamResolver) -> Result<UdpSocket> {
    // Bound the wait: a flow stuck resolving holds its cap slots.
    let resolved = tokio::time::timeout(UPSTREAM_RESOLVE_TIMEOUT, resolver.resolve(local_addr))
        .await
        .with_context(|| format!("resolving {local_addr} timed out"))?;
    let target = resolved
        .with_context(|| format!("cannot resolve {local_addr}"))?
        .into_iter()
        .next()
        .with_context(|| format!("{local_addr} resolved to no address"))?;
    let bind: std::net::SocketAddr = if target.is_ipv4() {
        (std::net::Ipv4Addr::UNSPECIFIED, 0).into()
    } else {
        (std::net::Ipv6Addr::UNSPECIFIED, 0).into()
    };
    let socket = UdpSocket::bind(bind)
        .await
        .with_context(|| format!("cannot bind a udp socket for {local_addr} on {bind}"))?;
    socket
        .connect(target)
        .await
        .with_context(|| format!("cannot connect udp upstream {local_addr} ({target})"))?;
    Ok(socket)
}

/// Forward each reply on `upstream` to the peer as a datagram of `flow`.
/// A reply counts as traffic for the flow's idle timer.
async fn relay_upstream_replies(
    conn: Connection,
    flow: crate::udp::FlowId,
    upstream: Arc<UdpSocket>,
    (flows, fault): (UdpFlowTable, Fault),
    stats: Arc<crate::udp::UdpStats>,
) {
    use crate::udp::{send_decision, DropReason, SendDecision};
    let mut buf = vec![0u8; 65535];
    loop {
        let n = match upstream.recv(&mut buf).await {
            Ok(n) => n,
            // A connected UDP socket reports an ICMP error from the target
            // (for example port unreachable while the target is not up yet)
            // on a later call. The datagram is lost; the flow stays.
            Err(error) if crate::udp::is_transient_recv_error(&error) => {
                tracing::debug!(flow = flow.0, "udp upstream receive failed: {error}");
                continue;
            }
            Err(error) => {
                tracing::warn!(flow = flow.0, "udp upstream receive ended: {error}");
                break;
            }
        };
        let now = std::time::Instant::now();
        if flow_table_section(&flows, &fault, |flows| flows.refresh(flow, now)).is_none() {
            return;
        }
        let max = conn.max_datagram_size();
        let limit = crate::udp::payload_limit(max);
        stats.set_datagram_limit(limit);
        match send_decision(max, n) {
            SendDecision::Send => {}
            SendDecision::Drop(DropReason::LimitShrink) => {
                stats.record_drop(DropReason::LimitShrink);
                tracing::warn!(flow = flow.0, "datagrams unusable, dropping reply");
                continue;
            }
            SendDecision::Drop(reason) => {
                // Only an oversize decision reaches here, which needs a usable limit.
                stats.record_drop(reason);
                tracing::warn!(
                    flow = flow.0,
                    size = n,
                    limit = limit.unwrap_or_default(),
                    "dropping oversize udp reply"
                );
                continue;
            }
        }
        #[expect(
            clippy::indexing_slicing,
            reason = "invariant: recv never reports more bytes than the buffer holds"
        )]
        let reply = &buf[..n];
        if let Err(error) = conn.send_datagram(crate::udp::encode(flow, reply)) {
            tracing::debug!(flow = flow.0, "udp reply not sent: {error}");
        }
    }
}

/// Abort/panic-safe status accounting, including tasks not yet first polled.
struct ActiveStream(Arc<AtomicU64>);

impl ActiveStream {
    fn new(counter: Arc<AtomicU64>) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        Self(counter)
    }
}

impl Drop for ActiveStream {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serve::bind_serve;
    use crate::settings::{NodeSettings, ServeServiceSettings};

    /// A failed upstream connect names the target and keeps the OS error.
    #[tokio::test]
    async fn open_upstream_failure_names_the_target() {
        // macOS and Windows refuse a udp connect to port 0, so CI on those runners
        // asserts this; Linux accepts it, and there is nothing to check there.
        let Err(err) = open_upstream("0.0.0.0:0", &UpstreamResolver::default()).await else {
            return;
        };
        let rendered = format!("{err:#}");
        assert!(rendered.contains("0.0.0.0:0"), "{rendered}");
        assert!(
            err.chain()
                .any(<dyn std::error::Error>::is::<std::io::Error>),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn connect_local_timeout_resets_only_that_channel() {
        let mut channels = JoinSet::new();
        channels.spawn(async {
            let hung = std::future::pending::<std::io::Result<u8>>();
            connect_local(hung, Duration::from_millis(50), "t:1", 1).await
        });
        channels.spawn(async {
            connect_local(
                std::future::ready(Ok(7u8)),
                Duration::from_millis(50),
                "t:1",
                2,
            )
            .await
        });
        channels.spawn(async {
            let refused =
                std::future::ready(Err::<u8, _>(std::io::ErrorKind::ConnectionRefused.into()));
            connect_local(refused, Duration::from_millis(50), "t:1", 3).await
        });
        let mut results = Vec::new();
        while let Some(result) = channels.join_next().await {
            results.push(result.unwrap());
        }
        results.sort();
        assert_eq!(results, vec![None, None, Some(7)]);
    }

    fn peer() -> iroh::EndpointId {
        iroh::SecretKey::generate().public()
    }

    #[test]
    fn track_refcounts_connections_until_the_last_untrack() {
        let tracker = PeerTracker::new(Fault::detached());
        let p = peer();
        let first = tracker.track(p, "echo").unwrap();
        let second = tracker.track(p, "echo").unwrap();

        let expected = vec![(p, vec!["echo".to_string()])];
        assert_eq!(tracker.snapshot().unwrap(), expected);

        drop(first);
        assert_eq!(
            tracker.snapshot().unwrap(),
            expected,
            "one connection left — the entry must survive"
        );

        drop(second);
        assert!(
            tracker.snapshot().unwrap().is_empty(),
            "peer must disappear with its last connection"
        );
    }

    #[test]
    fn guard_untracks_via_drop_without_any_explicit_call() {
        // The panic-safety contract: nothing calls `untrack` manually —
        // dropping the guard (as an unwind would) must clean up.
        let tracker = PeerTracker::new(Fault::detached());
        let p = peer();
        {
            let _tracked = tracker.track(p, "echo").unwrap();
            assert_eq!(
                tracker.snapshot().unwrap(),
                vec![(p, vec!["echo".to_string()])]
            );
        }
        assert!(tracker.snapshot().unwrap().is_empty());
    }

    #[test]
    fn services_merge_across_a_peers_connections() {
        let tracker = PeerTracker::new(Fault::detached());
        let p = peer();
        let web = tracker.track(p, "web").unwrap();
        let echo = tracker.track(p, "echo").unwrap();

        // Service names sorted within the peer's row.
        assert_eq!(
            tracker.snapshot().unwrap(),
            vec![(p, vec!["echo".to_string(), "web".to_string()])]
        );

        drop(web);
        assert_eq!(
            tracker.snapshot().unwrap(),
            vec![(p, vec!["echo".to_string()])]
        );
        drop(echo);
        assert!(tracker.snapshot().unwrap().is_empty());
    }

    #[test]
    fn a_poisoned_tracker_reports_errors_instead_of_panicking() {
        let tracker = PeerTracker::new(Fault::detached());
        let p = peer();
        let held = tracker.track(p, "echo").unwrap();
        let poisoner = tracker.clone();
        let _ = std::thread::spawn(move || {
            poisoner
                .registry
                .with(|_| panic!("poison the tracker"))
                .ok();
        })
        .join();

        assert!(tracker.track(p, "web").is_err(), "track must fail");
        assert!(tracker.snapshot().is_err(), "snapshot must fail");
        // Untracking through the guard must not panic either.
        drop(held);
    }

    #[test]
    fn snapshot_orders_peers_deterministically() {
        let tracker = PeerTracker::new(Fault::detached());
        let peers: Vec<_> = (0..3).map(|_| peer()).collect();
        // Hold every guard so all peers stay tracked.
        let guards: Vec<_> = peers
            .iter()
            .map(|p| tracker.track(*p, "echo").unwrap())
            .collect();

        let mut expected: Vec<String> = peers.iter().map(ToString::to_string).collect();
        expected.sort();
        let got: Vec<String> = tracker
            .snapshot()
            .unwrap()
            .into_iter()
            .map(|(p, _)| p.to_string())
            .collect();
        assert_eq!(got, expected);
        drop(guards);
    }

    // --- status publishing characterization (flush loop + initial write) ---

    type SaveLog = Arc<std::sync::Mutex<Vec<ServeObservation>>>;

    fn unreachable_relay_settings(services: Vec<ServeServiceSettings>) -> ServeSettings {
        ServeSettings {
            node: NodeSettings {
                relay_urls: vec!["https://relay.invalid".to_string()],
                ..Default::default()
            },
            services,
        }
    }

    fn echo_service() -> ServeServiceSettings {
        ServeServiceSettings {
            name: "echo".to_string(),
            protocol: Protocol::Tcp,
            host: "127.0.0.1".to_string(),
            port: 9,
            udp: UdpLimits::default(),
        }
    }

    async fn local_endpoint() -> iroh::Endpoint {
        bind_serve(&unreachable_relay_settings(Vec::new()))
            .await
            .expect("bind local endpoint")
    }

    fn empty_snapshot() -> StatusSnapshot {
        StatusSnapshot {
            node_id: "flush-test".to_string(),
            pid: 1,
            started_at: 0,
            services: Vec::new(),
        }
    }

    /// Runs the flush loop for ~3 ticks of virtual time, then stops it and
    /// returns the files `save` was called with. `fail_first` makes the
    /// first save return `Err`.
    async fn run_flush_for_three_ticks(
        last: Option<ServeObservation>,
        fail_first: bool,
    ) -> Vec<ServeObservation> {
        let ep = local_endpoint().await;
        tokio::time::pause();
        let interval = Duration::from_secs(5);
        let log: SaveLog = Arc::default();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let log_for_save = log.clone();
        let sink = StatusSink::new(move |observation| {
            let log = log_for_save.clone();
            Box::pin(async move {
                let mut log = log.lock().expect("log lock");
                log.push(observation);
                if fail_first && log.len() == 1 {
                    Err(anyhow::anyhow!("first save fails"))
                } else {
                    Ok(())
                }
            })
        });
        let flush = tokio::spawn(status_flush_loop(
            empty_snapshot(),
            ep,
            PeerTracker::new(Fault::detached()),
            sink,
            last,
            stop_rx,
            interval,
        ));
        tokio::time::sleep(interval * 3 + Duration::from_secs(1)).await;
        let _ = stop_tx.send(());
        flush.await.expect("flush task must not panic");
        let saved = log.lock().expect("log lock").clone();
        saved
    }

    #[tokio::test]
    async fn unchanged_status_is_not_rewritten() {
        let saved = run_flush_for_three_ticks(None, false).await;
        assert_eq!(
            saved.len(),
            1,
            "equal renders must be saved once: {saved:?}"
        );
    }

    #[tokio::test]
    async fn seeded_unchanged_status_is_never_rewritten() {
        let seed = empty_snapshot().render(None, Vec::new());
        let saved = run_flush_for_three_ticks(Some(seed), false).await;
        assert!(saved.is_empty(), "{saved:?}");
    }

    #[tokio::test]
    async fn failed_status_write_is_retried_next_tick() {
        let saved = run_flush_for_three_ticks(None, true).await;
        assert_eq!(
            saved.len(),
            2,
            "failure retried once, then settled: {saved:?}"
        );
        assert_eq!(saved[0], saved[1], "the retry saves the same file");
    }

    /// Strict ordering: the initial save completes BEFORE the accept loop
    /// runs. The recording sink holds the first save open; while it is held,
    /// a real client must not get through the handshake (nothing accepts).
    /// Releasing the save lets the same client connect.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn initial_status_saved_before_accept_loop() {
        use iroh::endpoint::presets::Minimal;
        let settings = unreachable_relay_settings(vec![echo_service()]);
        let alpn = proto::alpn_for("echo");
        let ep = bind_serve(&settings).await.expect("bind serve endpoint");
        let mut addr = iroh::EndpointAddr::new(ep.id());
        for sock in ep.bound_sockets() {
            let ip = if sock.ip().is_unspecified() {
                std::net::Ipv4Addr::LOCALHOST.into()
            } else {
                sock.ip()
            };
            addr = addr.with_ip_addr((ip, sock.port()).into());
        }
        let client = iroh::Endpoint::builder(Minimal)
            .relay_mode(iroh::endpoint::RelayMode::Disabled)
            .bind()
            .await
            .expect("bind client");

        let events: Arc<std::sync::Mutex<Vec<&'static str>>> = Arc::default();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel::<()>();
        let entered_tx = Arc::new(std::sync::Mutex::new(Some(entered_tx)));
        let release = Arc::new(tokio::sync::Notify::new());
        let sink = {
            let events = events.clone();
            let release = release.clone();
            StatusSink::new(move |_observation: ServeObservation| {
                let events = events.clone();
                let release = release.clone();
                let entered_tx = entered_tx.clone();
                Box::pin(async move {
                    let first = entered_tx.lock().expect("entered tx lock").take();
                    if let Some(tx) = first {
                        events.lock().expect("events lock").push("save-started");
                        let _ = tx.send(());
                        release.notified().await;
                        events.lock().expect("events lock").push("save-finished");
                    }
                    Ok(())
                })
            })
        };

        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let role = tokio::spawn(async move {
            let hooks = RoleHooks {
                on_ready: Box::new(|_| Ok(())),
                status: Some(sink),
            };
            run_serve(ep, settings, RunContext::new(), hooks, async move {
                let _ = stop_rx.await;
            })
            .await
        });

        entered_rx.await.expect("initial save must start");
        let early = tokio::time::timeout(
            Duration::from_millis(500),
            client.connect(addr.clone(), &alpn),
        )
        .await;
        assert!(
            early.is_err(),
            "nothing may be admitted while the initial save is in flight"
        );

        release.notify_one();
        let conn = tokio::time::timeout(Duration::from_secs(10), client.connect(addr, &alpn))
            .await
            .expect("admission opens once the initial save finished")
            .expect("connect after initial save");
        assert_eq!(
            *events.lock().expect("events lock"),
            ["save-started", "save-finished"]
        );
        drop(conn);

        let _ = stop_tx.send(());
        let completion = role.await.expect("role task");
        completion.result.expect("role result");
        assert!(completion.status_quiesced);
        client.close().await;
    }

    /// Serve closes a UDP connection whose access peer cannot carry
    /// datagrams: the peer sees the dedicated close code and reason, and
    /// no flow state is created.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn serve_closes_udp_connection_when_access_peer_cannot_carry_datagrams() {
        use iroh::endpoint::presets::Minimal;
        use iroh::endpoint::{ConnectionError, QuicTransportConfig, RelayMode, VarInt};
        let service = ServeServiceSettings {
            name: "udp-echo".to_string(),
            protocol: Protocol::Udp,
            host: "127.0.0.1".to_string(),
            port: 9,
            udp: UdpLimits::default(),
        };
        let alpn = proto::alpn_for("udp-echo");
        let settings = unreachable_relay_settings(vec![service]);
        let ep = bind_serve(&settings).await.expect("bind serve endpoint");
        let mut addr = iroh::EndpointAddr::new(ep.id());
        for sock in ep.bound_sockets() {
            let ip = if sock.ip().is_unspecified() {
                std::net::Ipv4Addr::LOCALHOST.into()
            } else {
                sock.ip()
            };
            addr = addr.with_ip_addr((ip, sock.port()).into());
        }
        // The access peer disables datagrams, so Serve sees no datagram size.
        let client = iroh::Endpoint::builder(Minimal)
            .relay_mode(RelayMode::Disabled)
            .transport_config(
                QuicTransportConfig::builder()
                    .datagram_receive_buffer_size(None)
                    .build(),
            )
            .bind()
            .await
            .expect("bind client");

        let target = ServiceTarget::new(
            "127.0.0.1:9".to_string(),
            Protocol::Udp,
            UdpLimits::default(),
            Fault::detached(),
        );
        let (_stop_tx, stop) = shutdown::channel();
        let sinks = shutdown::QuarantineSinks::new(Fault::detached());
        // Keep an endpoint handle alive past the Serve task: handle_udp_connection
        // returns right after the close, and dropping the last endpoint would tear
        // down before the close frame reaches the client.
        let server = ep.clone();
        let serve = tokio::spawn(async move {
            let incoming = ep.accept().await.expect("incoming connection");
            let conn = incoming.await.expect("handshake");
            handle_udp_connection(&conn, target, &sinks, stop).await;
        });

        let conn = tokio::time::timeout(Duration::from_secs(10), client.connect(addr, &alpn))
            .await
            .expect("connect within timeout")
            .expect("connect");
        // Serve's task ends right after it closes the connection; awaiting it
        // mirrors the production lifetime before the client reads the close.
        tokio::time::timeout(Duration::from_secs(10), serve)
            .await
            .expect("serve task must finish")
            .expect("serve task must not panic");
        let reason = tokio::time::timeout(Duration::from_secs(10), conn.closed())
            .await
            .expect("serve must close the connection");
        match reason {
            ConnectionError::ApplicationClosed(close) => {
                // Wire-contract literals, independent of the production constants.
                assert_eq!(close.error_code, VarInt::from_u32(0x1d47));
                assert_eq!(close.reason.as_ref(), b"udp datagrams unsupported");
            }
            other => panic!("expected dedicated application close, got {other:?}"),
        }
        client.close().await;
        server.close().await;
    }

    /// A Serve endpoint whose UDP connections all run `handle_udp_connection`
    /// against one shared `target`, as the accept loop does.
    struct UdpServeHarness {
        server: iroh::Endpoint,
        addr: iroh::EndpointAddr,
        alpn: Vec<u8>,
        stats: Arc<crate::udp::UdpStats>,
        peer_slots: Arc<crate::udp::PeerFlowSlots>,
        stop_tx: tokio::sync::watch::Sender<Option<tokio::time::Instant>>,
        accept: tokio::task::JoinHandle<()>,
    }

    impl UdpServeHarness {
        async fn start(limits: UdpLimits) -> Self {
            let target = ServiceTarget::new(
                "127.0.0.1:9".to_string(),
                Protocol::Udp,
                limits,
                Fault::detached(),
            );
            Self::start_with(target).await
        }

        /// Serve `target`; the accept task joins its connections on stop.
        async fn start_with(target: ServiceTarget) -> Self {
            let service = ServeServiceSettings {
                name: "udp-cap".to_string(),
                protocol: Protocol::Udp,
                host: "127.0.0.1".to_string(),
                port: 9,
                udp: target.udp_limits,
            };
            let alpn = proto::alpn_for(&service.name);
            let server = bind_serve(&unreachable_relay_settings(vec![service]))
                .await
                .expect("bind serve endpoint");
            let addr = loopback_addr(&server);
            let stats = target.udp_stats.clone();
            let peer_slots = target.udp_peer_slots.clone();
            let (stop_tx, stop) = shutdown::channel();
            let ep = server.clone();
            let accept = tokio::spawn(async move {
                let sinks = Arc::new(shutdown::QuarantineSinks::new(Fault::detached()));
                let mut conns = JoinSet::new();
                let mut admission = stop.clone();
                loop {
                    let incoming = tokio::select! {
                        _ = shutdown::stopped(&mut admission) => break,
                        incoming = ep.accept() => incoming,
                    };
                    let Some(incoming) = incoming else { break };
                    let Ok(conn) = incoming.await else { continue };
                    let (target, sinks, stop) = (target.clone(), sinks.clone(), stop.clone());
                    conns.spawn(async move {
                        handle_udp_connection(&conn, target, &sinks, stop).await;
                    });
                }
                shutdown::join(&mut conns).await;
            });
            Self {
                server,
                addr,
                alpn,
                stats,
                peer_slots,
                stop_tx,
                accept,
            }
        }

        async fn client() -> iroh::Endpoint {
            iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
                .relay_mode(iroh::endpoint::RelayMode::Disabled)
                .bind()
                .await
                .expect("bind client")
        }

        async fn connect(&self, client: &iroh::Endpoint) -> Connection {
            tokio::time::timeout(
                Duration::from_secs(10),
                client.connect(self.addr.clone(), &self.alpn),
            )
            .await
            .expect("connect within timeout")
            .expect("connect")
        }

        /// Wait until the service counters satisfy `done`.
        async fn stats_when(
            &self,
            done: impl Fn(&crate::observation::UdpObservation) -> bool,
        ) -> crate::observation::UdpObservation {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            loop {
                let snap = self.stats.snapshot();
                if done(&snap) {
                    return snap;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "udp stats never reached the expected state: {snap:?}"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }

        async fn stop(self) {
            self.accept.abort();
            self.server.close().await;
        }
    }

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

    fn send_flow(conn: &Connection, flow: u32) {
        conn.send_datagram(crate::udp::encode(crate::udp::FlowId(flow), b"x"))
            .expect("send datagram");
    }

    /// A UDP target that echoes every datagram back to its sender.
    async fn udp_echo_target() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        let socket = UdpSocket::bind("127.0.0.1:0").await.expect("bind echo");
        let addr = socket.local_addr().expect("echo addr");
        let echo = tokio::spawn(async move {
            let mut buf = vec![0u8; 2048];
            while let Ok((n, from)) = socket.recv_from(&mut buf).await {
                let _ = socket.send_to(&buf[..n], from).await;
            }
        });
        (addr, echo)
    }

    /// A resolver answering the n-th open (0-based) with `answer(n)` at once,
    /// or never when `answer(n)` is `None` (a hung resolver).
    fn scripted_resolver(
        answer: impl Fn(usize) -> Option<std::io::Result<Vec<std::net::SocketAddr>>>
            + Send
            + Sync
            + 'static,
    ) -> UpstreamResolver {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        UpstreamResolver(Arc::new(move |_local_addr: String| {
            let call = calls.fetch_add(1, Ordering::Relaxed);
            let answer = answer(call);
            Box::pin(async move {
                match answer {
                    Some(result) => result,
                    None => std::future::pending().await,
                }
            })
        }))
    }

    async fn read_flow_reply(conn: &Connection, flow: u32) {
        loop {
            let datagram = conn.read_datagram().await.expect("read reply");
            let (got, _) = crate::udp::decode(&datagram).expect("decode reply");
            if got.0 == flow {
                return;
            }
        }
    }

    /// #219: while one flow's target address is still resolving, an already
    /// live flow on the same connection keeps relaying without waiting.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_flow_still_opening_does_not_stall_a_live_flow() {
        let (echo_addr, echo) = udp_echo_target().await;
        let mut target = ServiceTarget::new(
            echo_addr.to_string(),
            Protocol::Udp,
            UdpLimits::default(),
            Fault::detached(),
        );
        // The first open (flow 0) resolves at once; the second (flow 1)
        // never does, as with a hung resolver.
        target.udp_resolver =
            scripted_resolver(move |call| (call == 0).then(|| Ok(vec![echo_addr])));
        let serve = UdpServeHarness::start_with(target).await;
        let client = UdpServeHarness::client().await;
        let conn = serve.connect(&client).await;

        send_flow(&conn, 0);
        tokio::time::timeout(Duration::from_secs(5), read_flow_reply(&conn, 0))
            .await
            .expect("flow 0 opens");

        send_flow(&conn, 1);
        // Flow 1 holds its slot once Serve has started opening it.
        serve.stats_when(|s| s.active_flows == 2).await;
        send_flow(&conn, 0);
        // Flow 1's resolver never answers, so any reply proves no wait on it.
        tokio::time::timeout(Duration::from_secs(1), read_flow_reply(&conn, 0))
            .await
            .expect("live flow 0 must keep relaying while flow 1 opens");

        client.close().await;
        serve.stop().await;
        echo.abort();
    }

    /// #219: a flow whose open fails gives back both cap reservations and
    /// counts its datagram as `cap`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_open_releases_its_cap_reservations() {
        let limits = UdpLimits {
            max_flows: 1,
            max_flows_per_peer: 1,
            ..UdpLimits::default()
        };
        let (echo_addr, echo) = udp_echo_target().await;
        let mut target = ServiceTarget::new(
            echo_addr.to_string(),
            Protocol::Udp,
            limits,
            Fault::detached(),
        );
        target.udp_resolver = scripted_resolver(move |call| {
            Some(if call == 0 {
                Err(std::io::ErrorKind::NotFound.into())
            } else {
                Ok(vec![echo_addr])
            })
        });
        let serve = UdpServeHarness::start_with(target).await;
        let client = UdpServeHarness::client().await;
        let conn = serve.connect(&client).await;

        send_flow(&conn, 0);
        let snap = serve.stats_when(|s| s.drops.cap == 1).await;
        assert_eq!(snap.active_flows, 0, "a refused flow holds no service slot");
        // The single slot of the service and of the peer is free again.
        send_flow(&conn, 1);
        tokio::time::timeout(Duration::from_secs(5), read_flow_reply(&conn, 1))
            .await
            .expect("the next flow opens in the released slot");
        assert_eq!(serve.stats.snapshot().drops.cap, 1);

        client.close().await;
        serve.stop().await;
        echo.abort();
    }

    /// #219: stopping while a flow is still opening ends the connection's
    /// task within the drain grace and releases the flow's reservations.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_during_an_open_finishes_within_the_drain_grace() {
        let mut target = ServiceTarget::new(
            "127.0.0.1:9".to_string(),
            Protocol::Udp,
            UdpLimits::default(),
            Fault::detached(),
        );
        target.udp_resolver = scripted_resolver(|_| None);
        let serve = UdpServeHarness::start_with(target).await;
        let client = UdpServeHarness::client().await;
        let conn = serve.connect(&client).await;

        send_flow(&conn, 0);
        serve.stats_when(|s| s.active_flows == 1).await;
        let peer_slots = serve.peer_slots.clone();
        let stats = serve.stats.clone();
        shutdown::begin(&serve.stop_tx, &RunContext::new());
        let UdpServeHarness { server, accept, .. } = serve;
        tokio::time::timeout(shutdown::DRAIN_GRACE, accept)
            .await
            .expect("connection tasks must end within the drain grace")
            .expect("accept task must not panic");
        assert_eq!(stats.snapshot().active_flows, 0);
        assert!(
            peer_slots.try_reserve(client.id(), 1).unwrap().is_some(),
            "the opening flow's peer slot must be released"
        );
        drop(conn);
        client.close().await;
        server.close().await;
    }

    /// #218: the per-peer cap counts a peer's live flows across all of its
    /// connections to the service, refuses the flow over it as `cap`, leaves
    /// other peers unaffected, and frees the peer's slots when its
    /// connections end.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn per_peer_cap_spans_every_connection_of_the_peer() {
        let limits = UdpLimits {
            max_flows: 4,
            max_flows_per_peer: 1,
            ..UdpLimits::default()
        };
        let serve = UdpServeHarness::start(limits).await;
        let first = UdpServeHarness::client().await;
        let other = UdpServeHarness::client().await;

        let conn_a = serve.connect(&first).await;
        send_flow(&conn_a, 0);
        serve.stats_when(|s| s.active_flows == 1).await;

        // Same peer, second connection: its share is already used.
        let conn_b = serve.connect(&first).await;
        send_flow(&conn_b, 0);
        let snap = serve.stats_when(|s| s.drops.cap >= 1).await;
        assert_eq!(snap.active_flows, 1, "the peer must stay at its cap");

        // Another peer is still served from the shared service cap.
        let conn_other = serve.connect(&other).await;
        send_flow(&conn_other, 0);
        serve.stats_when(|s| s.active_flows == 2).await;

        // The first peer's connections end: its live flow is quarantined
        // and its peer slot is free again for a new connection.
        conn_a.close(VarInt::from_u32(0), b"done");
        conn_b.close(VarInt::from_u32(0), b"done");
        serve
            .stats_when(|s| s.active_flows == 1 && s.quarantined_flows == 1)
            .await;
        let conn_c = serve.connect(&first).await;
        send_flow(&conn_c, 0);
        let snap = serve.stats_when(|s| s.active_flows == 2).await;
        assert_eq!(
            snap.drops.cap, 1,
            "the reconnected peer must not be refused"
        );

        first.close().await;
        other.close().await;
        serve.stop().await;
    }

    async fn quarantined_upstream(
        stats: &Arc<crate::udp::UdpStats>,
        stop: &shutdown::Stop,
        sinks: &shutdown::QuarantineSinks,
    ) {
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let idle = || tokio::spawn(std::future::pending::<()>()).abort_handle();
        let (queue, _rx) = crate::udp::FlowQueue::new(1);
        let reservation = FlowReservation {
            peer: Arc::new(crate::udp::PeerFlowSlots::new(Fault::detached()))
                .try_reserve(peer(), 1)
                .unwrap()
                .expect("peer slot"),
            service: ServiceFlowSlot::try_reserve(stats, usize::MAX).expect("service slot"),
        };
        let upstream = UdpUpstream {
            socket,
            queue,
            relay: idle(),
            writer: idle(),
            reservation,
        };
        quarantine(upstream, stats, stop, sinks);
    }

    /// #226: a poisoned flow table is never recovered; it raises the fault of
    /// its own connection only.
    #[test]
    fn a_poisoned_flow_table_raises_its_connection_fault() {
        let table = UdpFlowTable::default();
        let poisoner = table.clone();
        let panicked = std::thread::spawn(move || {
            poisoner.with(|_| panic!("poison the flow table")).ok();
        })
        .join();
        assert!(panicked.is_err());
        let (fault, mut faults) = fault::channel();
        assert!(flow_table_section(&table, &fault, |flows| flows.len()).is_none());
        let raised = faults.take().expect("the connection fault is raised");
        assert!(
            format!("{raised:#}").contains("udp flow table"),
            "{raised:#}"
        );
    }

    /// #226: once the sink set is poisoned a new sink is not tracked: it is
    /// dropped unpolled, releasing its slot, and the role fault is raised.
    #[tokio::test(start_paused = true)]
    async fn poisoned_quarantine_sinks_drop_new_sinks_and_raise_a_role_fault() {
        let stats = Arc::new(crate::udp::UdpStats::default());
        let (fault, mut faults) = fault::channel();
        let sinks = Arc::new(shutdown::QuarantineSinks::new(fault));
        let poisoner = sinks.clone();
        let panicked = std::thread::spawn(move || poisoner.poison_for_test()).join();
        assert!(panicked.is_err());
        let (_stop, stop_rx) = shutdown::channel();
        quarantined_upstream(&stats, &stop_rx, &sinks).await;
        assert_eq!(stats.snapshot().quarantined_flows, 0, "slot released");
        assert!(faults.take().is_some(), "the role fault is raised");
    }

    #[tokio::test(start_paused = true)]
    async fn quarantine_sinks_are_joined_on_role_stop() {
        let stats = Arc::new(crate::udp::UdpStats::default());
        let sinks = shutdown::QuarantineSinks::new(Fault::detached());
        let (stop, mut stop_rx) = shutdown::channel();
        let before = tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks();
        for _ in 0..3 {
            quarantined_upstream(&stats, &stop_rx, &sinks).await;
        }
        assert_eq!(stats.snapshot().quarantined_flows, 3);
        shutdown::begin(&stop, &RunContext::new());
        let deadline = shutdown::stopped(&mut stop_rx).await;
        sinks.shutdown(deadline).await;
        tokio::task::yield_now().await;
        let after = tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks();
        assert_eq!(after, before);
        assert_eq!(stats.snapshot().quarantined_flows, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn failed_quarantine_sink_releases_its_slot() {
        let stats = Arc::new(crate::udp::UdpStats::default());
        let sinks = shutdown::QuarantineSinks::new(Fault::detached());
        stats.quarantine_started();
        let slot = QuarantineSlot(stats.clone());
        // Same exit as the real sink on a non-transient recv error: break.
        sinks.spawn(async move {
            let _slot = slot;
            panic!("sink failed");
        });
        tokio::task::yield_now().await;
        sinks.reap();
        assert_eq!(stats.snapshot().quarantined_flows, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn quarantine_sink_outlives_its_connection() {
        let stats = Arc::new(crate::udp::UdpStats::default());
        let sinks = shutdown::QuarantineSinks::new(Fault::detached());
        let (_stop, stop_rx) = shutdown::channel();
        {
            let mut connection = JoinSet::new();
            connection.spawn(async {
                std::future::pending::<()>().await;
            });
            quarantined_upstream(&stats, &stop_rx, &sinks).await;
            drop(connection);
        }
        tokio::time::sleep(crate::udp::QUARANTINE / 2).await;
        sinks.reap();
        assert_eq!(stats.snapshot().quarantined_flows, 1);
        tokio::time::sleep(crate::udp::QUARANTINE).await;
        sinks.reap();
        assert_eq!(stats.snapshot().quarantined_flows, 0);
    }

    /// Spawn a stand-in flush task and run the shared stop+join on it.
    async fn quiesced_after(body: impl Future<Output = ()> + Send + 'static) -> RunCompletion {
        let (stop_tx, _stop_rx) = tokio::sync::oneshot::channel::<()>();
        let flush = tokio::spawn(body);
        RunCompletion {
            result: Ok(()),
            status_quiesced: support::stop_flush(stop_tx, flush).await,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn run_completion_reports_unquiesced_writer() {
        // A save blocked past the 5 s join bound: not quiesced.
        let blocked = quiesced_after(async {
            tokio::time::sleep(Duration::from_secs(6)).await;
        })
        .await;
        assert!(!blocked.status_quiesced);
        // A fast save: quiesced.
        let fast = quiesced_after(async {}).await;
        assert!(fast.status_quiesced);
        // A flush task that panics did not return Ok: not quiesced.
        let panicked = quiesced_after(async { panic!("flush failed") }).await;
        assert!(!panicked.status_quiesced);
    }
}
