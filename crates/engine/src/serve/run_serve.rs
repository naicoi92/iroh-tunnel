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
use tokio::task::JoinSet;

use crate::endpoint;
use crate::observation::{
    PeerConnectionObservation, ServeObservation, ServiceObservation, STATUS_FLUSH_INTERVAL,
};
use crate::proto;
use crate::runtime::support;
use crate::runtime::{ReadyReport, ReadyServices, ServeReady};
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

    // Build the ALPN -> target lookup for demultiplexing accepted
    // streams.
    let mut targets: HashMap<Vec<u8>, ServiceTarget> = HashMap::new();
    for svc in &cfg.services {
        let target =
            ServiceTarget::new(format!("{}:{}", svc.host, svc.port), svc.protocol, svc.udp);
        targets.insert(proto::alpn_for(&svc.name), target);
    }

    // Operator-facing status snapshot (T-13), refreshed by the flush task
    // below as stream counts change. Best-effort: a failure to write
    // status is logged but does not stop the tunnel. Built before the
    // accept task takes ownership of `targets`.
    let status = StatusSnapshot {
        node_id: endpoint::node_id_string(&ep),
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
                protocol: s.protocol.as_str().to_string(),
                local_addr: support::format_local_addr(&s.host, s.port),
                active_streams: targets
                    .get(&proto::alpn_for(&s.name))
                    .map(|t| t.active_streams())
                    .unwrap_or_default(),
                udp_stats: (s.protocol == Protocol::Udp)
                    .then(|| {
                        targets
                            .get(&proto::alpn_for(&s.name))
                            .map(|t| t.udp_stats.clone())
                    })
                    .flatten(),
            })
            .collect(),
    };
    let initial = status.render(current_home_relay(&ep), Vec::new());
    // Seed the flush loop's change detection with the initial write when
    // it succeeded — otherwise the first tick would rewrite an identical
    // idle snapshot. A failed initial write seeds `None`, so the first
    // tick retries it. Without a sink nothing is published.
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

    // Live registry of connected peers, fed by the accept loop below and
    // read by the status flush task (issue #57).
    let peers = PeerTracker::default();
    tracing::info!("serve endpoint ready, accepting connections");
    let accept_ep = ep.clone();
    let accept_peers = peers.clone();
    let (stop, stop_rx) = shutdown::channel();
    let sinks = Arc::new(shutdown::QuarantineSinks::default());
    let accept_sinks = sinks.clone();
    let mut accept = JoinSet::new();
    accept.spawn(async move {
        accept_loop(&accept_ep, targets, accept_peers, accept_sinks, stop_rx).await;
    });

    // Cooperative stop channel for the flush task.
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();

    // Refresh serve-status.json when the rendered snapshot changes —
    // stream counters, connected peers, or their transport states — at
    // most once per STATUS_FLUSH_INTERVAL: no disk churn under busy
    // stream churn, still near-live for operators.
    let flush = sink.map(|sink| {
        tokio::spawn(status_flush_loop(
            status,
            ep.clone(),
            peers,
            sink,
            seeded,
            stop_rx,
            STATUS_FLUSH_INTERVAL,
        ))
    });

    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => break,
            Some(result) = accept.join_next(), if !accept.is_empty() => shutdown::observe(result),
        }
    }
    shutdown::begin(&stop, &ctx);
    // The accept supervisor seals handshakes and connection admission;
    // its connection children seal stream admission and join pipe leaves.
    shutdown::join(&mut accept).await;
    // Quarantine sinks outlive their connections but not the role.
    let drain_deadline = (*stop.borrow()).unwrap_or_else(tokio::time::Instant::now);
    sinks.shutdown(drain_deadline).await;
    // Stop the flush task and bound its join; the run owner removes the
    // status file only when this reports the writer quiesced.
    let status_quiesced = match flush {
        Some(flush) => support::stop_flush(stop_tx, flush).await,
        None => true,
    };
    ep.close().await;
    RunCompletion {
        result: Ok(()),
        status_quiesced,
    }
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
}

