//! The Windows service dispatcher: registers with the SCM, reports state and
//! runs the role through [`run_to_code`]. Windows only; the decisions live in
//! the pure parent module.

use std::ffi::OsString;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use tracing_subscriber::fmt::writer::BoxMakeWriter;
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{
    self, ServiceControlHandlerResult, ServiceStatusHandle,
};
use windows_service::{define_windows_service, service_dispatcher};

use super::{
    dispatcher_error_code, exit_to_scm, PendingState, Report, ScmExit, ScmMachine, StopLatch,
};
use crate::app::run_to_code;
use crate::cli::{init_tracing_with, Binary, Cli, Role};
use crate::lifecycle::scm_stop;
use iroh_tunnel_host::service::{machine_log_dir, system_service_name, CappedLog};

/// Log cap before the single `.1` rotation.
const LOG_CAP: u64 = 10 * 1024 * 1024;
/// Interval of the pending-checkpoint ticker.
const TICK: Duration = Duration::from_secs(1);
/// Controls accepted while running.
const ACCEPTED: ServiceControlAccept = ServiceControlAccept::STOP
    .union(ServiceControlAccept::SHUTDOWN)
    .union(ServiceControlAccept::PRESHUTDOWN);

/// What `service_main` needs, handed over from `run_as_service`.
static START: OnceLock<Mutex<Option<(Cli, Binary)>>> = OnceLock::new();

define_windows_service!(ffi_service_main, service_main);

/// State shared by `service_main`, the control handler and the ticker.
struct Shared {
    handle: OnceLock<ServiceStatusHandle>,
    machine: Mutex<ScmMachine>,
    latch: StopLatch,
}

impl Shared {
    fn machine(&self) -> MutexGuard<'_, ScmMachine> {
        self.machine.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn send(&self, status: ServiceStatus) {
        if let Some(handle) = self.handle.get() {
            // A refused report cannot be retried usefully; the next tick or
            // the final Stopped report tells the SCM the truth.
            let _ = handle.set_service_status(status);
        }
    }

    fn send_pending(&self, report: Report) {
        let current_state = match report.state {
            PendingState::StartPending => ServiceState::StartPending,
            PendingState::StopPending => ServiceState::StopPending,
        };
        self.send(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state,
            controls_accepted: ServiceControlAccept::empty(),
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: report.checkpoint,
            wait_hint: report.wait_hint,
            process_id: None,
        });
    }

    fn send_state(&self, current_state: ServiceState, exit: ScmExit) {
        let running = current_state == ServiceState::Running;
        self.send(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state,
            controls_accepted: if running {
                ACCEPTED
            } else {
                ServiceControlAccept::empty()
            },
            exit_code: match exit {
                ScmExit::Clean => ServiceExitCode::Win32(0),
                ScmExit::Specific(code) => ServiceExitCode::ServiceSpecific(code),
            },
            checkpoint: 0,
            wait_hint: Duration::ZERO,
            process_id: None,
        });
    }

    /// A stop, shutdown or pre-shutdown control: the first one begins the drain.
    fn on_stop_control(&self) {
        if self.latch.set() {
            scm_stop().send_replace(true);
        }
        let mut machine = self.machine();
        if let Some(report) = machine.on_stop() {
            self.send_pending(report);
        }
    }

    /// The role is ready: report Running unless a stop is already in flight.
    fn on_ready(&self) {
        let mut machine = self.machine();
        if machine.on_ready() {
            self.send_state(ServiceState::Running, ScmExit::Clean);
        }
    }

    /// The run ended: report Stopped exactly once.
    fn on_finished(&self, code: i32) {
        let mut machine = self.machine();
        if machine.finish() {
            let exit = exit_to_scm(code, self.latch.was_set());
            self.send_state(ServiceState::Stopped, exit);
        }
    }
}

/// Advance the pending checkpoint every second until the service is done.
/// While Running, `tick` yields nothing, so the once-a-second wake-up is a
/// cheap no-op (kept deliberately: it also covers a later stop's pending
/// state without extra signalling).
fn spawn_ticker(shared: Arc<Shared>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(TICK);
        let mut machine = shared.machine();
        if machine.is_done() {
            return;
        }
        if let Some(report) = machine.tick() {
            shared.send_pending(report);
        }
    });
}

/// Log writer of the service: the capped machine log, or nowhere.
fn log_writer(role: &str) -> BoxMakeWriter {
    let log = machine_log_dir().and_then(|dir| CappedLog::open(&dir, role, LOG_CAP));
    match log {
        Some(log) => BoxMakeWriter::new(Mutex::new(log)),
        None => BoxMakeWriter::new(std::io::sink),
    }
}

fn role_name(parsed: &Cli) -> Option<&'static str> {
    match parsed.role {
        Role::Serve { .. } => Some("serve"),
        Role::Access { .. } => Some("access"),
        Role::Update(_) => None,
    }
}

/// Hand the process to the Service Control Manager and exit when the service
/// ends. Never returns: a failure to connect exits with
/// [`dispatcher_error_code`].
pub(crate) fn run_as_service(parsed: Cli, binary: Binary) -> ! {
    let Some(role) = role_name(&parsed) else {
        eprintln!("error: only the serve and access roles run as a Windows service");
        std::process::exit(2);
    };
    let name = system_service_name(role);
    let slot = START.get_or_init(|| Mutex::new(None));
    *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some((parsed, binary));
    match service_dispatcher::start(name, ffi_service_main) {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("error: cannot connect to the Service Control Manager: {e}");
            let raw = match &e {
                windows_service::Error::Winapi(io) => io.raw_os_error(),
                _ => None,
            };
            std::process::exit(dispatcher_error_code(raw));
        }
    }
}

fn service_main(_arguments: Vec<OsString>) {
    let taken = START
        .get()
        .and_then(|slot| slot.lock().unwrap_or_else(|e| e.into_inner()).take());
    let Some((parsed, binary)) = taken else {
        eprintln!("error: service started without a pending command");
        return;
    };
    let Some(role) = role_name(&parsed) else {
        eprintln!("error: only the serve and access roles run as a Windows service");
        return;
    };
    let shared = Arc::new(Shared {
        handle: OnceLock::new(),
        machine: Mutex::new(ScmMachine::default()),
        latch: StopLatch::default(),
    });

    let for_handler = shared.clone();
    let registered =
        service_control_handler::register(
            system_service_name(role),
            move |control| match control {
                ServiceControl::Stop | ServiceControl::Shutdown | ServiceControl::Preshutdown => {
                    for_handler.on_stop_control();
                    ServiceControlHandlerResult::NoError
                }
                ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
                _ => ServiceControlHandlerResult::NotImplemented,
            },
        );
    let handle = match registered {
        Ok(handle) => handle,
        Err(e) => {
            // No status handle: the SCM cannot be told anything.
            eprintln!("error: cannot register the service control handler: {e}");
            return;
        }
    };
    let _ = shared.handle.set(handle);

    {
        let mut machine = shared.machine();
        if let Some(report) = machine.tick() {
            shared.send_pending(report);
        }
    }
    spawn_ticker(shared.clone());

    let hook_shared = shared.clone();
    // Tracing setup is inside the unwind guard so a failure still ends in the
    // single Stopped report.
    let code = catch_unwind(AssertUnwindSafe(|| {
        init_tracing_with(parsed.verbose, parsed.quiet, log_writer(role));
        run_to_code(
            parsed,
            binary,
            Some(Box::new(move || hook_shared.on_ready())),
            true,
        )
    }))
    .unwrap_or(1);
    shared.on_finished(code);
}
