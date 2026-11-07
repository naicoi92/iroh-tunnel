//! Process-level stop handling: the signal wait and the forced-exit watchdog.
//!
//! Per-run drain mechanics live in the engine; this module owns the OS signal
//! side, which the engine never touches.

use std::time::Duration;

use tokio::signal;
use tokio::sync::watch;
use tokio::time::Instant;

pub use iroh_tunnel_host::RunContext;

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

/// The Windows Service Control Manager's stop request: a latch that is `false`
/// until the first stop, shutdown or pre-shutdown control, then `true` for good.
#[cfg(windows)]
pub(crate) fn scm_stop() -> &'static watch::Sender<bool> {
    static STOP: std::sync::OnceLock<watch::Sender<bool>> = std::sync::OnceLock::new();
    STOP.get_or_init(|| watch::channel(false).0)
}

/// Resolve when the latch `rx` fires. With `replay` a latch that is already
/// set fires immediately, once (the stop arrived before anyone listened);
/// otherwise only a later transition fires, so a watchdog that starts after
/// the stop does not take the same latch for a repeated signal.
#[cfg_attr(not(windows), allow(dead_code))]
async fn latch_fired(rx: &mut watch::Receiver<bool>, replay: &mut bool) {
    if std::mem::take(replay) && *rx.borrow_and_update() {
        return;
    }
    loop {
        if rx.changed().await.is_err() {
            return std::future::pending().await;
        }
        if *rx.borrow_and_update() {
            return;
        }
    }
}

/// Installed SIGINT/SIGTERM listeners (SIGINT only on non-Unix), plus the
/// SCM stop latch on Windows.
struct StopSignals {
    #[cfg(unix)]
    terminate: signal::unix::Signal,
    #[cfg(windows)]
    scm: watch::Receiver<bool>,
    #[cfg(windows)]
    replay: bool,
}

impl StopSignals {
    /// `replay`: a stop the SCM already requested still counts (the primary
    /// stop wait); the watchdog passes `false` and sees only later ones.
    fn install(replay: bool) -> Self {
        #[cfg(not(windows))]
        let _ = replay;
        Self {
            #[cfg(windows)]
            scm: scm_stop().subscribe(),
            #[cfg(windows)]
            replay,
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
        #[cfg(windows)]
        let scm = latch_fired(&mut self.scm, &mut self.replay);
        #[cfg(not(windows))]
        let scm = std::future::pending::<()>();
        tokio::select! {
            _ = ctrl_c => "SIGINT (Ctrl+C)",
            _ = term => "SIGTERM",
            () = scm => "service stop request",
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
    let signal_name = StopSignals::install(true).next().await;
    tracing::info!("received {signal_name}");
}

/// Resolve when a stuck or repeatedly interrupted shutdown must be abandoned.
///
/// Pending until the role has started a drain in `ctx` (and no signal handler
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
    let mut signals = StopSignals::install(false);
    forced_exit_after(deadline, watchdog, async || signals.next().await).await
}

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
    async fn latch_fires_on_transition_and_replays_only_when_asked() {
        let (tx, _keep) = watch::channel(false);
        let mut rx = tx.subscribe();
        let mut replay = false;
        assert!(
            tokio::time::timeout(Duration::from_millis(30), latch_fired(&mut rx, &mut replay))
                .await
                .is_err()
        );
        tx.send_replace(true);
        latch_fired(&mut rx, &mut replay).await;

        // Once set, a late watchdog subscriber is not woken by the same latch.
        let mut late = tx.subscribe();
        let mut no_replay = false;
        assert!(tokio::time::timeout(
            Duration::from_millis(30),
            latch_fired(&mut late, &mut no_replay)
        )
        .await
        .is_err());
        // The primary stop wait still sees a stop that came first, once.
        let mut early = tx.subscribe();
        let mut replay = true;
        latch_fired(&mut early, &mut replay).await;
        assert!(tokio::time::timeout(
            Duration::from_millis(30),
            latch_fired(&mut early, &mut replay)
        )
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
}

#[cfg(test)]
#[allow(clippy::mixed_attributes_style)]
mod scm_stop_tests {
    #![cfg(windows)]
    use super::*;

    #[tokio::test]
    async fn scm_stop_triggers_the_same_drain() {
        let wait = tokio::spawn(wait_for_signal());
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!wait.is_finished());
        scm_stop().send_replace(true);
        tokio::time::timeout(Duration::from_secs(2), wait)
            .await
            .expect("drain begins")
            .unwrap();
    }
}
