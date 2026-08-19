//! Shared scaffolding for the serve and access role `run` paths.
//!
//! The two roles share the skeleton: load config → resolve key → build
//! endpoint → print NodeId → loop until shutdown → drain + close. The
//! genuinely different pieces (serve's ALPN demux + status-file write;
//! access's N-listeners + retry-on-dial) live behind the [`RoleStrategy`]
//! trait so the shared skeleton has one definition site.
//!
//! Everything here is `pub(crate)` — these are internal seams between two
//! modules of this crate, not a public extension point.

use std::future::Future;
use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use iroh::endpoint::Connection;

use crate::config::{Protocol, RoleDoc};
use crate::shutdown;

// ---------------------------------------------------------------------------
// Small display helpers (shared, no polymorphism)
// ---------------------------------------------------------------------------

/// Lowercase protocol name for display (matches the serde form in `config`).
///
/// Previously duplicated verbatim in `serve.rs` and `access.rs`.
pub(crate) fn protocol_str(p: Protocol) -> &'static str {
    match p {
        Protocol::Tcp => "tcp",
        Protocol::Udp => "udp",
    }
}

/// Spawn a task that logs a disconnect line when the peer's QUIC connection
/// closes.
///
/// The weak handle is registered while `conn` is still alive, so iroh
/// guarantees the close event is delivered even if `conn` drops before this
/// resolves. Previously duplicated (with cosmetic differences) in both roles:
/// serve logged `peer disconnected` with a `service` field, access logged
/// `disconnected from serve peer` with an `sname` field. Both call sites
/// now build a single `message` string and a `peer` field, preserving the
/// information without the structural drift.
pub(crate) fn spawn_disconnect_watcher(conn: &Connection, peer: String, message: String) {
    let weak = conn.weak_handle();
    tokio::spawn(async move {
        let _ = weak.closed().await;
        tracing::info!(%peer, "{message}");
    });
}

// ---------------------------------------------------------------------------
// connect_with_retry + backoff schedule (access role)
// ---------------------------------------------------------------------------

/// Initial backoff (ms) for the access dial retry loop.
const INITIAL_BACKOFF_MS: u64 = 1_000;
/// Maximum backoff (ms) — the schedule caps here and stays flat.
const MAX_BACKOFF_MS: u64 = 30_000;

/// Compute the next backoff in the exponential schedule:
/// `1s → 2s → 4s → 8s → 16s → 30s → 30s → …` (capped).
///
/// Factored out of [`connect_with_retry`] so the schedule can be unit-tested
/// without driving a real iroh endpoint. The schedule is the load-bearing
/// piece — it bounds worst-case reconnect latency and matches Page 04 v2 §1.3.
pub(crate) fn next_backoff_ms(prev_ms: u64) -> u64 {
    (prev_ms.saturating_mul(2)).min(MAX_BACKOFF_MS)
}

/// Dial a peer with exponential backoff, retrying forever until success.
///
/// Plain [`iroh::Endpoint::connect`] with the primary `alpn` only. See
/// [`connect_with_retry_opts`] for the backoff contract and the option-aware
/// variant (used by the multiplex negotiation to offer additional ALPNs).
pub(crate) async fn connect_with_retry(
    ep: &iroh::Endpoint,
    addr: &iroh::EndpointAddr,
    alpn: &[u8],
    retry_if: impl Fn(&iroh::endpoint::ConnectError) -> bool,
) -> std::result::Result<Connection, iroh::endpoint::ConnectError> {
    connect_with_retry_opts(
        ep,
        addr,
        alpn,
        iroh::endpoint::ConnectOptions::new(),
        retry_if,
    )
    .await
}

