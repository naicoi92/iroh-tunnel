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

use anyhow::{Context, Result};
use iroh::endpoint::Connection;
use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;

use crate::config::ServeConfig;
use crate::endpoint;
use crate::proto;
use crate::role_run::RoleStrategy;
use crate::status::STATUS_FLUSH_INTERVAL;

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

/// Run the serve role like [`run_with_shutdown`], but write the status file
/// into the explicitly given `state_dir` instead of the env-resolved default.
///
/// Advanced/testing seam: integration tests point a serve instance at an
/// isolated tempdir without touching the process-global
/// `IROH_TUNNEL_STATE_DIR` variable (which cannot be mutated safely while
/// other test threads exist). Production callers use [`run_with_shutdown`];
/// operators relocate the file via the env variable instead.
pub async fn run_with_shutdown_with_state_dir(
    state_dir: &Path,
    config_path: &Path,
    shutdown: impl Future<Output = ()>,
) -> Result<()> {
    let state_dir = Some(state_dir.to_path_buf());
    crate::role_run::run_skeleton::<ServeStrategy, _, _>(config_path, |ep, cfg| {
        ServeStrategy::run_loop_with_state_dir(ep, cfg, shutdown, state_dir)
    })
    .await
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
        // the endpoint at build time (not filtered per-accept). The ALPN is
        // deliberately UNCHANGED by multiplexing (no negotiation): a 0.2.0
        // serve is fully backward-compatible with any access peer.
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
        Self::run_loop_with_state_dir(ep, cfg, shutdown, None).await
    }
}

impl ServeStrategy {
    /// [`RoleStrategy::run_loop`] with an optional injected state dir for
    /// the status file (see [`run_with_shutdown_with_state_dir`]).
    async fn run_loop_with_state_dir(
        ep: iroh::Endpoint,
        cfg: ServeConfig,
        shutdown: impl Future<Output = ()>,
        state_dir: Option<std::path::PathBuf>,
    ) -> Result<()> {
        // Build the ALPN -> target lookup for demultiplexing accepted
        // streams.
        let mut targets: HashMap<Vec<u8>, ServiceTarget> = HashMap::new();
        for svc in &cfg.services {
            let target = ServiceTarget::new(format!("{}:{}", svc.host, svc.port));
            targets.insert(proto::alpn_for(&svc.name), target);
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
            state_dir,
        };
        let initial = status.render(Vec::new());
        // Seed the flush loop's change detection with the initial write when
        // it succeeded — otherwise the first tick would rewrite an identical
        // idle snapshot. A failed initial write seeds `None`, so the first
        // tick retries it.
        let seeded = match save_status_file(initial.clone(), status.state_dir.clone()).await {
            Ok(p) => {
                tracing::info!(path = %p.display(), "wrote status file");
                Some(initial)
            }
            Err(e) => {
                tracing::warn!("failed to write status file: {e}");
                None
            }
        };

        // Live registry of connected peers, fed by the accept loop below and
        // read by the status flush task (issue #57).
        let peers = PeerTracker::default();
        tracing::info!("serve endpoint ready, accepting connections");
        let accept_ep = ep.clone();
        let accept_peers = peers.clone();
        let accept = tokio::spawn(async move {
            accept_loop(&accept_ep, targets, accept_peers).await;
        });

        // Graceful-cleanup inputs captured BEFORE `status` moves into the
        // flush task: the state-dir choice (the ownership-checked removal
        // resolves its own path from it) and the cooperative stop channel.
        let cleanup_state_dir = status.state_dir.clone();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();

        // Refresh serve-status.json when the rendered snapshot changes —
        // stream counters, connected peers, or their transport states — at
        // most once per STATUS_FLUSH_INTERVAL: no disk churn under busy
        // stream churn, still near-live for operators.
        let mut flush = tokio::spawn(status_flush_loop(
            status,
            ep.clone(),
            peers,
            seeded,
            stop_rx,
            STATUS_FLUSH_INTERVAL,
            save_status_file,
        ));

        // Wait for the injected shutdown signal, then drain in-flight streams
        // before closing the endpoint (T-08). The accept task is aborted
        // first so it stops handing new connections to the pipe.
        shutdown.await;
        accept.abort();
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
        let flush_stopped = tokio::time::timeout(Duration::from_secs(5), &mut flush)
            .await
            .is_ok();
        if !flush_stopped {
            tracing::warn!(
                "status flush task did not stop within 5s; aborting it and KEEPING \
                 the status file (removal could race its in-flight save)"
            );
            flush_abort.abort();
        }
        ep.close().await;
        // Remove the status file only after a clean join AND only if it
        // still belongs to THIS process (its `pid` field decides — two
        // instances can share a state dir, and an idle peer would not
        // recreate its own file after a blind removal). A crash never runs
        // this path; the reader-side pid-liveness warning covers the stale
        // file it leaves behind.
        if flush_stopped {
            crate::status::remove_own_status_file(
                crate::status::StatusWriter::serve(),
                cleanup_state_dir.as_deref(),
            )
            .await;
        }
        Ok(())
    }
}

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
    /// Injected state dir (testing seam); `None` → env-resolved default.
    state_dir: Option<std::path::PathBuf>,
}
struct StatusServiceRow {
    name: String,
    protocol: String,
    local_addr: String,
    active_streams: Arc<AtomicU64>,
}

