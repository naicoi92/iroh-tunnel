//! Access role: consume remote services to local.
//!
//! Implements T-07. Loads the access config, builds an [`Endpoint`], and for
//! each service opens a local TCP listener. When a local client connects,
//! access opens a bidirectional QUIC stream to the remote serve peer and
//! pipes the client stream through it.
//!
//! ## Multiplexing (0.2.0)
//!
//! Each service has a [`MultiplexMode`] (config `multiplex`, default `auto`).
//! The mode is negotiated with the serve peer via standard ALPN offer
//! negotiation (RFC 7301) on the dual-ALPN convention of [`crate::proto`]:
//!
//! - **`auto`** (default): every dial offers `[iroh-tunnel/{name}/multi,
//!   iroh-tunnel/{name}]` in one TLS handshake. The *serve* side picks (rustls
//!   follows the server's registration order): a 0.2.0+ serve that registers
//!   the multiplex ALPN first picks `…/multi`, a pre-0.2.0 serve picks the
//!   legacy ALPN. The negotiated [`Connection::alpn`] then decides the mode:
//!   - `…/multi` — the service keeps ONE long-lived iroh connection and opens
//!     one bidirectional stream per local client. Handshakes are paid once.
//!     A dead connection surfaces as EOF on its channels (correct
//!     port-forward semantics) and the next channel dials a fresh one.
//!   - legacy — this connection carries this channel only, exactly the
//!     pre-0.2.0 behavior. No fallback state is kept: the next channel
//!     re-offers both ALPNs, so a serve peer that upgrades starts
//!     multiplexing immediately.
//! - **`on`**: offer the multiplex ALPN only. A pre-0.2.0 serve peer refuses
//!   the handshake outright (QUIC strict-ALPN, `NoApplicationProtocol`) —
//!   this fails fast (~100 ms, not a timeout) and surfaces as a loud error.
//! - **`off`**: offer the legacy ALPN only — the pre-0.2.0 behavior verbatim.
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
/// Owns the multiplexed-connection cache. The legacy per-channel path is
/// stateless here: there is no fallback bookkeeping, because with the
/// offer-both negotiation every dial discovers the serve peer's capability
/// anew.
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
        }
    }

    /// Get the shared multiplexed connection, dialing it if there is none.
    ///
    /// With `offer_legacy` (mode `auto`) the dial offers `[multi, legacy]` in
    /// one handshake (see module docs); with any serve peer that knows this
    /// service the connect itself always succeeds, so the usual retry-forever
    /// backoff is correct. Without it (mode `on`) the offer is multi-only and
    /// a refusing peer fails fast (~100 ms, no retry) with a loud hint.
    ///
    /// The lock is held across the dial on purpose: N concurrent channels
    /// produce exactly one connection. Only a *negotiated-multi* connection
    /// is cached — a negotiated-legacy one belongs to its single channel.
    async fn get_or_dial_multi(&self, offer_legacy: bool) -> Result<iroh::endpoint::Connection> {
        let mut guard = self.conn.lock().await;
        if let Some(conn) = guard.as_ref() {
            return Ok(conn.clone());
        }
        let opts = if offer_legacy {
            iroh::endpoint::ConnectOptions::new().with_additional_alpns(vec![self.alpn.clone()])
        } else {
            iroh::endpoint::ConnectOptions::new()
        };
        let retry_if = move |e: &iroh::endpoint::ConnectError| offer_legacy || !is_peer_refusal(e);
        let conn = crate::role_run::connect_with_retry_opts(
            &self.ep,
            &self.addr,
            &self.multi_alpn,
            opts,
            retry_if,
        )
        .await
        .map_err(|e| {
            if !offer_legacy && is_peer_refusal(&e) {
                anyhow::Error::new(e).context(
                    "multiplex = \"on\" but the serve peer refused the multiplex ALPN \
                     (it is pre-0.2.0?); use multiplex = \"auto\" or \"off\"",
                )
            } else {
                anyhow::Error::new(e)
            }
        })
        .inspect(|conn| {
            let mode = if conn.alpn() == self.multi_alpn.as_slice() {
                "multiplexed"
            } else {
                "legacy"
            };
            let remote_id = conn.remote_id();
            tracing::info!(peer = %remote_id, %self.svc_name, mode, "connected to serve peer");
            crate::role_run::spawn_disconnect_watcher(
                conn,
                remote_id.to_string(),
                format!(
                    "disconnected from serve peer (service {}, {mode})",
                    self.svc_name
                ),
            );
        })?;
        if conn.alpn() == self.multi_alpn.as_slice() {
            *guard = Some(conn.clone());
        }
        Ok(conn)
    }

    /// Drop the cached multiplexed connection (it died); the next channel
    /// dials a fresh one.
    async fn invalidate_multi(&self) {
        *self.conn.lock().await = None;
    }
}

