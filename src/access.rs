//! Access role: consume remote services to local.
//!
//! Implements T-07. Loads the access config, builds an [`Endpoint`], and for
//! each service opens a local TCP listener. When a local client connects,
//! access opens a bidirectional QUIC stream to the remote serve peer and
//! pipes the client stream through it.
//!
//! ## Multiplexing (0.2.0)
//!
//! Each service has a [`MultiplexMode`] (config `multiplex`, default `auto`):
//!
//! - **Multiplexed** (`auto` with a 0.2.0+ serve peer, or `on`): the service
//!   keeps ONE long-lived iroh connection to the serve peer and opens one
//!   bidirectional stream per local client. Handshakes are paid once. When
//!   the connection dies every tunneled channel sees EOF (correct
//!   port-forward semantics) and the next channel dials a fresh connection.
//! - **Legacy** (`off`, or `auto` after a pre-0.2.0 serve peer refused the
//!   multiplex ALPN): one iroh connection per local client, exactly the
//!   pre-0.2.0 behavior.
//!
//! The mode is negotiated via the dual-ALPN convention (see [`crate::proto`]):
//! access dials `iroh-tunnel/{name}/multi`; a serve peer that does not know
//! that ALPN refuses the connection at the TLS handshake (fail-fast, no
//! hang), which access detects and falls back from.
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

use std::future::Future;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

use crate::config::{AccessConfig, AccessService, MultiplexMode};
use crate::endpoint;
use crate::proto;
use crate::role_run::RoleStrategy;

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
        endpoint::create_access_endpoint(&cfg.node).await
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
            println!(
                "Exposed: {} {listen_addr} -> peer {node_id_display} ({}://{listen_addr})",
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
        // Resolve the relay URLs and parse every service's node_id before
        // spawning listeners, so a bad config fails fast.
        let relay_urls: Vec<iroh::RelayUrl> = endpoint::resolve_relay_urls(&cfg.node.relay_urls)?;
        if cfg.node.relay_urls.is_empty() {
            tracing::info!(
                count = relay_urls.len(),
                "no relay_urls configured, falling back to n0 default relays"
            );
        }

        let mut handles = Vec::new();
        for svc in cfg.services {
            let node_id = svc
                .node_id
                .parse::<iroh::EndpointId>()
                .with_context(|| format!("invalid node_id: {}", svc.node_id))?;
            let listen_addr = format!("{}:{}", svc.host, svc.port);
            let dialer = Arc::new(ServiceDialer::new(
                &ep,
                node_id,
                &svc,
                &relay_urls,
                listen_addr,
            ));
            handles.push(tokio::spawn(listen_loop(dialer)));
        }

        tracing::info!("access endpoint ready, listening for local clients");
        shutdown.await;
        // Abort each per-service listener so they stop accepting new local
        // clients before the endpoint close tears down the in-flight dials.
        for h in handles {
            h.abort();
        }
        ep.close().await;
        Ok(())
    }
}

