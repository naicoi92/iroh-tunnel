//! What the service tells the SCM and when, independent of the Windows API:
//! the dispatcher binds [`StatusSink`] to the real status handle, tests to a
//! fake. Every report failure is logged; `Running` and `Stopped` are retried,
//! and a lost `Running` stops the service with a failure code instead of
//! leaving it in start-pending.

use std::fmt::Display;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use super::{exit_to_scm, Report, ScmExit, ScmMachine, StopLatch};

/// Attempts for a `Running` or `Stopped` report before giving up.
const REPORT_ATTEMPTS: u32 = 3;

/// A status the SCM is told about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    Pending(Report),
    Running,
    Stopped(ScmExit),
}

impl Status {
    /// State and checkpoint, for log lines.
    fn describe(self) -> String {
        match self {
            Status::Pending(report) => {
                format!("{:?} (checkpoint {})", report.state, report.checkpoint)
            }
            Status::Running => "Running".to_owned(),
            Status::Stopped(exit) => format!("Stopped ({exit:?})"),
        }
    }
}

/// Where status reports go: the SCM's status handle.
pub(crate) trait StatusSink {
    type Error: Display;
    fn set(&self, status: Status) -> Result<(), Self::Error>;
}

/// A failure of the service's own bookkeeping, fatal for the service. Each
/// carries the service-specific exit code reported with the final `Stopped`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fault {
    /// The state lock was poisoned by a panic inside its critical section.
    StatePoisoned,
    /// The SCM refused a `Running` or `Stopped` report on every attempt.
    StatusReport,
    /// The pending-checkpoint ticker thread could not be started.
    TickerSpawn,
}

impl Fault {
    /// Service-specific exit code; distinct from the CLI exit codes (0-6, 130).
    pub(crate) fn code(self) -> u32 {
        match self {
            Fault::StatePoisoned => 100,
            Fault::StatusReport => 101,
            Fault::TickerSpawn => 102,
        }
    }
}

/// No fault recorded.
const NO_FAULT: u32 = 0;

/// State shared by `service_main`, the control handler and the ticker.
pub(crate) struct Session<S> {
    sink: OnceLock<S>,
    machine: Mutex<ScmMachine>,
    latch: StopLatch,
    /// Code of the first [`Fault`] that is to end the service, or [`NO_FAULT`].
    fault: AtomicU32,
    /// Begins the role's drain.
    request_stop: Box<dyn Fn() + Send + Sync>,
    retry_delay: Duration,
}

impl<S: StatusSink> Session<S> {
    pub(crate) fn new(request_stop: Box<dyn Fn() + Send + Sync>, retry_delay: Duration) -> Self {
        Self {
            sink: OnceLock::new(),
            machine: Mutex::new(ScmMachine::default()),
            latch: StopLatch::default(),
            fault: AtomicU32::new(NO_FAULT),
            request_stop,
            retry_delay,
        }
    }

    /// Bind the status handle once the control handler is registered; a
    /// second handle is ignored.
    pub(crate) fn attach(&self, sink: S) {
        let _first_wins = self.sink.set(sink);
    }