impl ServiceTarget {
    fn new(local_addr: String, protocol: Protocol, udp_limits: UdpLimits) -> Self {
        Self {
            local_addr,
            protocol,
            udp_limits,
            udp_stats: Arc::default(),
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
#[derive(Clone, Default)]
struct PeerTracker(Arc<std::sync::Mutex<HashMap<iroh::EndpointId, HashMap<String, u64>>>>);

impl PeerTracker {
    /// Record one accepted connection from `peer` for `service`, returning
    /// the RAII handle that untracks it.
    ///
    /// The guard's [`Drop`] decrements the refcount, so a connection task
    /// that returns *or panics* cannot leak the entry — the unwind drops the
    /// guard like any local.
    fn track(&self, peer: iroh::EndpointId, service: &str) -> TrackedPeer {
        // std Mutex is deliberate: the critical section is a few map ops,
        // the guard is never held across an await, and unlock failures can
        // only mean a panic mid-section — unwrapping is the right response.
        self.0
            .lock()
            .unwrap()
            .entry(peer)
            .or_default()
            .entry(service.to_string())
            .and_modify(|n| *n += 1)
            .or_insert(1);
        TrackedPeer {
            peers: self.clone(),
            peer,
            service: service.to_string(),
        }
    }

    /// Drop one connection from `peer` for `service`; removes the service
    /// (and with it the peer) when its refcount hits zero.
    fn untrack(&self, peer: iroh::EndpointId, service: &str) {
        let mut guard = self.0.lock().unwrap();
        let Some(services) = guard.get_mut(&peer) else {
            return;
        };
        let Some(count) = services.get_mut(service) else {
            return;
        };
        *count -= 1;
        if *count == 0 {
            services.remove(service);
            if services.is_empty() {
                guard.remove(&peer);
            }
        }
    }

    /// Point-in-time copy of the registry: `(peer, sorted service names)`,
    /// sorted by peer id for deterministic output.
    fn snapshot(&self) -> Vec<(iroh::EndpointId, Vec<String>)> {
        let mut peers: Vec<_> = self
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|(peer, services)| {
                let mut names: Vec<String> = services.keys().cloned().collect();
                names.sort();
                (*peer, names)
            })
            .collect();
        // Cached key: the node-id string is computed once per peer instead
        // of once per comparison.
        peers.sort_by_cached_key(|(peer, _)| peer.to_string());
        peers
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
        self.peers.untrack(self.peer, &self.service);
    }
}

/// Render the tracker into the status file's `connections` array: one row
/// per tracked peer with a fresh [`crate::conn_path::peer_path_report`]
/// snapshot.
///
/// Peers whose iroh remote-map entry has already expired (`remote_info` →
/// `None`) are skipped — the flush cadence picks them back up if they
/// reconnect.
async fn render_connections(
    ep: &iroh::Endpoint,
    peers: &PeerTracker,
) -> Vec<PeerConnectionObservation> {
    let mut out = Vec::new();
    for (peer, services) in peers.snapshot() {
        let Some(report) = crate::conn_path::peer_path_report(ep, peer).await else {
            continue;
        };
        out.push(PeerConnectionObservation {
            path: report,
            services,
        });
    }
    out
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
        let connections = render_connections(&ep, &peers).await;
        let observation = status.render(current_home_relay(&ep), connections);
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
    let mut connections = JoinSet::new();
    let mut watchers = JoinSet::new();
    loop {
        let incoming = tokio::select! {
            biased;
            _ = shutdown::stopped(&mut stop) => break,
            Some(result) = connections.join_next(), if !connections.is_empty() => {
                shutdown::observe(result);
                continue;
            }
            Some(result) = watchers.join_next(), if !watchers.is_empty() => {
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

        // Demultiplex by the negotiated ALPN to find the service target.
        let alpn = conn.alpn().to_vec();
        let Some(target) = targets.get(&alpn).cloned() else {
            let name = proto::name_from_alpn(&alpn)
                .map(String::from)
                .unwrap_or_else(|| format!("{alpn:02x?}"));
            tracing::warn!("connection with unknown ALPN for service '{name}', dropping");
            continue;
        };

        // The access peer's NodeId, from its cert. Logged on connect and
        // (via the watcher below) on disconnect, so operators can see who is
        // tunneling in and correlate with access-side logs.
        let remote_id = conn.remote_id();
        let name = proto::name_from_alpn(&alpn)
            .map(String::from)
            .unwrap_or_else(|| format!("{alpn:02x?}"));
        // Query the fresh path so the connect line carries it — symmetric
        // with the access role's established lines (`via relay=…`). Right
        // after the handshake iroh may not have an active-path snapshot
        // yet; that renders as `paths pending` and the status file's
        // `connections` array catches up on its next flush.
        let report = tokio::select! {
            biased;
            _ = shutdown::stopped(&mut stop) => break,
            report = crate::conn_path::peer_path_report(ep, remote_id) => report,
        };
        tracing::info!(
            peer = %remote_id,
            service = %name,
            "peer connected via {}",
            crate::conn_path::render_established_paths(report.as_ref())
        );

        // Watcher: emit a disconnect line when the QUIC connection closes. The
        // weak handle is registered while `conn` is still alive (before the
        // stream-handling task takes it), so iroh guarantees the close event is
        // delivered even if the connection drops before this resolves.
        watchers.spawn(support::disconnect_watcher(
            &conn,
            remote_id.to_string(),
            format!("peer disconnected (service {name})"),
        ));
        let conn_peers = peers.clone();
        let conn_stop = stop.clone();
        let conn_sinks = sinks.clone();

        connections.spawn(async move {
            // Track the peer for the status file's `connections` array for
            // as long as this connection lives (issue #57). The guard
            // untracks on drop — normal return or panic unwind alike — so
            // the refcount can never leak.
            let _tracked = conn_peers.track(remote_id, &name);
            handle_connection(&conn, target, &conn_sinks, conn_stop).await;
        });
    }
    // Supervisors are not aborted: each joins its own stream children using
    // the same absolute deadline. Idle connections do not consume grace.
    shutdown::join(&mut connections).await;
    watchers.abort_all();
    shutdown::join(&mut watchers).await;
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
        let local_addr = target.local_addr.clone();
        streams.spawn(async move {
            let _counter = counter;
            // Connect inside the stream's own task: a slow target must not
            // hold up accepting other streams on this connection. A failed
            // or timed-out connect closes only this channel (dropping the
            // stream halves resets it).
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
            // AsyncWrite::shutdown only queues QUIC FIN. Keep channel owned
            // until final bytes are acknowledged, or last-pipe completion
            // could close the endpoint and discard its queued response. The
            // wait is bounded: a vanished peer must not pin this task and
            // the active_streams counter.
            let delivered = send.stopped();
            match crate::tcp::pipe_tcp_bidirectional(local, (recv, send)).await {
                Ok(()) => {
                    match crate::tcp::await_delivery(delivered, crate::tcp::DELIVERY_ACK_GRACE)
                        .await
                    {
                        crate::tcp::Delivery::Acknowledged => {
                            tracing::debug!("stream closed normally")
                        }
                        crate::tcp::Delivery::PeerStopped(code) => {
                            tracing::debug!(%code, "peer stopped completed stream")
                        }
                        crate::tcp::Delivery::Unconfirmed => tracing::warn!(
                            stream = stream_no,
                            "peer lost: no delivery acknowledgement within grace"
                        ),
                        crate::tcp::Delivery::Lost(error) => {
                            tracing::warn!("stream delivery error: {error}")
                        }
                    }
                }
                Err(error) => tracing::warn!("stream error: {error}"),
            }
        });
    };
    if let Some(deadline) = deadline {
        shutdown::drain(&mut streams, deadline).await;
    } else {
        // Peer loss cannot drain and must never replay partially copied data.
        streams.abort_all();
        shutdown::join(&mut streams).await;
    }
}

/// One flow's upstream: its socket, the bounded queue feeding the writer task,
/// and the handles that end the reply relay and the writer.
struct UdpUpstream {
    socket: Arc<UdpSocket>,
    queue: crate::udp::FlowQueue,
    relay: tokio::task::AbortHandle,
    writer: tokio::task::AbortHandle,
}

/// One peer connection's live UDP flows, shared with the reply relays.
type UdpFlowTable = Arc<std::sync::Mutex<crate::udp::PeerFlows<UdpUpstream>>>;

/// Carry one peer's UDP flows over QUIC DATAGRAMs (#88).
///
/// Each flow gets its own connected upstream socket, so a reply is attributed
/// to a flow by the socket it arrives on, never by inspecting the payload.
/// Contract: `docs/udp-integration-contract.md`. Flows are capped per
/// peer and per service, expire after the service's idle timeout, and their
/// sockets are then quarantined as sinks; each flow has a bounded queue.
async fn handle_udp_connection(
    conn: &Connection,
    target: ServiceTarget,
    sinks: &shutdown::QuarantineSinks,
    mut stop: shutdown::Stop,
) {
    use crate::udp::{
        carries_datagrams, closed_for_unsupported_datagrams, Admission, DropReason,
        DATAGRAMS_UNSUPPORTED_CODE, DATAGRAMS_UNSUPPORTED_REASON,
    };
    let peer = conn.remote_id();
    let alpn = conn.alpn();
    let service = proto::name_from_alpn(alpn)
        .map(String::from)
        .unwrap_or_else(|| format!("{alpn:02x?}"));
    // A connection that cannot carry datagrams never gets a flow table,
    // sweep or reader: close it with the dedicated code and stop.
    if !carries_datagrams(conn.max_datagram_size()) {
        tracing::error!(
            peer = %peer,
            service = %service,
            "udp datagrams unsupported by access peer, closing the connection"
        );
        conn.close(
            VarInt::from_u32(DATAGRAMS_UNSUPPORTED_CODE),
            DATAGRAMS_UNSUPPORTED_REASON,
        );
        return;
    }
    let stats = target.udp_stats.clone();
    let limits = target.udp_limits;
    let idle = limits.idle_timeout;
    let flows = UdpFlowTable::default();
    let mut readers = JoinSet::new();
    let mut sweep = tokio::time::interval(crate::udp::sweep_interval(idle));
    sweep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        let datagram = tokio::select! {
            biased;
            _ = shutdown::stopped(&mut stop) => break,
            _ = sweep.tick() => {
                let ended = flows
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .expire(std::time::Instant::now(), idle);
                if !ended.is_empty() {
                    tracing::debug!(count = ended.len(), "udp upstream flows expired");
                }
                for upstream in ended {
                    quarantine(upstream, &stats, &stop, sinks);
                }
                continue;
            }
            Some(result) = readers.join_next(), if !readers.is_empty() => {
                shutdown::observe(result);
                continue;
            }
            result = conn.read_datagram() => match result {
                Ok(datagram) => datagram,
                Err(error) => {
                    if closed_for_unsupported_datagrams(&error) {
                        tracing::error!(
                            peer = %peer,
                            service = %service,
                            "access peer closed the udp connection: datagrams unsupported"
                        );
                    } else {
                        tracing::debug!("udp connection closed: {error}");
                    }
                    break;
                }
            }
        };
        let Ok((flow, payload)) = crate::udp::decode(&datagram) else {
            stats.record_drop(DropReason::Malformed);
            tracing::debug!("dropping malformed udp datagram");
            continue;
        };
        let now = std::time::Instant::now();
        let admission = flows
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .admit(flow, now);
        let upstream = match admission {
            Admission::Unknown => {
                stats.record_drop(DropReason::UnknownFlow);
                tracing::debug!(flow = flow.0, "dropping datagram for unknown udp flow");
                continue;
            }
            Admission::Live => {
                let table = flows.lock().unwrap_or_else(|p| p.into_inner());
                match table.get(flow) {
                    Some(upstream) => upstream.queue.clone(),
                    None => continue,
                }
            }
            Admission::New => {
                // Caps: the peer's own flows, then the service's flows across
                // every peer. A refused flow is counted and its identifier
                // spent; nothing running is evicted.
                let peer_flows = flows.lock().unwrap_or_else(|p| p.into_inner()).len();
                if peer_flows >= limits.max_flows_per_peer || !stats.try_open_flow(limits.max_flows)
                {
                    flows.lock().unwrap_or_else(|p| p.into_inner()).refuse(flow);
                    stats.record_drop(DropReason::Cap);
                    tracing::debug!(flow = flow.0, "udp flow refused: flow cap reached");
                    continue;
                }
                let opened = tokio::select! {
                    biased;
                    _ = shutdown::stopped(&mut stop) => {
                        // Give back the slot reserved above.
                        stats.flows_closed(1);
                        break;
                    }
                    opened = open_upstream(&target.local_addr) => opened,
                };
                let socket = match opened {
                    Ok(socket) => Arc::new(socket),
                    Err(error) => {
                        // Bind or descriptor exhaustion refuses the flow; the
                        // process keeps running (contract: counter `cap`).
                        stats.flows_closed(1);
                        flows.lock().unwrap_or_else(|p| p.into_inner()).refuse(flow);
                        stats.record_drop(DropReason::Cap);
                        tracing::warn!(flow = flow.0, "cannot open udp upstream: {error}");
                        continue;
                    }
                };
                let relay = readers.spawn(relay_upstream_replies(
                    conn.clone(),
                    flow,
                    socket.clone(),
                    flows.clone(),
                    stats.clone(),
                ));
                // A writer per flow: a slow or blocked target stalls only this
                // flow's bounded queue, never the receive loop.
                let (queue, rx) = crate::udp::FlowQueue::new(limits.queue_depth);
                let writer = readers.spawn(crate::udp::drain_queue(rx, {
                    let socket = socket.clone();
                    let stats = stats.clone();
                    move |datagram: bytes::Bytes| {
                        let socket = socket.clone();
                        let stats = stats.clone();
                        async move {
                            if let Err(error) = socket.send(&datagram).await {
                                stats.record_drop(DropReason::UpstreamError);
                                tracing::debug!(flow = flow.0, "udp upstream send failed: {error}");
                            }
                        }
                    }
                }));
                let entry = queue.clone();
                flows.lock().unwrap_or_else(|p| p.into_inner()).insert(
                    flow,
                    UdpUpstream {
                        socket,
                        queue,
                        relay,
                        writer,
                    },
                    now,
                );
                entry
            }
        };
        match upstream.push(payload) {
            crate::udp::Push::Queued => {}
            crate::udp::Push::Full => {
                stats.record_drop(DropReason::QueueFull);
                tracing::debug!(flow = flow.0, "udp flow queue full, dropping newest");
            }
            crate::udp::Push::Closed => {
                stats.record_drop(DropReason::UpstreamError);
                tracing::debug!(flow = flow.0, "udp flow writer ended, dropping datagram");
            }
        }
    }
    // The connection's flows end with it, but their sockets are quarantined
    // like any expired flow: they outlive the incarnation as sinks only.
    let ended = flows.lock().unwrap_or_else(|p| p.into_inner()).drain_all();
    for upstream in ended {
        quarantine(upstream, &stats, &stop, sinks);
    }
    readers.abort_all();
    shutdown::join(&mut readers).await;
}

/// End a flow: stop relaying its replies and keep its socket open as a sink
/// for [`crate::udp::QUARANTINE`], so a late reply cannot reach a new flow
/// that the OS would otherwise give the same local port.
///
/// The socket keeps its slot against the caps until the sink closes. The sink
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
        ..
    } = upstream;
    // The relay and the writer hold clones of the socket; ending them leaves
    // the sink as the only reader. Datagrams still queued are discarded: the
    // flow is over.
    relay.abort();
    writer.abort();
    stats.quarantine_started();
    let slot = QuarantineSlot(stats.clone());
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

/// Bind a fresh socket of the target's address family and connect it, so the
/// kernel drops replies from any other source.
async fn open_upstream(local_addr: &str) -> Result<UdpSocket> {
    // A hostname target is resolved here, in the receive loop: bound the wait
    // so a slow resolver cannot stall every flow of the peer.
    let resolved = tokio::time::timeout(
        UPSTREAM_RESOLVE_TIMEOUT,
        tokio::net::lookup_host(local_addr),
    )
    .await
    .with_context(|| format!("resolving {local_addr} timed out"))?;
    let target = resolved
        .with_context(|| format!("cannot resolve {local_addr}"))?
        .next()
        .with_context(|| format!("{local_addr} resolved to no address"))?;
    let bind: std::net::SocketAddr = if target.is_ipv4() {
        (std::net::Ipv4Addr::UNSPECIFIED, 0).into()
    } else {
        (std::net::Ipv6Addr::UNSPECIFIED, 0).into()
    };
    let socket = UdpSocket::bind(bind).await?;
    socket.connect(target).await?;
    Ok(socket)
}

/// Forward each reply on `upstream` to the peer as a datagram of `flow`.
/// A reply counts as traffic for the flow's idle timer.
async fn relay_upstream_replies(
    conn: Connection,
    flow: crate::udp::FlowId,
    upstream: Arc<UdpSocket>,
    flows: UdpFlowTable,
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
        flows
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .refresh(flow, std::time::Instant::now());
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
        if let Err(error) = conn.send_datagram(crate::udp::encode(flow, &buf[..n])) {
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
        let tracker = PeerTracker::default();
        let p = peer();
        let first = tracker.track(p, "echo");
        let second = tracker.track(p, "echo");

        let expected = vec![(p, vec!["echo".to_string()])];
        assert_eq!(tracker.snapshot(), expected);

        drop(first);
        assert_eq!(
            tracker.snapshot(),
            expected,
            "one connection left — the entry must survive"
        );

        drop(second);
        assert!(
            tracker.snapshot().is_empty(),
            "peer must disappear with its last connection"
        );
    }

    #[test]
    fn guard_untracks_via_drop_without_any_explicit_call() {
        // The panic-safety contract: nothing calls `untrack` manually —
        // dropping the guard (as an unwind would) must clean up.
        let tracker = PeerTracker::default();
        let p = peer();
        {
            let _tracked = tracker.track(p, "echo");
            assert_eq!(tracker.snapshot(), vec![(p, vec!["echo".to_string()])]);
        }
        assert!(tracker.snapshot().is_empty());
    }

    #[test]
    fn services_merge_across_a_peers_connections() {
        let tracker = PeerTracker::default();
        let p = peer();
        let web = tracker.track(p, "web");
        let echo = tracker.track(p, "echo");

        // Service names sorted within the peer's row.
        assert_eq!(
            tracker.snapshot(),
            vec![(p, vec!["echo".to_string(), "web".to_string()])]
        );

        drop(web);
        assert_eq!(tracker.snapshot(), vec![(p, vec!["echo".to_string()])]);
        drop(echo);
        assert!(tracker.snapshot().is_empty());
    }

    #[test]
    fn snapshot_orders_peers_deterministically() {
        let tracker = PeerTracker::default();
        let peers: Vec<_> = (0..3).map(|_| peer()).collect();
        // Hold every guard so all peers stay tracked.
        let guards: Vec<_> = peers.iter().map(|p| tracker.track(*p, "echo")).collect();

        let mut expected: Vec<String> = peers.iter().map(|p| p.to_string()).collect();
        expected.sort();
        let got: Vec<String> = tracker
            .snapshot()
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
                let mut log = log.lock().unwrap();
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
            PeerTracker::default(),
            sink,
            last,
            stop_rx,
            interval,
        ));
        tokio::time::sleep(interval * 3 + Duration::from_secs(1)).await;
        let _ = stop_tx.send(());
        flush.await.expect("flush task must not panic");
        let saved = log.lock().unwrap().clone();
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
                    let first = entered_tx.lock().unwrap().take();
                    if let Some(tx) = first {
                        events.lock().unwrap().push("save-started");
                        let _ = tx.send(());
                        release.notified().await;
                        events.lock().unwrap().push("save-finished");
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
        assert_eq!(*events.lock().unwrap(), ["save-started", "save-finished"]);
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
        );
        let (_stop_tx, stop) = shutdown::channel();
        let sinks = shutdown::QuarantineSinks::default();
        // Keep an endpoint handle alive past the Serve task: handle_udp_connection
        // returns right after the close, and dropping the last endpoint would tear
        // down before the close frame reaches the client.
        let server = ep.clone();
        let (server_conn_tx, server_conn_rx) = tokio::sync::oneshot::channel::<Connection>();
        let _serve = tokio::spawn(async move {
            let incoming = ep.accept().await.expect("incoming connection");
            let conn = incoming.await.expect("handshake");
            let _ = server_conn_tx.send(conn.clone());
            handle_udp_connection(&conn, target, &sinks, stop).await;
        });

