//! The Windows service dispatcher: registers with the SCM, reports state and
//! runs the role through [`run_to_code`]. Windows only; the decisions live in
//! the pure parent module and [`Session`].

use std::ffi::OsString;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tracing_subscriber::fmt::writer::BoxMakeWriter;
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{
    self, ServiceControlHandlerResult, ServiceStatusHandle,
};
use windows_service::{define_windows_service, service_dispatcher};

use super::session::{Fault, Session, Status, StatusSink};
use super::{dispatcher_error_code, PendingState, ScmExit};
use crate::app::run_to_code;
use crate::cli::{init_tracing_with, Binary, Cli, Role};
use crate::lifecycle::scm_stop;
use iroh_tunnel_host::service::{machine_log_dir, system_service_name, CappedLog};

/// Log cap before the single `.1` rotation.
const LOG_CAP: u64 = 10 * 1024 * 1024;
/// Interval of the pending-checkpoint ticker.
const TICK: Duration = Duration::from_secs(1);
/// Pause between attempts of a refused `Running` or `Stopped` report.
const REPORT_RETRY: Duration = Duration::from_millis(200);
/// Controls accepted while running.
const ACCEPTED: ServiceControlAccept = ServiceControlAccept::STOP
    .union(ServiceControlAccept::SHUTDOWN)
    .union(ServiceControlAccept::PRESHUTDOWN);

/// What `service_main` needs, handed over from `run_as_service`.
static START: OnceLock<Mutex<Option<(Cli, Binary)>>> = OnceLock::new();

define_windows_service!(ffi_service_main, service_main);

impl StatusSink for ServiceStatusHandle {
    type Error = windows_service::Error;

    fn set(&self, status: Status) -> Result<(), Self::Error> {
        self.set_service_status(to_service_status(status))
    }
}

fn to_service_status(status: Status) -> ServiceStatus {
    let (current_state, checkpoint, wait_hint, exit) = match status {
        Status::Pending(report) => (
            match report.state {
                PendingState::StartPending => ServiceState::StartPending,
                PendingState::StopPending => ServiceState::StopPending,
            },
            report.checkpoint,
            report.wait_hint,
            ScmExit::Clean,
        ),
        Status::Running => (ServiceState::Running, 0, Duration::ZERO, ScmExit::Clean),
        Status::Stopped(exit) => (ServiceState::Stopped, 0, Duration::ZERO, exit),
    };
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state,
        controls_accepted: if status == Status::Running {
            ACCEPTED
        } else {
            ServiceControlAccept::empty()
        },
        exit_code: match exit {
            ScmExit::Clean => ServiceExitCode::Win32(0),
            ScmExit::Specific(code) => ServiceExitCode::ServiceSpecific(code),
        },
        checkpoint,
        wait_hint,
        process_id: None,
    }
}

type Shared = Session<ServiceStatusHandle>;

/// End the process after a fatal [`Fault`] in a path that cannot return it
/// to `service_main` (the control handler, the ticker, the ready hook).
fn abandon(shared: &Shared, fault: Fault) -> ! {
    std::process::exit(shared.abandon(fault))
}

/// Advance the pending checkpoint every second until the service is done.
/// While Running, `tick` yields nothing, so the once-a-second wake-up is a
/// cheap no-op (kept deliberately: it also covers a later stop's pending
/// state without extra signalling).
fn spawn_ticker(shared: Arc<Shared>) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("scm-ticker".into())
        .spawn(move || loop {
            std::thread::sleep(TICK);
            match shared.tick() {
                Ok(true) => return,
                Ok(false) => {}
                Err(fault) => abandon(&shared, fault),
            }
        })
        .map(drop)
}

/// Log writer of the service: the capped machine log, or nowhere when the
/// machine directory does not resolve or fails its trust check. A log file
/// that cannot be opened yet is retried by [`CappedLog`] itself.
fn log_writer(role: &str) -> BoxMakeWriter {
    match machine_log_dir() {
        Some(dir) => BoxMakeWriter::new(Mutex::new(CappedLog::open(&dir, role, LOG_CAP))),
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
        crate::output::report("error: only the serve and access roles run as a Windows service\n");
        std::process::exit(2);
    };
    let name = system_service_name(role);
    let slot = START.get_or_init(|| Mutex::new(None));
    match slot.lock() {
        Ok(mut pending) => *pending = Some((parsed, binary)),
        Err(_) => {
            crate::output::report("error: the pending service command lock is poisoned\n");
            std::process::exit(1);
        }
    }
    match service_dispatcher::start(name, ffi_service_main) {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            crate::output::report(&format!(
                "error: cannot connect to the Service Control Manager: {e}\n"
            ));
            let raw = match &e {
                windows_service::Error::Winapi(io) => io.raw_os_error(),
                _ => None,
            };
            std::process::exit(dispatcher_error_code(raw));
        }
    }
}

/// The command handed over by [`run_as_service`], if any.
fn take_pending() -> Option<(Cli, Binary)> {
    match START.get()?.lock() {
        Ok(mut pending) => pending.take(),
        Err(_) => {
            crate::output::report("error: the pending service command lock is poisoned\n");
            None
        }
    }
}

fn service_main(_arguments: Vec<OsString>) {
    let Some((parsed, binary)) = take_pending() else {
        crate::output::report("error: service started without a pending command\n");
        return;
    };
    let Some(role) = role_name(&parsed) else {
        crate::output::report("error: only the serve and access roles run as a Windows service\n");
        return;
    };
    let shared: Arc<Shared> = Arc::new(Session::new(
        Box::new(|| {
            scm_stop().send_replace(true);
        }),
        REPORT_RETRY,
    ));

    let for_handler = shared.clone();
    let registered =
        service_control_handler::register(
            system_service_name(role),
            move |control| match control {
                ServiceControl::Stop | ServiceControl::Shutdown | ServiceControl::Preshutdown => {
                    if let Err(fault) = for_handler.on_stop_control() {
                        abandon(&for_handler, fault);
                    }
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
            crate::output::report(&format!(
                "error: cannot register the service control handler: {e}\n"
            ));
            return;
        }
    };
    shared.attach(handle);
    if let Err(fault) = shared.tick() {
        abandon(&shared, fault);
    }

    let hook_shared = shared.clone();
    // Tracing setup and the ticker spawn are inside the unwind guard so a
    // failure still ends in the single Stopped report.
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        init_tracing_with(parsed.verbose, parsed.quiet, log_writer(role));
        if let Err(e) = spawn_ticker(shared.clone()) {
            tracing::error!("cannot start the service status ticker: {e}");
            return Err(Fault::TickerSpawn);
        }
        let on_ready = move || {
            if let Err(fault) = hook_shared.on_ready() {
                abandon(&hook_shared, fault);
            }
        };
        Ok(run_to_code(parsed, binary, Some(Box::new(on_ready)), true))
    }));
    let finished = match outcome {
        Ok(Ok(code)) => shared.on_finished(code),
        Ok(Err(fault)) => shared.fail_start(fault),
        Err(_panic) => shared.on_finished(1),
    };
    if let Err(fault) = finished {
        abandon(&shared, fault);
    }
}