/// Dial a peer with exponential backoff, retrying until success or until a
/// failure the caller refuses to retry.
///
/// Retries [`iroh::Endpoint::connect_with_opts`] on failure with the schedule
/// [`INITIAL_BACKOFF_MS`] → [`next_backoff_ms`] → … → [`MAX_BACKOFF_MS`]:
/// `1s → 2s → 4s → 8s → 16s → 30s (cap)`. On the first success after one or
/// more failures, logs `reconnected after N attempts`. When `retry_if`
/// returns false for an error, that typed error is returned immediately
/// (fail-fast — e.g. the peer refused this ALPN at the handshake) so the
/// caller can act on the specific error class. Per-service independent: each
/// local-client task runs its own retry, so one unreachable peer never
/// affects another service (Page 04 v2 §1.3).
pub(crate) async fn connect_with_retry_opts(
    ep: &iroh::Endpoint,
    addr: &iroh::EndpointAddr,
    alpn: &[u8],
    opts: iroh::endpoint::ConnectOptions,
    retry_if: impl Fn(&iroh::endpoint::ConnectError) -> bool,
) -> std::result::Result<Connection, iroh::endpoint::ConnectError> {
    let mut backoff_ms = INITIAL_BACKOFF_MS;
    let mut attempt = 1u32;
    loop {
        match ep.connect_with_opts(addr.clone(), alpn, opts.clone()).await {
            Ok(connecting) => match connecting.await {
                Ok(conn) => {
                    if attempt > 1 {
                        tracing::info!("reconnected after {attempt} attempts");
                    }
                    return Ok(conn);
                }
                Err(e) => {
                    let err = iroh::endpoint::ConnectError::from(e);
                    if !retry_if(&err) {
                        if attempt > 1 {
                            tracing::warn!("connect failed after {attempt} attempts: {err}");
                        }
                        return Err(err);
                    }
                    tracing::warn!(
                        "connect attempt {attempt} failed: {err}, retrying in {backoff_ms}ms"
                    );
                    tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                    backoff_ms = next_backoff_ms(backoff_ms);
                    attempt += 1;
                }
            },
            Err(e) => {
                let err = iroh::endpoint::ConnectError::from(e);
                if !retry_if(&err) {
                    if attempt > 1 {
                        tracing::warn!("connect failed after {attempt} attempts: {err}");
                    }
                    return Err(err);
                }
                tracing::warn!(
                    "connect attempt {attempt} failed: {err}, retrying in {backoff_ms}ms"
                );
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                backoff_ms = next_backoff_ms(backoff_ms);
                attempt += 1;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// RoleStrategy trait + shared run skeleton
// ---------------------------------------------------------------------------

/// The role's half of the `run` skeleton.
///
/// Each implementer carries the genuinely-different pieces (build the
/// endpoint with role-specific args, print the per-service status lines,
/// run the role-specific loop). The shared skeleton — load config, resolve
/// key, print NodeId, wait for shutdown, drain + close — lives in
/// [`run_with_shutdown`] so it has exactly one definition site.
///
/// The associated type [`RoleStrategy::Config`] is the role's TOML document
/// (`ServeConfig` / `AccessConfig`); it must load+validate+persist its own
/// key via the [`crate::config::RoleDoc`] trait.
pub(crate) trait RoleStrategy {
    /// The TOML config document this role loads.
    type Config: crate::config::RoleDoc;

    /// Build the iroh endpoint for this role (serve registers ALPNs, access
    /// dials out only).
    fn build_endpoint(
        cfg: &Self::Config,
    ) -> impl std::future::Future<Output = Result<iroh::Endpoint>>;

    /// Print the per-service status lines after the endpoint is up (serve:
    /// "Serving: ..."; access: "Exposed: ... -> peer ...").
    fn print_services(cfg: &Self::Config);

    /// Run the role's accept/listen loop until `shutdown` resolves.
    ///
    /// The loop owns the endpoint (cloned if needed) and is responsible for
    /// spawning per-connection tasks. It MUST return when `shutdown` resolves
    /// so the shared skeleton can proceed to drain + close.
    fn run_loop(
        ep: iroh::Endpoint,
        cfg: Self::Config,
        shutdown: impl Future<Output = ()>,
    ) -> impl std::future::Future<Output = Result<()>>;
}

/// Drive a role end-to-end: load → resolve key → endpoint → print → loop →
/// shutdown footer.
///
/// Production callers wrap this with [`shutdown::wait_for_signal`]; tests
/// inject a `oneshot::Receiver` so the role can be driven without sending
/// real signals.
///
/// The status-file write (serve-only) and any other post-build hooks live
/// inside the strategy's `run_loop` — that's where the role has access to
/// both the loaded config and the live endpoint, and where the timing fits
/// between "endpoint ready" and "shutdown received".
pub(crate) async fn run_with_shutdown<S: RoleStrategy>(
    config_path: &Path,
    shutdown: impl Future<Output = ()>,
) -> Result<()> {
    // Load and validate the config, then resolve+persist the secret key. Both
    // steps are shared across roles via the RoleDoc trait.
    let mut cfg = S::Config::load(config_path)?;
    cfg.resolve_and_save_key(config_path)?;

    // Build the role-specific endpoint (serve registers ALPNs, access is
    // outbound-only).
    let ep = S::build_endpoint(&cfg).await?;

    // Print the operator-facing NodeId line (identical across roles).
    println!("NodeId: {}", crate::endpoint::node_id_string(&ep));
    if let Some(relay) = crate::endpoint::home_relay(&ep) {
        println!("Home relay: {relay}");
    }

    // Per-service status lines diverge (Serving vs Exposed), so they sit
    // behind the strategy. The strategy is also responsible for warning
    // when there are no services configured.
    S::print_services(&cfg);

    tracing::info!("endpoint ready");
    // Drive the role-specific loop. The strategy MUST honor `shutdown`.
    S::run_loop(ep, cfg, shutdown).await?;

    // Shared shutdown footer: drain in-flight streams. The endpoint close
    // itself happens inside `run_loop` because each role needs to abort its
    // accept task(s) before closing.
    shutdown::drain_connections(Duration::from_secs(5)).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_str_matches_serde_form() {
        // Must match the lowercase serde rename on the Protocol enum so the
        // displayed form round-trips through config.
        assert_eq!(protocol_str(Protocol::Tcp), "tcp");
        assert_eq!(protocol_str(Protocol::Udp), "udp");
    }

    #[test]
    fn backoff_schedule_doubles_then_caps_at_max() {
        // Page 04 v2 §1.3 contract: 1s → 2s → 4s → 8s → 16s → 30s → 30s → …
        let steps = [
            (1_000u64, 2_000u64),
            (2_000, 4_000),
            (4_000, 8_000),
            (8_000, 16_000),
            (16_000, 30_000), // doubles to 32_000 but capped at MAX_BACKOFF_MS
            (30_000, 30_000), // already at cap, stays at cap
            (30_000, 30_000),
        ];
        for (prev, expected_next) in steps {
            let got = next_backoff_ms(prev);
            assert_eq!(
                got, expected_next,
                "next_backoff_ms({prev}ms) should be {expected_next}ms, got {got}ms"
            );
        }
    }

    #[test]
    fn backoff_schedule_starts_at_1s_and_caps_at_30s() {
        // Regression guards for the two load-bearing constants.
        assert_eq!(INITIAL_BACKOFF_MS, 1_000, "initial backoff must be 1s");
        assert_eq!(MAX_BACKOFF_MS, 30_000, "max backoff must be 30s");
        // The cap is reachable from the initial value within 5 doublings.
        let mut ms = INITIAL_BACKOFF_MS;
        for _ in 0..5 {
            ms = next_backoff_ms(ms);
        }
        assert_eq!(ms, MAX_BACKOFF_MS, "should reach cap after 5 doublings");
    }

    #[test]
    fn backoff_does_not_overflow_on_huge_input() {
        // saturating_mul must keep this finite even for absurd inputs.
        let prev = u64::MAX;
        let next = next_backoff_ms(prev);
        assert_eq!(next, MAX_BACKOFF_MS, "huge input must cap, not overflow");
    }
}
