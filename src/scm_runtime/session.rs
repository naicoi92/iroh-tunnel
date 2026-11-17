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
    /// Retries hold the state lock and may block the caller for up to
    /// `REPORT_ATTEMPTS - 1` retry delays, so a racing stop control waits
    /// and then sees the settled phase.
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
    /// the state lock (best effort) and return the process exit code. After
    /// [`Fault::StatusReport`] the SCM already refused `Stopped` every time,
    /// so nothing is resent; the process exit code is all it gets.
    pub(crate) fn abandon(&self, fault: Fault) -> i32 {
        let code = fault.code();
        if fault != Fault::StatusReport {
            let _process_exit_follows =
                self.send_retrying(Status::Stopped(ScmExit::Specific(code)));
        }
        i32::try_from(code).unwrap_or(1)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{PendingState, STOP_HINT};
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::sync::Arc;

    /// Decides whether the fake refuses a status.
    type Refusal = fn(Status) -> bool;

    #[derive(Default)]
    struct FakeLog {
        sent: Vec<Status>,
        attempts: Vec<Status>,
        /// Each matching attempt consumes the front refusal.
        refuse: VecDeque<Refusal>,
        /// One-shot pause of the first `Running` attempt: signal "entered",
        /// then wait for "release" (bounded, so a failed test cannot hang).
        gate: Option<(Sender<()>, Receiver<()>)>,
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

        /// Like [`Self::refusing`], and the first `Running` attempt pauses
        /// until the test thread passes both returned barriers.
        fn gated_on_first_running(refusals: &[Refusal]) -> (Self, Receiver<()>, Sender<()>) {
            let handle = Self::refusing(refusals);
            let (entered_tx, entered_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();
            handle.log().gate = Some((entered_tx, release_rx));
            (handle, entered_rx, release_tx)
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
            if status == Status::Running {
                // Wait outside the log lock so the test thread can inspect it.
                let gate = self.0.lock().map_err(|_| "fake poisoned")?.gate.take();
                if let Some((entered, release)) = gate {
                    // Send/recv failures mean the test thread is gone or
                    // timed out; carry on so the scope can join.
                    let _ = entered.send(());
                    let _ = release.recv_timeout(GATE_TIMEOUT);
                }
            }
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

    /// Bound on every gate wait, so a missed signal fails instead of hanging.
    const GATE_TIMEOUT: Duration = Duration::from_secs(5);

    /// Runs `on_ready` on one thread and, once its first `Running` attempt is
    /// paused at the gate, issues the stop control on another. Returns whether
    /// the gate was reached and whether the state lock was held at that point;
    /// the caller asserts after both threads joined, and the gate is always
    /// released first, so a failed check cannot hang the test.
    fn race_stop_with_running_retry(
        session: &Session<FakeHandle>,
        entered: &Receiver<()>,
        release: &Sender<()>,
    ) -> (bool, bool) {
        std::thread::scope(|scope| {
            scope.spawn(|| session.on_ready().unwrap());
            let reached = entered.recv_timeout(GATE_TIMEOUT).is_ok();
            let held = reached && session.machine.try_lock().is_err();
            if reached {
                scope.spawn(|| session.on_stop_control().unwrap());
            }
            let _ = release.send(());
            (reached, held)
        })
    }

    fn is_pending(status: Status) -> bool {
        matches!(status, Status::Pending(_))
    }

    fn is_start_pending(status: &Status) -> bool {
        matches!(
            status,
            Status::Pending(Report {
                state: PendingState::StartPending,
                ..
            })
        )
    }

    fn stop_pending(checkpoint: u32) -> Status {
        Status::Pending(Report {
            state: PendingState::StopPending,
            wait_hint: STOP_HINT,
            checkpoint,
        })
    }

    /// Log text written by the fmt subscriber.
    #[derive(Clone, Default)]
    struct LogBuffer(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for LogBuffer {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .map_err(|_| std::io::Error::other("log buffer poisoned"))?
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogBuffer {
        type Writer = LogBuffer;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// Run `f` on this thread under a fmt subscriber and return what it logged.
    fn capture_logs(f: impl FnOnce()) -> String {
        let buffer = LogBuffer::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(buffer.clone())
            .with_ansi(false)
            .finish();
        tracing::subscriber::with_default(subscriber, f);
        let bytes = buffer.0.lock().unwrap().clone();
        String::from_utf8(bytes).unwrap()
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
    fn refused_pending_report_is_logged_with_state_and_checkpoint() {
        let handle = FakeHandle::refusing(&[is_pending]);
        let (session, _) = session(&handle);
        let logs = capture_logs(|| session.on_stop_control().unwrap());
        assert_eq!(handle.attempts(), [stop_pending(1)]);
        assert!(logs.contains("StopPending"), "{logs}");
        assert!(logs.contains("checkpoint 1"), "{logs}");
    }

    #[test]
    fn running_report_refused_once_is_retried() {
        let handle = FakeHandle::refusing(&[is_running]);
        let (session, stops) = session(&handle);
        let logs = capture_logs(|| session.on_ready().unwrap());
        assert_eq!(handle.attempts(), [Status::Running, Status::Running]);
        assert_eq!(handle.sent(), [Status::Running]);
        assert_eq!(stops.load(Ordering::SeqCst), 0);
        assert!(logs.contains("SCM refused status Running"), "{logs}");
        session.on_finished(0).unwrap();
        assert_eq!(handle.sent().last(), Some(&Status::Stopped(ScmExit::Clean)));
    }

    #[test]
    fn running_report_always_refused_ends_with_101() {
        let handle = FakeHandle::refusing(&[is_running as Refusal; 3]);
        let (session, stops) = session(&handle);
        session.on_ready().unwrap();
        assert_eq!(stops.load(Ordering::SeqCst), 1, "the drain was requested");
        assert_eq!(handle.sent(), [stop_pending(1)]);
        assert!(!session.tick().unwrap());
        let sent = handle.sent();
        assert!(!sent.iter().any(is_start_pending), "{sent:?}");
        assert_eq!(sent, [stop_pending(1), stop_pending(2)]);
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
    fn stop_request_while_running_is_retried_then_stops_cleanly() {
        let (handle, entered, release) = FakeHandle::gated_on_first_running(&[is_running]);
        let (session, stops) = session(&handle);
        let (entered, held) = race_stop_with_running_retry(&session, &entered, &release);
        assert!(entered, "the first Running attempt was reached");
        assert!(held, "the retry holds the state lock");
        assert!(!session.tick().unwrap());
        session.on_finished(0).unwrap();
        assert_eq!(stops.load(Ordering::SeqCst), 1);
        assert_eq!(
            handle.attempts(),
            [
                Status::Running,
                Status::Running,
                stop_pending(1),
                stop_pending(2),
                Status::Stopped(ScmExit::Clean),
            ]
        );
    }

    #[test]
    fn stop_request_while_running_keeps_failing_ends_with_101() {
        let (handle, entered, release) =
            FakeHandle::gated_on_first_running(&[is_running as Refusal; 3]);
        let (session, stops) = session(&handle);
        let (entered, _) = race_stop_with_running_retry(&session, &entered, &release);
        assert!(entered, "the first Running attempt was reached");
        assert_eq!(
            stops.load(Ordering::SeqCst),
            1,
            "the drain was requested once"
        );
        assert_eq!(
            handle.sent(),
            [stop_pending(1)],
            "the stop control adds no report"
        );
        assert!(!session.tick().unwrap());
        let sent = handle.sent();
        assert!(!sent.iter().any(is_start_pending), "{sent:?}");
        assert_eq!(sent, [stop_pending(1), stop_pending(2)]);
        // A requested stop's exit code still yields the fault's code.
        session.on_finished(130).unwrap();
        assert_eq!(
            handle.sent().last(),
            Some(&Status::Stopped(ScmExit::Specific(
                Fault::StatusReport.code()
            )))
        );
    }

    #[test]
    fn stopped_report_refused_once_is_retried() {
        let handle = FakeHandle::refusing(&[is_stopped]);
        let (session, _) = session(&handle);
        session.on_finished(2).unwrap();
        assert_eq!(handle.sent(), [Status::Stopped(ScmExit::Specific(2))]);
        assert_eq!(handle.attempts().len(), 2);
    }

    #[test]
    fn stopped_report_always_refused_exits_with_101() {
        let handle = FakeHandle::refusing(&[is_stopped as Refusal; 3]);
        let (session, _) = session(&handle);
        let logs = capture_logs(|| {
            assert_eq!(session.on_finished(0), Err(Fault::StatusReport));
        });
        assert!(logs.contains("Stopped"), "{logs}");
        assert!(logs.contains("giving up"), "{logs}");
        assert_eq!(session.abandon(Fault::StatusReport), 101);
        assert!(handle.sent().is_empty());
        assert_eq!(handle.attempts().len(), 3, "abandon does not resend");
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
    fn status_ticker_cannot_start_ends_with_102() {
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
    fn pending_report_refused_is_not_resent_and_next_tick_advances() {
        let handle = FakeHandle::refusing(&[is_pending]);
        let (session, _) = session(&handle);
        assert!(!session.tick().unwrap());
        assert_eq!(handle.attempts().len(), 1, "not resent");
        assert!(!session.tick().unwrap());
        let attempts = handle.attempts();
        let [Status::Pending(first), Status::Pending(second)] = attempts.as_slice() else {
            panic!("two pending attempts expected: {attempts:?}");
        };
        assert_eq!(first.state, PendingState::StartPending);
        assert!(second.checkpoint > first.checkpoint, "{attempts:?}");
        assert_eq!(handle.sent(), [attempts[1]], "the next tick reports again");
    }

    #[test]
    fn poisoned_service_state_ends_with_100() {
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