    fn machine(&self) -> Result<MutexGuard<'_, ScmMachine>, Fault> {
        self.machine.lock().map_err(|_| {
            tracing::error!("service state lock poisoned; stopping the service");
            Fault::StatePoisoned
        })
    }

    /// One attempt; a failure is logged with the state and checkpoint.
    fn send_once(&self, status: Status) -> bool {
        let Some(sink) = self.sink.get() else {
            return true;
        };
        match sink.set(status) {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!("SCM refused status {}: {e}", status.describe());
                false
            }
        }
    }

    /// Up to [`REPORT_ATTEMPTS`] attempts, `retry_delay` apart.
    fn send_retrying(&self, status: Status) -> Result<(), Fault> {
        for attempt in 1..=REPORT_ATTEMPTS {
            if self.send_once(status) {
                return Ok(());
            }
            if attempt < REPORT_ATTEMPTS {
                std::thread::sleep(self.retry_delay);
            }
        }
        tracing::error!(
            "SCM refused status {} {REPORT_ATTEMPTS} times; giving up",
            status.describe()
        );
        Err(Fault::StatusReport)
    }

    /// Remember the first fault; later ones keep it.
    fn record_fault(&self, fault: Fault) {
        let _first_wins =
            self.fault
                .compare_exchange(NO_FAULT, fault.code(), Ordering::SeqCst, Ordering::SeqCst);
    }

    /// Begin the drain on the first stop request, the SCM's or our own.
    fn begin_stop(&self, machine: &mut ScmMachine) {
        if self.latch.set() {
            (self.request_stop)();
        }
        if let Some(report) = machine.on_stop() {
            // A lost pending report is repeated by the next tick.
            let _ticker_repeats = self.send_once(Status::Pending(report));
        }
    }

    /// A stop, shutdown or pre-shutdown control.
    ///
    /// # Errors
    ///
    /// [`Fault::StatePoisoned`]: the caller ends the process via [`Self::abandon`].
    pub(crate) fn on_stop_control(&self) -> Result<(), Fault> {
        let mut machine = self.machine()?;
        self.begin_stop(&mut machine);
        Ok(())
    }

    /// The role is ready: report `Running` unless a stop is already in
    /// flight. A `Running` report the SCM keeps refusing stops the service
    /// with [`Fault::StatusReport`] rather than leaving it start-pending.
    ///
    /// # Errors
    ///
    /// [`Fault::StatePoisoned`]: the caller ends the process via [`Self::abandon`].
    pub(crate) fn on_ready(&self) -> Result<(), Fault> {
        let mut machine = self.machine()?;
        if machine.on_ready() && self.send_retrying(Status::Running).is_err() {
            self.record_fault(Fault::StatusReport);
            self.begin_stop(&mut machine);
        }
        Ok(())
    }

    /// The periodic tick; `Ok(true)` once the service is done.
    ///
    /// # Errors
    ///
    /// [`Fault::StatePoisoned`]: the caller ends the process via [`Self::abandon`].
    pub(crate) fn tick(&self) -> Result<bool, Fault> {
        let mut machine = self.machine()?;
        if machine.is_done() {
            return Ok(true);
        }
        if let Some(report) = machine.tick() {
            // Not retried: the next tick sends a fresh checkpoint.
            let _next_tick_repeats = self.send_once(Status::Pending(report));
        }
        Ok(false)
    }

    /// The run ended with process exit `code`: report `Stopped` exactly once,
    /// carrying a recorded fault's code in place of the run's.
    ///
    /// # Errors
    ///
    /// [`Fault::StatePoisoned`], or [`Fault::StatusReport`] when the SCM kept
    /// refusing `Stopped`; the caller ends the process via [`Self::abandon`].
    pub(crate) fn on_finished(&self, code: i32) -> Result<(), Fault> {
        let mut machine = self.machine()?;
        if !machine.finish() {
            return Ok(());
        }
        let exit = match self.fault.load(Ordering::SeqCst) {
            NO_FAULT => exit_to_scm(code, self.latch.was_set()),
            fault => ScmExit::Specific(fault),
        };
        self.send_retrying(Status::Stopped(exit))
    }

    /// End the service before the run starts because of `fault`.
    ///
    /// # Errors
    ///
    /// As [`Self::on_finished`].
    pub(crate) fn fail_start(&self, fault: Fault) -> Result<(), Fault> {
        self.record_fault(fault);
        self.on_finished(1)
    }

    /// Last resort for a fatal fault: report `Stopped` with its code without
    /// the state lock (best effort) and return the process exit code.
    pub(crate) fn abandon(&self, fault: Fault) -> i32 {
        let code = fault.code();
        let _process_exit_follows = self.send_retrying(Status::Stopped(ScmExit::Specific(code)));
        i32::try_from(code).unwrap_or(1)
    }
}

#[cfg(test)]
mod tests {
    use super::super::PendingState;
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;

    /// Decides whether the fake refuses a status.
    type Refusal = fn(Status) -> bool;

    #[derive(Default)]
    struct FakeLog {
        sent: Vec<Status>,
        attempts: Vec<Status>,
        /// Each matching attempt consumes the front refusal.
        refuse: VecDeque<Refusal>,
    }

    /// Records every attempted and accepted status; refuses an attempt that
    /// matches the front of the queued refusals.
    #[derive(Clone, Default)]
    struct FakeHandle(Arc<Mutex<FakeLog>>);

    impl FakeHandle {
        fn refusing(refusals: &[Refusal]) -> Self {
            let handle = Self::default();
            handle.log().refuse.extend(refusals.iter().copied());
            handle
        }

        fn log(&self) -> MutexGuard<'_, FakeLog> {
            self.0.lock().unwrap()
        }

        fn sent(&self) -> Vec<Status> {
            self.log().sent.clone()
        }

