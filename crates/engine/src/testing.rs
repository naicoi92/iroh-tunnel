//! Test-only seams, compiled under `test-utils` (the hang point also exists
//! as a no-op in production builds so callers need no cfg).

#[cfg(feature = "test-utils")]
static TEST_RELAY_ONLY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Force (or stop forcing) relay-only endpoints for roles started afterwards.
///
/// Process-global: use it from one sequential test, and reset it before the
/// test ends. Endpoints already built are unaffected.
#[cfg(feature = "test-utils")]
pub fn set_relay_only(on: bool) {
    TEST_RELAY_ONLY.store(on, std::sync::atomic::Ordering::SeqCst);
}

#[cfg(feature = "test-utils")]
pub(crate) fn relay_only() -> bool {
    TEST_RELAY_ONLY.load(std::sync::atomic::Ordering::SeqCst)
}

/// With `IROH_TUNNEL_TEST_HANG_SHUTDOWN=1` the calling shutdown step never
/// completes, so only the forced-exit paths can end the process.
#[cfg(feature = "test-utils")]
pub async fn hang_point() {
    if std::env::var("IROH_TUNNEL_TEST_HANG_SHUTDOWN").is_ok_and(|v| v == "1") {
        std::future::pending::<()>().await;
    }
}

/// Production build: no hang seam.
#[cfg(not(feature = "test-utils"))]
pub async fn hang_point() {}
