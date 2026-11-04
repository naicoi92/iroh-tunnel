//! Shutdown stops admission before draining owned channel tasks.

use std::sync::Arc;
use std::time::Duration;

use tokio::signal;
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

/// Extra time past the drain deadline (endpoint close, status cleanup) before
/// a stuck shutdown is abandoned by [`forced_exit`]. Technical implementation
/// default, not configurable.
const EXIT_WATCHDOG: Duration = Duration::from_secs(10);

/// Why [`forced_exit`] fired. The CLI boundary maps it to a process status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForcedExit {
    /// A stop signal arrived after the drain grace had already elapsed.
    Signal,
    /// Shutdown did not finish within the grace plus [`EXIT_WATCHDOG`].
    Watchdog,
}

impl ForcedExit {
    /// Process exit status: 130 for a repeated stop signal after the grace
    /// (the conventional 128 + SIGINT), 1 for an abandoned stuck shutdown.
    pub fn exit_code(self) -> i32 {
        match self {
            ForcedExit::Signal => 130,
            ForcedExit::Watchdog => 1,
        }
    }
}

/// Installed SIGINT/SIGTERM listeners (SIGINT only on non-Unix).
struct StopSignals {
    #[cfg(unix)]
    terminate: signal::unix::Signal,
}

impl StopSignals {
    fn install() -> Self {
        Self {
            // Installation only fails on OS resource exhaustion; without the
            // handler the process could never stop gracefully, so refuse to
            // start rather than run unstoppable.
            #[cfg(unix)]
            terminate: signal::unix::signal(signal::unix::SignalKind::terminate())
                .expect("failed to install SIGTERM handler"),
        }
    }

    /// Wait for the next stop signal and name it.
    async fn next(&mut self) -> &'static str {
        let ctrl_c = async {
            signal::ctrl_c()
                .await
                .expect("failed to install Ctrl+C handler");
        };
        #[cfg(unix)]
        let term = self.terminate.recv();
        #[cfg(not(unix))]
        let term = std::future::pending::<Option<()>>();
        tokio::select! {
            _ = ctrl_c => "SIGINT (Ctrl+C)",
            _ = term => "SIGTERM",
        }
    }
}

/// Wait for a shutdown signal (SIGINT or SIGTERM), then return.
///
/// Both signals trigger the same graceful shutdown path. The function logs
/// which signal was received so operators can tell Ctrl-C apart from a
/// service manager stop. Repeated signals are handled by [`forced_exit`],
/// which the CLI boundary races against the whole run.
pub async fn wait_for_signal() {
    let signal_name = StopSignals::install().next().await;
    tracing::info!("received {signal_name}");
}

/// Resolve when a stuck or repeatedly interrupted shutdown must be abandoned.
///
/// Pending until [`begin`] has started a drain in `ctx` (and no signal handler
/// is installed until then). Then, measured from that
/// drain's single deadline: a stop signal inside the grace is ignored (an
/// immediate repeat is usually a double-sent stop); one after it resolves
/// [`ForcedExit::Signal`]; and if nothing else happened by the deadline plus
/// [`EXIT_WATCHDOG`] it resolves [`ForcedExit::Watchdog`]. Tokio keeps
/// consuming signals after [`wait_for_signal`] returns, so without this a stuck
/// shutdown could not be interrupted short of SIGKILL.
///
/// This future does not exit the process: the caller (`src/main.rs`) drops the
/// run future, which cancels its owned tasks and endpoint, attempts own-state
/// cleanup, and only then exits with [`ForcedExit::exit_code`].
pub async fn forced_exit(ctx: &RunContext) -> ForcedExit {
    let deadline = ctx.drain_deadline().await;
    #[cfg(feature = "test-utils")]
    let watchdog = std::env::var("IROH_TUNNEL_TEST_EXIT_WATCHDOG_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
        .map(Duration::from_millis)
        .unwrap_or(EXIT_WATCHDOG);
    #[cfg(not(feature = "test-utils"))]
    let watchdog = EXIT_WATCHDOG;
    let mut signals = StopSignals::install();
    forced_exit_after(deadline, watchdog, async || signals.next().await).await
}

/// Test-only seam: with `IROH_TUNNEL_TEST_HANG_SHUTDOWN=1` the calling
/// shutdown step never completes, so only the forced-exit paths can end the
/// process. Compiled only under `test-utils`; production binaries never read
/// the variable.
#[cfg(feature = "test-utils")]
pub async fn test_hang_point() {
    if std::env::var("IROH_TUNNEL_TEST_HANG_SHUTDOWN").is_ok_and(|v| v == "1") {
        std::future::pending::<()>().await;
    }
}

/// Production build: no hang seam.
#[cfg(not(feature = "test-utils"))]
pub async fn test_hang_point() {}

/// Pure core of [`forced_exit`] with injected signal source, for tests.
async fn forced_exit_after<S>(
    deadline: Instant,
    watchdog: Duration,
    mut next_signal: impl AsyncFnMut() -> S,
) -> ForcedExit {
    let watchdog = tokio::time::sleep_until(deadline + watchdog);
    tokio::pin!(watchdog);
    loop {
        tokio::select! {
            _ = &mut watchdog => return ForcedExit::Watchdog,
            _ = next_signal() => {
                if Instant::now() >= deadline {
                    return ForcedExit::Signal;
                }
                tracing::debug!("repeated stop signal inside drain grace ignored");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Signals arrive at the given offsets (ms from now), then never again.
    fn scripted(offsets_ms: &[u64]) -> impl AsyncFnMut() {
        let start = Instant::now();
        let mut queue: std::collections::VecDeque<u64> = offsets_ms.iter().copied().collect();
        async move || match queue.pop_front() {
            Some(ms) => tokio::time::sleep_until(start + Duration::from_millis(ms)).await,
            None => std::future::pending().await,
        }
    }

    #[tokio::test]
    async fn signal_inside_grace_is_ignored_and_one_after_it_forces_exit() {
        let deadline = Instant::now() + Duration::from_millis(300);
        let outcome =
            forced_exit_after(deadline, Duration::from_secs(5), scripted(&[50, 400])).await;
        assert_eq!(outcome, ForcedExit::Signal);
        assert!(Instant::now() >= deadline);
    }

    #[tokio::test]
    async fn watchdog_fires_after_grace_plus_extra_without_signals() {
        let started = Instant::now();
        let deadline = started + Duration::from_millis(100);
        let outcome = forced_exit_after(deadline, Duration::from_millis(200), scripted(&[])).await;
        assert_eq!(outcome, ForcedExit::Watchdog);
        assert!(started.elapsed() >= Duration::from_millis(300));
    }

    #[test]
    fn exit_codes_are_130_for_signal_and_1_for_watchdog() {
        assert_eq!(ForcedExit::Signal.exit_code(), 130);
        assert_eq!(ForcedExit::Watchdog.exit_code(), 1);
    }

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
    async fn management_context_never_resolves() {
        let ctx = RunContext::new();
        assert!(
            tokio::time::timeout(Duration::from_millis(50), forced_exit(&ctx))
                .await
                .is_err()
        );
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
