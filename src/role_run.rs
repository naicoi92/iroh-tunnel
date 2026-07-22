//! Shared scaffolding for the serve and access role `run` paths.
//!
//! The two roles share ~70% of their skeleton (load config → resolve key →
//! build endpoint → print status → loop → shutdown footer), but the shared
//! pieces were duplicated in `src/serve.rs` and `src/access.rs`. This module
//! hosts the helpers that are genuinely identical across roles so each role
//! file only carries its own divergence.
//!
//! Everything here is `pub(crate)` — these are internal seams between two
//! modules of this crate, not a public extension point.

use std::time::Duration;

use anyhow::Result;
use iroh::endpoint::Connection;

use crate::config::Protocol;

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
/// The weak handle is registered while \`conn\` is still alive, so iroh
/// guarantees the close event is delivered even if \`conn\` drops before this
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
/// Retries [`iroh::Endpoint::connect`] on failure with the schedule
/// [`INITIAL_BACKOFF_MS`] → [`next_backoff_ms`] → … → [`MAX_BACKOFF_MS`]:
/// `1s → 2s → 4s → 8s → 16s → 30s (cap)`. On the first success after one or
/// more failures, logs \`reconnected after N attempts\`. Per-service
/// independent: each local-client task runs its own retry, so one unreachable
/// peer never affects another service (Page 04 v2 §1.3).
///
/// Lifted from \`access::handle_local_connection\` so the backoff contract has
/// one definition site and the schedule is testable.
pub(crate) async fn connect_with_retry(
    ep: &iroh::Endpoint,
    addr: &iroh::EndpointAddr,
    alpn: &[u8],
) -> Result<Connection> {
    let mut backoff_ms = INITIAL_BACKOFF_MS;
    let mut attempt = 1u32;
    loop {
        match ep.connect(addr.clone(), alpn).await {
            Ok(conn) => {
                if attempt > 1 {
                    tracing::info!("reconnected after {attempt} attempts");
                }
                return Ok(conn);
            }
            Err(e) => {
                tracing::warn!("connect attempt {attempt} failed: {e}, retrying in {backoff_ms}ms");
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                backoff_ms = next_backoff_ms(backoff_ms);
                attempt += 1;
            }
        }
    }
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
