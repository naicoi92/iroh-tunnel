//! Shutdown stops admission before draining owned channel tasks.

use std::time::Duration;

use tokio::sync::watch;
use tokio::task::{JoinError, JoinSet};
use tokio::time::Instant;

use crate::fault::Fault;
use crate::locked::Locked;
use crate::runtime::RunContext;

// One budget from stop receipt across every service/connection, not one grace
// per nested owner. Short exchanges can finish; held tunnels cannot block stop.
// Technical default, not a previously approved #76 numeric policy.
pub(crate) const DRAIN_GRACE: Duration = Duration::from_secs(3);

pub(crate) type Stop = watch::Receiver<Option<Instant>>;

pub(crate) fn channel() -> (watch::Sender<Option<Instant>>, Stop) {
    watch::channel(None)
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
///
/// The set is role-wide state: a poisoned lock raises a role fault. Sinks it
/// can no longer track are dropped (releasing their slots) or aborted when
/// the set itself is dropped at the end of the role.
pub(crate) struct QuarantineSinks {
    tasks: Locked<JoinSet<()>>,
    fault: Fault,
}

impl QuarantineSinks {
    pub(crate) fn new(fault: Fault) -> Self {
        Self {
            tasks: Locked::default(),
            fault,
        }
    }

    /// Own `sink` until it ends or the role stops. A sink that cannot be
    /// tracked is dropped unpolled.
    pub(crate) fn spawn(&self, sink: impl std::future::Future<Output = ()> + Send + 'static) {
        self.reap();
        let spawned = self.tasks.with(|tasks| {
            tasks.spawn(sink);
        });
        self.check(spawned);
    }

    /// Observe sinks that have already finished, without waiting.
    pub(crate) fn reap(&self) {
        let finished = self
            .tasks
            .with(|tasks| std::iter::from_fn(|| tasks.try_join_next()).collect::<Vec<_>>());
        if let Some(finished) = self.check(finished) {
            finished.into_iter().for_each(observe);
        }
    }

    /// Join every sink, aborting those still running at `deadline`.
    pub(crate) async fn shutdown(&self, deadline: Instant) {
        let taken = self.tasks.with(std::mem::take);
        if let Some(mut tasks) = self.check(taken) {
            drain(&mut tasks, deadline).await;
        }
    }

    fn check<T>(&self, result: Result<T, crate::locked::Poisoned>) -> Option<T> {
        result
            .map_err(|poisoned| {
                self.fault
                    .raise(anyhow::Error::new(poisoned).context("quarantine sinks"));
            })
            .ok()
    }
}
