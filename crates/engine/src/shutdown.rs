//! Shutdown stops admission before draining owned channel tasks.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use tokio::task::{JoinError, JoinSet};
use tokio::time::Instant;

// One budget from stop receipt across every service/connection, not one grace
// per nested owner. Short exchanges can finish; held tunnels cannot block stop.
// Technical default, not a previously approved #76 numeric policy.
pub(crate) const DRAIN_GRACE: Duration = Duration::from_secs(3);

pub(crate) type Stop = watch::Receiver<Option<Instant>>;

pub(crate) fn channel() -> (watch::Sender<Option<Instant>>, Stop) {
    watch::channel(None)
}

/// Per-invocation record of the first drain deadline.
///
/// `app::execute` creates one per run and shares clones with the role run
/// future and [`forced_exit`], so the CLI boundary measures its watchdog from
/// the same drain the role started, without any process-global state. Clones
/// share one channel; dropping all clones just leaves the deadline unset.
/// Nothing here spawns a task or installs a signal handler.
#[derive(Clone)]
pub struct RunContext(Arc<watch::Sender<Option<Instant>>>);

impl Default for RunContext {
    fn default() -> Self {
        Self::new()
    }
}

impl RunContext {
    /// A context whose drain has not started.
    pub fn new() -> Self {
        Self(Arc::new(watch::channel(None).0))
    }

    /// Resolve with the deadline of the first drain started in this context.
    ///
    /// Pending until [`begin`] publishes; cancel-safe (dropping the future
    /// only drops its subscription).
    pub async fn drain_deadline(&self) -> Instant {
        let mut rx = self.0.subscribe();
        stopped(&mut rx).await
    }

    /// Record `at` unless a drain deadline is already published (first wins).
    pub(crate) fn publish(&self, at: Instant) {
        self.0.send_if_modified(|published| {
            published.is_none() && {
                *published = Some(at);
                true
            }
        });
    }
}

/// Start the drain on `stop` (idempotent) and publish its deadline to `ctx`.
pub(crate) fn begin(stop: &watch::Sender<Option<Instant>>, ctx: &RunContext) {
    if stop.send_if_modified(|deadline| {
        if deadline.is_some() {
            return false;
        }
        let at = Instant::now() + DRAIN_GRACE;
        *deadline = Some(at);
        ctx.publish(at);
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

/// Quarantine sinks owned for one role run.
///
/// A sink outlives its connection (it holds a socket for `QUARANTINE`) but not
/// the role: finished sinks are reaped while the role runs, and `shutdown`
/// joins the rest, aborting at the drain deadline.
#[derive(Default)]
pub(crate) struct QuarantineSinks {
    // Never held across an await.
    tasks: std::sync::Mutex<JoinSet<()>>,
}

impl QuarantineSinks {
    pub(crate) fn spawn(&self, sink: impl std::future::Future<Output = ()> + Send + 'static) {
        self.reap();
        self.lock().spawn(sink);
    }

    /// Observe sinks that have already finished, without waiting.
    pub(crate) fn reap(&self) {
        let mut tasks = self.lock();
        while let Some(result) = tasks.try_join_next() {
            observe(result);
        }
    }

    /// Join every sink, aborting those still running at `deadline`.
    pub(crate) async fn shutdown(&self, deadline: Instant) {
        let mut tasks = std::mem::take(&mut *self.lock());
        drain(&mut tasks, deadline).await;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, JoinSet<()>> {
        self.tasks.lock().unwrap_or_else(|p| p.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn begin_publishes_to_its_own_context() {
        let (tx, _rx) = channel();
        let ctx = RunContext::new();
        begin(&tx, &ctx);
        let published = ctx.drain_deadline().await;
        assert_eq!(Some(published), *tx.borrow());
    }

    #[tokio::test]
    async fn two_runs_do_not_share_a_deadline() {
        let (tx1, _rx1) = channel();
        let ctx1 = RunContext::new();
        let ctx2 = RunContext::new();
        begin(&tx1, &ctx1);
        let one_ms = Duration::from_millis(1);
        assert!(tokio::time::timeout(one_ms, ctx1.drain_deadline())
            .await
            .is_ok());
        assert!(tokio::time::timeout(one_ms, ctx2.drain_deadline())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn first_published_deadline_wins() {
        let ctx = RunContext::new();
        let first = Instant::now() + Duration::from_secs(1);
        ctx.publish(first);
        ctx.publish(first + Duration::from_secs(5));
        assert_eq!(ctx.drain_deadline().await, first);
    }
}