/// Per-service dialing state shared by all local-client tasks.
///
/// Owns the multiplexed-connection cache and the fallback bookkeeping. The
/// legacy per-channel path keeps no state here beyond the fallback counters.
struct ServiceDialer {
    ep: iroh::Endpoint,
    addr: iroh::EndpointAddr,
    /// Legacy ALPN (`iroh-tunnel/{name}`) — one connection per channel.
    alpn: Vec<u8>,
    /// Multiplex ALPN (`iroh-tunnel/{name}/multi`) — shared connection.
    multi_alpn: Vec<u8>,
    svc_name: String,
    listen_addr: String,
    mode: MultiplexMode,
    /// The shared multiplexed connection, if any. Guarded by a tokio Mutex so
    /// concurrent channels serialize the (re)dial instead of racing N dials.
    conn: Mutex<Option<iroh::endpoint::Connection>>,
    /// Set once the serve peer refuses the multiplex ALPN (pre-0.2.0 peer).
    /// Only consulted in `auto` mode; cleared again once the last legacy
    /// channel closes so multi is re-probed (a serve peer may upgrade).
    refused: AtomicBool,
    /// Number of in-flight legacy channels created *because* of the refusal.
    /// When it drops to zero, [`ServiceDialer::refused`] is cleared.
    legacy_active: AtomicUsize,
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
            multi_alpn: proto::multiplex_alpn_for(&svc.name),
            svc_name: svc.name.clone(),
            listen_addr,
            mode: svc.multiplex,
            conn: Mutex::new(None),
            refused: AtomicBool::new(false),
            legacy_active: AtomicUsize::new(0),
        }
    }

    /// Get the shared multiplexed connection, dialing it if there is none.
    ///
    /// Retries transient failures with the usual backoff schedule, but fails
    /// fast (no retry) when the peer refused the multiplex ALPN — that is a
    /// version mismatch, not a network problem. The lock is held across the
    /// dial on purpose: N concurrent channels produce exactly one connection.
    async fn get_or_dial_multi(
        &self,
    ) -> std::result::Result<iroh::endpoint::Connection, iroh::endpoint::ConnectError> {
        let mut guard = self.conn.lock().await;
        if let Some(conn) = guard.as_ref() {
            return Ok(conn.clone());
        }
        let conn =
            crate::role_run::connect_with_retry(&self.ep, &self.addr, &self.multi_alpn, |e| {
                !is_peer_refusal(e)
            })
            .await
            .inspect(|conn| {
                let remote_id = conn.remote_id();
                tracing::info!(
                    peer = %remote_id,
                    %self.svc_name,
                    "connected to serve peer (multiplexed)"
                );
                crate::role_run::spawn_disconnect_watcher(
                    conn,
                    remote_id.to_string(),
                    format!(
                        "disconnected from serve peer (service {}, multiplexed)",
                        self.svc_name
                    ),
                );
            })?;
        *guard = Some(conn.clone());
        Ok(conn)
    }

    /// Drop the cached multiplexed connection (it died); the next channel
    /// dials a fresh one.
    async fn invalidate_multi(&self) {
        *self.conn.lock().await = None;
    }

    /// Record that the peer refused multiplexing. While fallback channels
    /// exist the service stays legacy; the last one out clears the flag so
    /// multi is re-probed (see [`ServiceDialer::legacy_channel_finished`]).
    fn note_refusal(&self) {
        self.refused.store(true, Ordering::Release);
    }

    fn legacy_channel_started(&self) {
        self.legacy_active.fetch_add(1, Ordering::AcqRel);
    }

    fn legacy_channel_finished(&self) {
        if self.legacy_active.fetch_sub(1, Ordering::AcqRel) == 1 {
            // Last fallback channel gone: re-probe multiplexing on the next
            // dial cycle (the serve peer may have upgraded to 0.2.0+).
            self.refused.store(false, Ordering::Release);
        }
    }
}

