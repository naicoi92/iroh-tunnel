//! Running a role under the Windows Service Control Manager (SCM).
//!
//! The state machine, the exit mapping and the pending-report arithmetic are
//! pure and compiled (and tested) on every platform; only the dispatcher in
//! [`dispatcher`] touches Windows APIs.

// Consumed only by the Windows dispatcher; the tests cover them elsewhere.
#![cfg_attr(not(windows), allow(dead_code))]

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

#[cfg(windows)]
mod dispatcher;
#[cfg(windows)]
pub(crate) use dispatcher::run_as_service;

/// Wait hint while starting: covers the claim, the config load and the
/// endpoint bring-up.
const START_HINT: Duration = Duration::from_secs(30);
/// Wait hint while stopping: the 3 s drain plus the 10 s watchdog, with slack.
const STOP_HINT: Duration = Duration::from_secs(15);
/// Exit code of a run that found the role already running (clean for the SCM).
const ALREADY_RUNNING: i32 = 6;

/// How the service ends, as reported to the SCM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScmExit {
    /// Stopped without error; the SCM applies no recovery action.
    Clean,
    /// Service-specific failure carrying the process exit code.
    Specific(u32),
}

/// Map a run's exit code to the SCM exit. A requested stop and an already
/// running role (code 6) are clean; any other non-zero code is a failure.
pub(crate) fn exit_to_scm(code: i32, stop_requested: bool) -> ScmExit {
    if stop_requested || code == 0 || code == ALREADY_RUNNING {
        ScmExit::Clean
    } else {
        ScmExit::Specific(u32::try_from(code).unwrap_or(1))
    }
}

/// Process exit code when the dispatcher cannot start. 1063
/// (`ERROR_FAILED_SERVICE_CONTROLLER_CONNECT`) means the process was not
/// started by the SCM, which is a configuration error.
pub(crate) fn dispatcher_error_code(raw_os_error: Option<i32>) -> i32 {
    const NOT_FROM_SCM: i32 = 1063;
    if raw_os_error == Some(NOT_FROM_SCM) {
        2
    } else {
        1
    }
}

/// A pending state the SCM is told about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingState {
    StartPending,
    StopPending,
}

/// One pending report: state, wait hint and checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Report {
    pub state: PendingState,
    pub wait_hint: Duration,
    pub checkpoint: u32,
}

/// Produces pending reports whose checkpoint advances on every tick.
#[derive(Debug, Default)]
pub(crate) struct PendingReporter {
    checkpoint: u32,
}

impl PendingReporter {
    fn tick(&mut self, state: PendingState, wait_hint: Duration) -> Report {
        self.checkpoint += 1;
        Report {
            state,
            wait_hint,
            checkpoint: self.checkpoint,
        }
    }

    /// Next start-pending report.
    pub(crate) fn start(&mut self) -> Report {
        self.tick(PendingState::StartPending, START_HINT)
    }

    /// Next stop-pending report.
    pub(crate) fn stop(&mut self) -> Report {
        self.tick(PendingState::StopPending, STOP_HINT)
    }
}

/// Remembers that the SCM asked to stop; only the first request counts.
#[derive(Debug, Default)]
pub(crate) struct StopLatch(AtomicBool);

impl StopLatch {
    /// Record a stop request; `true` only for the first one.
    pub(crate) fn set(&self) -> bool {
        !self.0.swap(true, Ordering::SeqCst)
    }

    pub(crate) fn was_set(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Starting,
    Running,
    Stopping,
    Done,
}

/// The service lifecycle as the SCM sees it: starting, running, stopping,
/// done. It decides which report is due; the caller sends it.
#[derive(Debug)]
pub(crate) struct ScmMachine {
    phase: Phase,
    reporter: PendingReporter,
}

impl Default for ScmMachine {
    fn default() -> Self {
        Self {
            phase: Phase::Starting,
            reporter: PendingReporter::default(),
        }
    }
}

impl ScmMachine {
    /// A stop request: the stop-pending report on the first one while the
    /// service is live, `None` afterwards.
    pub(crate) fn on_stop(&mut self) -> Option<Report> {
        match self.phase {
            Phase::Starting | Phase::Running => {
                self.phase = Phase::Stopping;
                Some(self.reporter.stop())
            }
            Phase::Stopping | Phase::Done => None,
        }
    }

