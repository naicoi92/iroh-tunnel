//! Process-level stop handling: the signal wait and the forced-exit watchdog.
//!
//! Per-run drain mechanics live in the engine; this module owns the OS signal
//! side, which the engine never touches.

use std::time::Duration;

use tokio::signal;
use tokio::time::Instant;

pub use iroh_tunnel_engine::RunContext;

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
    let mut signals = StopSignals::install();
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
    async fn management_context_never_resolves() {
        let ctx = RunContext::new();
        assert!(
            tokio::time::timeout(Duration::from_millis(50), forced_exit(&ctx))
                .await
                .is_err()
        );
    }
}