/// Whether a connect error means the *peer refused* the connection (as
/// opposed to a transient/network failure).
///
/// An ALPN the serve peer does not register is refused during the TLS
/// handshake: the peer aborts the connection, surfaced as
/// `ConnectError::Connection { ConnectionError::ConnectionClosed }`. Network
/// problems surface as other variants (`TimedOut`, …) and must NOT be treated
/// as a version mismatch — retrying those is correct.
pub(crate) fn is_peer_refusal(err: &iroh::endpoint::ConnectError) -> bool {
    matches!(
        err,
        iroh::endpoint::ConnectError::Connection {
            source: iroh::endpoint::ConnectionError::ConnectionClosed(_),
            ..
        }
    )
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

/// Distinguishes "peer refused the multiplex ALPN" (a version mismatch the
/// caller may fall back from) from every other dial failure.
#[derive(Debug)]
enum MultiplexError {
    Refused(anyhow::Error),
    Other(anyhow::Error),
}

/// Tunnel one local client according to the service's multiplex mode.
///
/// See the module docs for the mode semantics. Errors close only this local
/// client's channel.
async fn handle_local_connection(dialer: &ServiceDialer, local: TcpStream) -> Result<()> {
    match dialer.mode {
        MultiplexMode::Off => dialer.pipe_legacy(local).await,
        MultiplexMode::On => match dialer.open_multiplexed().await {
            Ok((send, recv)) => crate::pipe::pipe_tcp_bidirectional(local, (recv, send)).await,
            Err(MultiplexError::Refused(inner)) => Err(inner.context(
                "multiplex = \"on\" but the serve peer refused the multiplex ALPN \
                 (it is pre-0.2.0?); use multiplex = \"auto\" or \"off\"",
            )),
            Err(MultiplexError::Other(inner)) => Err(inner),
        },
        MultiplexMode::Auto => {
            if dialer.refused.load(Ordering::Acquire) {
                // Known pre-0.2.0 peer (refused earlier): legacy path for
                // this channel; the last channel out re-probes multi.
                return dialer.pipe_legacy_fallback(local).await;
            }
            match dialer.open_multiplexed().await {
                Ok((send, recv)) => crate::pipe::pipe_tcp_bidirectional(local, (recv, send)).await,
                Err(MultiplexError::Refused(e)) => {
                    tracing::info!(
                        svc = %dialer.svc_name,
                        "serve peer refused multiplex ALPN ({e:#}), \
                         falling back to one-connection-per-channel"
                    );
                    dialer.note_refusal();
                    dialer.pipe_legacy_fallback(local).await
                }
                Err(MultiplexError::Other(e)) => Err(e),
            }
        }
    }
}

impl ServiceDialer {
    /// Multiplexed path, dial phase: get (or redial) the shared connection
    /// and open one bidirectional stream on it for this channel.
    ///
    /// If `open_bi` fails with a connection-level error, the cache is dropped
    /// and exactly one redial + retry is attempted before giving up — a dead
    /// connection surfaces as EOF-style errors on the active channels and the
    /// next channel gets a fresh one.
    async fn open_multiplexed(
        &self,
    ) -> std::result::Result<(iroh::endpoint::SendStream, iroh::endpoint::RecvStream), MultiplexError>
    {
        let conn = self.get_or_dial_multi().await.map_err(classify)?;
        // open_bi returns (SendStream, RecvStream) — send first. Our pipe
        // wants the remote pair as (read, write) = (recv, send), so the
        // caller swaps.
        match conn.open_bi().await {
            Ok(pair) => Ok(pair),
            Err(e) => {
                tracing::warn!(
                    svc = %self.svc_name,
                    "multiplexed open_bi failed: {e}, redialing once"
                );
                self.invalidate_multi().await;
                let conn = self.get_or_dial_multi().await.map_err(classify)?;
                conn.open_bi()
                    .await
                    .context("open bidirectional stream failed after redial")
                    .map_err(MultiplexError::Other)
            }
        }
    }

    /// Legacy path (mode `off`): exactly the pre-0.2.0 behavior — one iroh
    /// connection per local client, retried with backoff until it connects.
    async fn pipe_legacy(&self, local: TcpStream) -> Result<()> {
        self.legacy_connect_and_pipe(local).await
    }

    /// Legacy path taken because the peer refused multiplexing (`auto`
    /// fallback): same as [`ServiceDialer::pipe_legacy`] plus fallback
    /// bookkeeping — while fallback channels exist the service stays legacy;
    /// the last one out clears the refusal so multi is re-probed.
    async fn pipe_legacy_fallback(&self, local: TcpStream) -> Result<()> {
        self.legacy_channel_started();
        let res = self.legacy_connect_and_pipe(local).await;
        self.legacy_channel_finished();
        res
    }

    async fn legacy_connect_and_pipe(&self, local: TcpStream) -> Result<()> {
        // iroh 1.0's Endpoint::connect does NOT reuse an existing connection
        // — every call is a fresh QUIC connection (relay session + TLS
        // handshake). That is precisely the cost multiplexing removes; this
        // path keeps the pre-0.2.0 one-connection-per-channel semantics.
        let conn =
            crate::role_run::connect_with_retry(&self.ep, &self.addr, &self.alpn, |_| true).await?;

        let remote_id = conn.remote_id();
        tracing::info!(peer = %remote_id, %self.svc_name, "connected to serve peer");
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

/// Map a dial failure to its [`MultiplexError`] class by refusal-ness.
fn classify(e: iroh::endpoint::ConnectError) -> MultiplexError {
    if is_peer_refusal(&e) {
        MultiplexError::Refused(anyhow::Error::new(e))
    } else {
        MultiplexError::Other(anyhow::Error::new(e))
    }
}

// No unit test for `is_peer_refusal`: iroh's `ConnectError` is
// `#[non_exhaustive]` with a stack-error `meta` field, so a refusal cannot
// be constructed outside the iroh crate. The classification is verified
// end-to-end by the fallback integration test (a serve endpoint that
// registers only the legacy ALPN must be detected and fallen back from —
// see tests/serve_access_tunnel.rs).