impl StatusSnapshot {
    fn render(
        &self,
        connections: Vec<crate::status::PeerConnectionStatus>,
    ) -> crate::status::StatusFile {
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
            connections,
        }
    }
}

/// Live registry of which peers are connected over which services (issue #57).
///
/// The accept loop tracks `(peer, service)` when a connection is accepted and
/// untracks it when the connection task ends; the status flush task renders
/// the registry into `connections`. A peer with several connections to the
/// same service (e.g. an access node with `multiplex = false`) is
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
) -> Vec<crate::status::PeerConnectionStatus> {
    let mut out = Vec::new();
    for (peer, services) in peers.snapshot() {
        let Some(report) = crate::conn_path::peer_path_report(ep, peer).await else {
            continue;
        };
        out.push(crate::status::PeerConnectionStatus {
            path: report,
            services,
        });
    }
    out
}

/// Persist the rendered status file via the shared writer: into `state_dir`
/// when the testing seam injected one, otherwise the env-aware default
/// path (see [`crate::status::StatusWriter`]).
///
/// Runs on the blocking pool: the atomic save fsyncs, and a stalled disk
/// must never stall an async worker (the accept loop shares this runtime).
async fn save_status_file(
    file: crate::status::StatusFile,
    state_dir: Option<std::path::PathBuf>,
) -> Result<std::path::PathBuf> {
    let writer = crate::status::StatusWriter::serve();
    let payload = crate::status::StatusPayload::Serve(file);
    tokio::task::spawn_blocking(move || writer.save_with_state_dir(state_dir.as_deref(), &payload))
        .await
        .context("status save task failed")?
}

/// Periodically rewrite serve-status.json, but only when the rendered
/// snapshot changed (stream counters, peer set, or transport states).
///
/// Stops COOPERATIVELY on `stop`: aborting this task would not cancel an
/// in-flight `spawn_blocking` save (write+fsync+rename runs to completion
/// on the blocking pool and could recreate the file after the shutdown
/// cleanup removed it). The run loop therefore signals, then JOINS this
/// task before removing the file — the loop here must exit on the signal
/// while letting the current save finish.
///
/// `interval` and `save` are seams for unit-testing the stop handshake;
/// production passes [`STATUS_FLUSH_INTERVAL`] and [`save_status_file`].
async fn status_flush_loop<S, F>(
    status: StatusSnapshot,
    ep: iroh::Endpoint,
    peers: PeerTracker,
    mut last: Option<crate::status::StatusFile>,
    mut stop: tokio::sync::oneshot::Receiver<()>,
    interval: Duration,
    mut save: S,
) where
    S: FnMut(crate::status::StatusFile, Option<std::path::PathBuf>) -> F,
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
        let connections = render_connections(&ep, &peers).await;
        let file = status.render(connections);
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

/// Accept connections forever, demultiplexing each to its service by ALPN.
///
/// Returns only if the endpoint is closed (e.g. after Ctrl-C). Per-connection
/// errors are logged, not propagated.
async fn accept_loop(
    ep: &iroh::Endpoint,
    targets: HashMap<Vec<u8>, ServiceTarget>,
    peers: PeerTracker,
) {
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
        let report = crate::conn_path::peer_path_report(ep, remote_id).await;
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
        crate::role_run::spawn_disconnect_watcher(
            &conn,
            remote_id.to_string(),
            format!("peer disconnected (service {name})"),
        );
        let conn_peers = peers.clone();

        tokio::spawn(async move {
            // Track the peer for the status file's `connections` array for
            // as long as this connection lives (issue #57). The guard
            // untracks on drop — normal return or panic unwind alike — so
            // the refcount can never leak.
            let _tracked = conn_peers.track(remote_id, &name);
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
    let mut stream_no: u64 = 0;
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
        stream_no += 1;
        tracing::debug!(stream = stream_no, "accepted stream on connection");

        let local = match TcpStream::connect(&target.local_addr).await {
            Ok(local) => local,
            Err(e) => {
                // Dropping the halves resets this one stream (RESET_STREAM /
                // STOP_SENDING); the access side surfaces it as a failed
                // channel while the connection stays usable.
                tracing::warn!(
                    local_addr = %target.local_addr,
                    stream = stream_no,
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
