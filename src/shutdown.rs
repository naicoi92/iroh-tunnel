//! Shutdown stops admission before draining owned channel tasks.

use std::time::Duration;

use tokio::signal;
use tokio::sync::watch;
use tokio::task::{JoinError, JoinSet};
use tokio::time::Instant;

// One budget from stop receipt across every service/connection, not one grace
// per nested owner. Short exchanges can finish; held tunnels cannot block stop.
// Technical default, not a previously approved #76 numeric policy.
const DRAIN_GRACE: Duration = Duration::from_secs(3);

pub(crate) type Stop = watch::Receiver<Option<Instant>>;

pub(crate) fn channel() -> (watch::Sender<Option<Instant>>, Stop) {
    watch::channel(None)
}

pub(crate) fn begin(stop: &watch::Sender<Option<Instant>>) {
    if stop.send_if_modified(|deadline| {
        if deadline.is_some() {
            return false;
        }
        *deadline = Some(Instant::now() + DRAIN_GRACE);
        true
    }) {
        tracing::info!(
            grace_ms = DRAIN_GRACE.as_millis(),
            "stopping admission, draining active channels"
        );
    }
}

/// Sender loss also cancels admission; no owner may outlive its parent.
pub(crate) async fn stopped(stop: &mut Stop) -> Instant {
    loop {
        if let Some(deadline) = *stop.borrow_and_update() {
            return deadline;
        }
        if stop.changed().await.is_err() {
            return Instant::now();
        }
    }
}

pub(crate) fn observe(result: Result<(), JoinError>) {
    if let Err(error) = result {
        if !error.is_cancelled() {
            tracing::error!(%error, "owned lifecycle task failed");
        }
    }
}

/// Only leaf tasks may be aborted: supervisors must first join their children.
pub(crate) async fn drain(tasks: &mut JoinSet<()>, deadline: Instant) {
    if tokio::time::timeout_at(deadline, join(tasks))
        .await
        .is_err()
    {
        tracing::info!(
            remaining = tasks.len(),
            "drain deadline expired, closing remaining channels"
        );
        tasks.abort_all();
        join(tasks).await;
    }
}

pub(crate) async fn join(tasks: &mut JoinSet<()>) {
    while let Some(result) = tasks.join_next().await {
        observe(result);
    }
}

/// Wait for a shutdown signal (SIGINT or SIGTERM), then return.
///
/// Both signals trigger the same graceful shutdown path. The function logs
/// which signal was received so operators can tell Ctrl-C apart from a
/// service manager stop. Later signals do not reset or shorten drain: Tokio's
/// installed process handlers remain active after this future returns.
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