        let conn = tokio::time::timeout(Duration::from_secs(10), client.connect(addr, &alpn))
            .await
            .expect("connect within timeout")
            .expect("connect");
        let reason = tokio::time::timeout(Duration::from_secs(10), conn.closed())
            .await
            .expect("serve must close the connection");
        let _server_conn = server_conn_rx.await.expect("server connection");
        match reason {
            ConnectionError::ApplicationClosed(close) => {
                assert_eq!(
                    close.error_code,
                    VarInt::from_u32(crate::udp::DATAGRAMS_UNSUPPORTED_CODE)
                );
                assert_eq!(
                    close.reason.as_ref(),
                    crate::udp::DATAGRAMS_UNSUPPORTED_REASON
                );
            }
            other => panic!("expected dedicated application close, got {other:?}"),
        }
        client.close().await;
        server.close().await;
    }

    async fn quarantined_upstream(
        stats: &Arc<crate::udp::UdpStats>,
        stop: &shutdown::Stop,
        sinks: &shutdown::QuarantineSinks,
    ) {
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let idle = || tokio::spawn(std::future::pending::<()>()).abort_handle();
        let (queue, _rx) = crate::udp::FlowQueue::new(1);
        let upstream = UdpUpstream {
            socket,
            queue,
            relay: idle(),
            writer: idle(),
        };
        quarantine(upstream, stats, stop, sinks);
    }

    #[tokio::test(start_paused = true)]
    async fn quarantine_sinks_are_joined_on_role_stop() {
        let stats = Arc::new(crate::udp::UdpStats::default());
        let sinks = shutdown::QuarantineSinks::default();
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
        let sinks = shutdown::QuarantineSinks::default();
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
        let sinks = shutdown::QuarantineSinks::default();
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