/// Whether a connect error means the *peer refused* the connection (as
/// opposed to a transient/network failure).
///
/// An ALPN the serve peer does not register is refused during the TLS
/// handshake via QUIC strict-ALPN (`NoApplicationProtocol`, crypto error
/// 0x178): the peer aborts, surfaced as `ConnectError::Connection {
/// ConnectionError::ConnectionClosed }` within ~100 ms. Network problems
/// surface as other variants (`TimedOut`, …) and must NOT be treated as a
/// version mismatch — retrying those is correct.
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

/// Tunnel one local client according to the service's multiplex mode.
///
/// See the module docs for the negotiation semantics. Errors close only this
/// local client's channel.
async fn handle_local_connection(dialer: &ServiceDialer, local: TcpStream) -> Result<()> {
    match dialer.mode {
        MultiplexMode::Off => dialer.pipe_legacy(local).await,
        MultiplexMode::On => dialer.pipe_multiplex_only(local).await,
        MultiplexMode::Auto => dialer.pipe_negotiated(local).await,
    }
}

impl ServiceDialer {
    /// `auto`: one dial offering both ALPNs; the negotiated ALPN picks the
    /// mode (multiplexed stream on the shared connection, or this channel
    /// rides the negotiated legacy connection alone).
    async fn pipe_negotiated(&self, local: TcpStream) -> Result<()> {
        let conn = self.get_or_dial_multi(true).await?;
        if conn.alpn() == self.multi_alpn.as_slice() {
            let (send, recv) = self.open_bi_with_redial(&conn).await?;
            crate::pipe::pipe_tcp_bidirectional(local, (recv, send)).await
        } else {
            // The serve peer chose the legacy ALPN (pre-0.2.0 peer): this
            // connection carries this channel only — the pre-0.2.0 behavior.
            // The next channel re-offers both ALPNs, so an upgraded serve
            // peer starts multiplexing without any state here.
            self.pipe_channel_on(&conn, local).await
        }
    }

    /// `on`: multiplex or a loud, fast error — never fall back.
    async fn pipe_multiplex_only(&self, local: TcpStream) -> Result<()> {
        // Multi-only offer: a pre-0.2.0 peer refuses at the handshake
        // (fail-fast, see is_peer_refusal) instead of negotiating legacy.
        let conn = self.get_or_dial_multi(false).await?;
        debug_assert_eq!(conn.alpn(), self.multi_alpn.as_slice());
        let (send, recv) = self.open_bi_with_redial(&conn).await?;
        crate::pipe::pipe_tcp_bidirectional(local, (recv, send)).await
    }

    /// `off`: exactly the pre-0.2.0 behavior — one iroh connection per local
    /// client, retried with backoff until it connects.
    async fn pipe_legacy(&self, local: TcpStream) -> Result<()> {
        let conn =
            crate::role_run::connect_with_retry(&self.ep, &self.addr, &self.alpn, |_| true).await?;
        self.pipe_channel_on(&conn, local).await
    }

    /// Pipe one channel over an already-established connection that this
    /// channel owns exclusively (both legacy paths).
    async fn pipe_channel_on(
        &self,
        conn: &iroh::endpoint::Connection,
        local: TcpStream,
    ) -> Result<()> {
        // iroh 1.0's Endpoint::connect does NOT reuse an existing connection
        // — every call is a fresh QUIC connection (relay session + TLS
        // handshake). That is precisely the cost multiplexing removes; the
        // legacy paths keep the pre-0.2.0 one-connection-per-channel
        // semantics.
        let remote_id = conn.remote_id();
        tracing::info!(peer = %remote_id, %self.svc_name, "connected to serve peer");
        crate::role_run::spawn_disconnect_watcher(
            conn,
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

    /// Open one bidirectional stream on the shared multiplexed connection.
    ///
    /// If `open_bi` fails with a connection-level error, the cache is dropped
    /// and exactly one redial + retry is attempted before giving up — a dead
    /// connection surfaces as EOF-style errors on the active channels and the
    /// next channel gets a fresh one.
    async fn open_bi_with_redial(
        &self,
        conn: &iroh::endpoint::Connection,
    ) -> Result<(iroh::endpoint::SendStream, iroh::endpoint::RecvStream)> {
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
                let offer_legacy = self.mode != MultiplexMode::On;
                let conn = self.get_or_dial_multi(offer_legacy).await?;
                if conn.alpn() != self.multi_alpn.as_slice() {
                    anyhow::bail!(
                        "service '{}': peer negotiated the legacy ALPN while reopening a stream",
                        self.svc_name
                    );
                }
                conn.open_bi()
                    .await
                    .context("open bidirectional stream failed after redial")
            }
        }
    }
}

// No unit test for `is_peer_refusal`: iroh's `ConnectError` is
// `#[non_exhaustive]` with a stack-error `meta` field, so a refusal cannot
// be constructed outside the iroh crate. The classification is verified
// end-to-end by the fallback integration test (a serve endpoint that
// registers only the legacy ALPN refuses a multi-only offer fast —
// see tests/serve_access_tunnel.rs).
