//! Runtime mechanics shared by the serve and access role loops.

use std::future::Future;
use std::time::Duration;

use anyhow::Result;
use iroh::endpoint::Connection;
use tokio::task::JoinHandle;

/// How long a role waits for its status flush task to stop at shutdown.
const STATUS_FLUSH_JOIN_BOUND: Duration = Duration::from_secs(5);

/// What a role's run loop reports to the run owner.
///
/// `status_quiesced` is `true` only when the status flush task joined within
/// [`STATUS_FLUSH_JOIN_BOUND`] and returned `Ok`, or when no flush task ever
/// ran (no sink, or the body failed before spawning one). Only then may the
/// run owner remove the status file: an unquiesced writer may still have a
/// save in flight that would recreate it. Every role-body path states it
/// explicitly; none is inferred from the result.
pub struct RunCompletion {
    pub result: Result<()>,
    pub status_quiesced: bool,
}

/// Stop a status flush task cooperatively and join it within
/// [`STATUS_FLUSH_JOIN_BOUND`]; returns whether it quiesced.
///
/// Aborting would not cancel an in-flight `spawn_blocking` save
/// (write+fsync+rename runs to completion on the blocking pool and could
/// recreate the file AFTER a cleanup removal). The task exits only at an
/// await point and every save is awaited inside its loop, so a successful
/// join proves none of its saves is still running. On a pathological stall
/// the join is abandoned and the caller must keep the status file.
pub(crate) async fn stop_flush(
    stop_tx: tokio::sync::oneshot::Sender<()>,
    mut flush: JoinHandle<()>,
) -> bool {
    // Dropping the sender alone would also close the channel, but only at
    // scope end — far too late.
    let _ = stop_tx.send(());
    let flush_abort = flush.abort_handle();
    match tokio::time::timeout(STATUS_FLUSH_JOIN_BOUND, &mut flush).await {
        Ok(result) => {
            let clean = result.is_ok();
            crate::engine::shutdown::observe(result);
            clean
        }
        Err(_) => {
            tracing::warn!("status flush task did not stop within 5s; keeping status file");
            flush_abort.abort();
            crate::engine::shutdown::observe(flush.await);
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Small display helpers (shared, no polymorphism)
// ---------------------------------------------------------------------------

/// Format a `host:port` pair for the machine-readable status file, bracketing
/// IPv6 literals (`[::1]:8080`) so the result is unambiguous. Plain IPv4
/// addresses and hostnames are left as `host:port`.
///
/// Shared by both roles' status rows.
pub(crate) fn format_local_addr(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// Future that logs a disconnect line when the peer's QUIC connection
/// closes.
///
/// The weak handle is registered while `conn` is still alive, so iroh
/// guarantees the close event is delivered even if `conn` drops before this
/// resolves. Previously duplicated (with cosmetic differences) in both roles:
/// serve logged `peer disconnected` with a `service` field, access logged
/// `disconnected from serve peer` with an `sname` field. Both call sites
/// now build a single `message` string and a `peer` field, preserving the
/// information without the structural drift.
pub(crate) fn disconnect_watcher(
    conn: &Connection,
    peer: String,
    message: String,
) -> impl Future<Output = ()> + Send + 'static {
    let weak = conn.weak_handle();
    async move {
        let _ = weak.closed().await;
        tracing::info!(%peer, "{message}");
    }
}

// ---------------------------------------------------------------------------
// connect_with_retry + backoff schedule (access role)
// ---------------------------------------------------------------------------

/// Initial backoff (ms) for the access dial retry loop.
const INITIAL_BACKOFF_MS: u64 = 1_000;
/// First per-attempt dial bound; later attempts get more room (see
/// [`attempt_timeout`]).
const DIAL_ATTEMPT_STEP: Duration = Duration::from_secs(5);
/// Ceiling of the per-attempt dial bound.
const DIAL_ATTEMPT_MAX: Duration = Duration::from_secs(20);
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

/// Per-attempt dial bound: `5s → 10s → 15s → 20s (cap)`.
///
/// A stalled first attempt still leaves room for backoff and a retry inside
/// the caller's total budget, while a slow-but-working relay or hole-punch
/// dial gets progressively longer to finish. The caller's deadline, not this,
/// is the real bound.
pub(crate) fn attempt_timeout(attempt: u32) -> Duration {
    DIAL_ATTEMPT_STEP
        .saturating_mul(attempt)
        .min(DIAL_ATTEMPT_MAX)
}

/// Dial a peer with exponential backoff until success or `deadline`.
///
/// Retries [`iroh::Endpoint::connect`] on failure with the schedule
/// [`INITIAL_BACKOFF_MS`] → [`next_backoff_ms`] → … → [`MAX_BACKOFF_MS`]:
/// `1s → 2s → 4s → 8s → 16s → 30s (cap)`. On the first success after one or
/// more failures, logs `reconnected after N attempts`. Per-service
/// independent: an unreachable peer never affects another service.
///
/// # Errors
///
/// Returns an error once `deadline` passes: attempts and backoff sleeps are
/// both truncated at it, so the loop can never run unbounded (RS-09). Callers
/// may additionally cancel the future (for shutdown); dropping it cancels the
/// in-flight dial.
pub(crate) async fn connect_with_retry(
    ep: &iroh::Endpoint,
    addr: &iroh::EndpointAddr,
    alpn: &[u8],
    deadline: tokio::time::Instant,
) -> Result<Connection> {
    let mut backoff_ms = INITIAL_BACKOFF_MS;
    let mut attempt = 1u32;
    loop {
        let now = tokio::time::Instant::now();
        anyhow::ensure!(now < deadline, "dial deadline expired");
        // Dropping a timed-out attempt cancels that dial future.
        let limit = (now + attempt_timeout(attempt)).min(deadline);
        let dial = tokio::time::timeout_at(limit, ep.connect(addr.clone(), alpn)).await;
        match dial {
            Ok(Ok(conn)) => {
                if attempt > 1 {
                    tracing::info!("reconnected after {attempt} attempts");
                }
                return Ok(conn);
            }
            Ok(Err(error)) => {
                tracing::warn!(
                    "connect attempt {attempt} failed: {error}, retrying in {backoff_ms}ms"
                );
            }
            Err(_) if limit == deadline => anyhow::bail!("dial deadline expired"),
            Err(_) => {
                tracing::warn!("connect attempt {attempt} timed out, retrying in {backoff_ms}ms");
            }
        }
        tokio::time::sleep_until(
            (tokio::time::Instant::now() + Duration::from_millis(backoff_ms)).min(deadline),
        )
        .await;
        backoff_ms = next_backoff_ms(backoff_ms);
        attempt += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_local_addr_brackets_ipv6_literal() {
        // IPv6 addresses contain ':' and must be bracketed to be unambiguous.
        assert_eq!(format_local_addr("::1", 8080), "[::1]:8080");
        assert_eq!(format_local_addr("fe80::1", 443), "[fe80::1]:443");
    }

    #[test]
    fn format_local_addr_passes_ipv4_through_unchanged() {
        assert_eq!(format_local_addr("127.0.0.1", 8080), "127.0.0.1:8080");
        assert_eq!(format_local_addr("0.0.0.0", 9000), "0.0.0.0:9000");
    }

    #[test]
    fn format_local_addr_passes_hostname_through_unchanged() {
        assert_eq!(format_local_addr("localhost", 8080), "localhost:8080");
        assert_eq!(format_local_addr("db.internal", 5432), "db.internal:5432");
    }

    #[test]
    fn attempt_timeout_grows_then_caps() {
        let secs: Vec<u64> = (1..=6).map(|n| attempt_timeout(n).as_secs()).collect();
        assert_eq!(secs, [5, 10, 15, 20, 20, 20]);
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

    /// The retry loop is bounded by the caller's deadline by construction:
    /// an unreachable peer yields an error at the deadline, not a hang.
    #[tokio::test]
    async fn connect_with_retry_stops_at_the_deadline() {
        use iroh::endpoint::presets::Minimal;
        let ep = iroh::Endpoint::builder(Minimal)
            .relay_mode(iroh::endpoint::RelayMode::Disabled)
            .bind()
            .await
            .unwrap();
        // Nobody listens on this id: every attempt fails or stalls.
        let addr = iroh::EndpointAddr::new(iroh::SecretKey::generate().public())
            .with_ip_addr((std::net::Ipv4Addr::LOCALHOST, 9).into());
        let started = std::time::Instant::now();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(1500);
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            connect_with_retry(&ep, &addr, b"t", deadline),
        )
        .await
        .expect("connect_with_retry must return by itself at the deadline");
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(4));
    }
}
