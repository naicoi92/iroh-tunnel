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

use crate::config::{AccessConfig, AccessService};
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
        for svc in cfg.services {
            let node_id = svc
                .node_id
                .parse::<iroh::EndpointId>()
                .with_context(|| format!("invalid node_id: {}", svc.node_id))?;
            let listen_addr = format!("{}:{}", svc.host, svc.port);
            let effective: Vec<iroh::RelayUrl> = if svc.relay_urls.is_empty() {
                node_relay_urls.clone()
            } else {
                endpoint::resolve_relay_urls(&svc.relay_urls)
                    .with_context(|| format!("service '{}': invalid relay_urls", svc.name))?
            };
            let dialer = Arc::new(ServiceDialer::new(
                &ep,
                node_id,
                &svc,
                &effective,
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
            svc_name: svc.name.clone(),
            listen_addr,
            multiplex: svc.multiplex,
            conn: Mutex::new(None),
        }
    }

    /// Get the shared multiplexed connection, dialing it if there is none.
    ///
    /// The lock is held across the retry loop on purpose: N concurrent
    /// channels produce exactly one connection, not N.
    async fn get_or_dial(&self) -> Result<iroh::endpoint::Connection> {
        let mut guard = self.conn.lock().await;
        if let Some(conn) = guard.as_ref() {
            return Ok(conn.clone());
        }
        let conn = crate::role_run::connect_with_retry(&self.ep, &self.addr, &self.alpn)
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
    async fn invalidate(&self) {
        *self.conn.lock().await = None;
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