    /// The role is ready; `true` when `Running` is to be reported (not after a
    /// stop request).
    pub(crate) fn on_ready(&mut self) -> bool {
        let starting = self.phase == Phase::Starting;
        if starting {
            self.phase = Phase::Running;
        }
        starting
    }

    /// The periodic tick: a pending report while starting or stopping.
    pub(crate) fn tick(&mut self) -> Option<Report> {
        match self.phase {
            Phase::Starting => Some(self.reporter.start()),
            Phase::Stopping => Some(self.reporter.stop()),
            Phase::Running | Phase::Done => None,
        }
    }

    /// The run ended; `true` exactly once, when `Stopped` is to be reported.
    pub(crate) fn finish(&mut self) -> bool {
        let first = self.phase != Phase::Done;
        self.phase = Phase::Done;
        first
    }

    pub(crate) fn is_done(&self) -> bool {
        self.phase == Phase::Done
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_to_scm_maps_codes() {
        assert_eq!(exit_to_scm(0, false), ScmExit::Clean);
        assert_eq!(exit_to_scm(6, false), ScmExit::Clean);
        assert_eq!(exit_to_scm(2, false), ScmExit::Specific(2));
        assert_eq!(exit_to_scm(1, false), ScmExit::Specific(1));
        assert_eq!(exit_to_scm(130, true), ScmExit::Clean);
        assert_eq!(exit_to_scm(1, true), ScmExit::Clean);
    }

    #[test]
    fn stop_pending_reports_15s_hint_and_increments_checkpoint() {
        let mut reporter = PendingReporter::default();
        let first = reporter.stop();
        let second = reporter.stop();
        assert_eq!(first.state, PendingState::StopPending);
        assert_eq!(first.wait_hint, Duration::from_secs(15));
        assert_eq!(second.checkpoint, first.checkpoint + 1);
    }

    #[test]
    fn start_pending_reports_30s_hint_and_ticks() {
        let mut machine = ScmMachine::default();
        let first = machine.tick().unwrap();
        let second = machine.tick().unwrap();
        assert_eq!(first.state, PendingState::StartPending);
        assert_eq!(first.wait_hint, Duration::from_secs(30));
        assert_eq!(second.checkpoint, first.checkpoint + 1);
    }

    #[test]
    fn second_scm_stop_is_ignored() {
        let latch = StopLatch::default();
        assert!(!latch.was_set());
        assert!(latch.set());
        assert!(!latch.set());
        assert!(latch.was_set());
        let mut machine = ScmMachine::default();
        assert!(machine.on_stop().is_some());
        assert!(machine.on_stop().is_none());
    }

    #[test]
    fn stop_before_running_ends_cleanly() {
        let mut machine = ScmMachine::default();
        assert!(machine.on_stop().is_some());
        // A late ready must not report Running over StopPending.
        assert!(!machine.on_ready());
        assert_eq!(machine.tick().unwrap().state, PendingState::StopPending);
        assert_eq!(exit_to_scm(1, true), ScmExit::Clean);
        assert!(machine.finish());
        assert!(!machine.finish());
        assert!(machine.is_done());
    }

    #[test]
    fn ready_moves_to_running_and_stops_ticking() {
        let mut machine = ScmMachine::default();
        assert!(machine.on_ready());
        assert!(!machine.on_ready());
        assert!(machine.tick().is_none());
        assert_eq!(machine.on_stop().unwrap().state, PendingState::StopPending);
    }

    #[test]
    fn dispatcher_error_1063_is_config_error() {
        assert_eq!(dispatcher_error_code(Some(1063)), 2);
        assert_eq!(dispatcher_error_code(Some(5)), 1);
        assert_eq!(dispatcher_error_code(None), 1);
    }
}