        fn attempts(&self) -> Vec<Status> {
            self.log().attempts.clone()
        }
    }

    impl StatusSink for FakeHandle {
        type Error = &'static str;
        fn set(&self, status: Status) -> Result<(), Self::Error> {
            let mut log = self.0.lock().map_err(|_| "fake poisoned")?;
            log.attempts.push(status);
            if log.refuse.front().is_some_and(|refused| refused(status)) {
                log.refuse.pop_front();
                return Err("refused");
            }
            log.sent.push(status);
            Ok(())
        }
    }

    fn is_running(status: Status) -> bool {
        status == Status::Running
    }

    fn is_stopped(status: Status) -> bool {
        matches!(status, Status::Stopped(_))
    }

    fn session(handle: &FakeHandle) -> (Session<FakeHandle>, Arc<AtomicUsize>) {
        let stops = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&stops);
        let session = Session::new(
            Box::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
            Duration::ZERO,
        );
        session.attach(handle.clone());
        (session, stops)
    }

    #[test]
    fn running_refused_once_is_retried() {
        let handle = FakeHandle::refusing(&[is_running]);
        let (session, stops) = session(&handle);
        session.on_ready().unwrap();
        assert_eq!(handle.attempts(), [Status::Running, Status::Running]);
        assert_eq!(handle.sent(), [Status::Running]);
        assert_eq!(stops.load(Ordering::SeqCst), 0);
        session.on_finished(0).unwrap();
        assert_eq!(handle.sent().last(), Some(&Status::Stopped(ScmExit::Clean)));
    }

    #[test]
    fn running_always_refused_stops_with_a_failure_code() {
        let handle = FakeHandle::refusing(&[is_running as Refusal; 3]);
        let (session, stops) = session(&handle);
        session.on_ready().unwrap();
        assert_eq!(stops.load(Ordering::SeqCst), 1, "the drain was requested");
        let pending = handle.sent();
        assert!(
            matches!(
                pending.as_slice(),
                [Status::Pending(Report {
                    state: PendingState::StopPending,
                    ..
                })]
            ),
            "{pending:?}"
        );
        // The drained run exits cleanly, yet the SCM learns of the failure.
        session.on_finished(0).unwrap();
        assert_eq!(
            handle.sent().last(),
            Some(&Status::Stopped(ScmExit::Specific(
                Fault::StatusReport.code()
            )))
        );
        assert!(session.tick().unwrap(), "done: the ticker exits");
    }

    #[test]
    fn stopped_refused_once_is_retried() {
        let handle = FakeHandle::refusing(&[is_stopped]);
        let (session, _) = session(&handle);
        session.on_finished(2).unwrap();
        assert_eq!(handle.sent(), [Status::Stopped(ScmExit::Specific(2))]);
        assert_eq!(handle.attempts().len(), 2);
    }

    #[test]
    fn stopped_always_refused_is_a_fault() {
        let handle = FakeHandle::refusing(&[is_stopped as Refusal; 3]);
        let (session, _) = session(&handle);
        assert_eq!(session.on_finished(0), Err(Fault::StatusReport));
        assert_eq!(session.abandon(Fault::StatusReport), 101);
        assert_eq!(
            handle.sent(),
            [Status::Stopped(ScmExit::Specific(101))],
            "abandon retries the final report"
        );
    }

    #[test]
    fn stopped_is_reported_once() {
        let handle = FakeHandle::default();
        let (session, _) = session(&handle);
        session.on_finished(1).unwrap();
        session.on_finished(1).unwrap();
        assert_eq!(handle.sent(), [Status::Stopped(ScmExit::Specific(1))]);
    }

    #[test]
    fn scm_stop_is_clean_and_requests_the_drain_once() {
        let handle = FakeHandle::default();
        let (session, stops) = session(&handle);
        session.on_ready().unwrap();
        session.on_stop_control().unwrap();
        session.on_stop_control().unwrap();
        assert_eq!(stops.load(Ordering::SeqCst), 1);
        session.on_finished(130).unwrap();
        assert_eq!(handle.sent().last(), Some(&Status::Stopped(ScmExit::Clean)));
    }

    #[test]
    fn ticker_spawn_failure_stops_with_its_code() {
        let handle = FakeHandle::default();
        let (session, _) = session(&handle);
        session.fail_start(Fault::TickerSpawn).unwrap();
        assert_eq!(
            handle.sent(),
            [Status::Stopped(ScmExit::Specific(
                Fault::TickerSpawn.code()
            ))]
        );
    }

    #[test]
    fn pending_reports_are_not_retried() {
        let handle = FakeHandle::refusing(&[|s| matches!(s, Status::Pending(_))]);
        let (session, _) = session(&handle);
        assert!(!session.tick().unwrap());
        assert_eq!(handle.attempts().len(), 1);
        assert!(!session.tick().unwrap());
        assert_eq!(handle.sent().len(), 1, "the next tick reports again");
    }

    #[test]
    fn poisoned_state_is_a_fault_not_recovered() {
        let handle = FakeHandle::default();
        let (session, _) = session(&handle);
        let session = Arc::new(session);
        let poisoner = Arc::clone(&session);
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.machine.lock().unwrap();
            panic!("poison the state lock");
        })
        .join();
        assert_eq!(session.on_ready(), Err(Fault::StatePoisoned));
        assert_eq!(session.on_stop_control(), Err(Fault::StatePoisoned));
        assert_eq!(session.tick(), Err(Fault::StatePoisoned));
        assert_eq!(session.on_finished(0), Err(Fault::StatePoisoned));
        assert!(handle.sent().is_empty());
        assert_eq!(session.abandon(Fault::StatePoisoned), 100);
        assert_eq!(handle.sent(), [Status::Stopped(ScmExit::Specific(100))]);
    }

    #[test]
    fn fault_codes_do_not_collide_with_cli_exit_codes() {
        for fault in [
            Fault::StatePoisoned,
            Fault::StatusReport,
            Fault::TickerSpawn,
        ] {
            let code = fault.code();
            assert!(code > 6 && code != 130, "{fault:?} = {code}");
        }
    }
}
