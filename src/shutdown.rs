//! Graceful shutdown: catch SIGINT/SIGTERM and drain in-flight work.
//!
//! Implements T-08. Replaces the ad-hoc `tokio::signal::ctrl_c()` calls in
//! serve/access with a shared handler that accepts both Ctrl-C (SIGINT) and
//! `kill -TERM` (SIGTERM), then runs a short drain before the endpoint closes.
//!
//! ## Drain model
//!
//! The PoC does not track active connections yet, so [`drain_connections`]
//! just sleeps for a bounded grace period to let in-flight streams finish.
//! Production will swap this for a real "wait for active-connection count to
//! reach zero, capped by `timeout`" implementation.
//!
//! Based on Page 06 v5 §7 (signal handling).

use std::time::Duration;

use tokio::signal;

/// Wait for a shutdown signal (SIGINT or SIGTERM), then return.
///
/// Both signals trigger the same graceful shutdown path. The function logs
/// which signal was received so operators can tell Ctrl-C apart from a
/// service manager stop.
pub async fn wait_for_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("received SIGINT (Ctrl+C)"),
        _ = terminate => tracing::info!("received SIGTERM"),
    }
}

/// Drain in-flight connections, capped at `timeout`.
///
/// Waits up to `timeout` for active streams to finish before the caller closes
/// the endpoint. In this PoC there is no connection tracker, so we just sleep
/// for a short grace period (the configured timeout is logged but the actual
/// wait is bounded to 500ms); production will wait on a real active-count
/// signal instead.
pub async fn drain_connections(timeout: Duration) {
    tracing::info!("draining connections (timeout {}s)...", timeout.as_secs());
    // PoC: bounded sleep instead of tracking active connections.
    tokio::time::sleep(Duration::from_millis(500)).await;
}
