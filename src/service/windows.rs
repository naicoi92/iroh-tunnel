use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

use windows_service::service::{
    Service, ServiceAccess, ServiceAction as ScAction, ServiceActionType, ServiceErrorControl,
    ServiceFailureActions, ServiceInfo, ServiceStartType, ServiceState, ServiceType,
};
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceStatus,
};
use windows_service::service_control_handler::{
    self, ServiceControlHandlerResult, ServiceStatusHandle,
};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{define_windows_service, service_dispatcher};

use crate::status::StatusRole;

use super::{resolve_binary, RestartPolicy, ServiceScope};

const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

const FAILURE_RESET: Duration = Duration::from_secs(86_400);

const STOP_POLL_INTERVAL: Duration = Duration::from_millis(100);

const STOP_POLL_TIMEOUT: Duration = Duration::from_secs(60);

/// Wait until the SCM reports the service stopped: `Service::stop` only
/// delivers the control and returns, so an immediate `start` would race it.
fn ensure_stopped(service: &Service) -> Result<()> {
    let state = service.query_status()?.current_state;
    if state == ServiceState::Stopped {
        return Ok(());
    }
    // A service already draining rejects a second stop control.
    if state != ServiceState::StopPending {
        service.stop().context("failed to stop the service")?;
    }
    wait_until_stopped(service, STOP_POLL_TIMEOUT)
}

fn wait_until_stopped(service: &Service, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        let state = service
            .query_status()
            .context("failed to query service status while stopping")?
            .current_state;
        if state == ServiceState::Stopped {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!(
                "service did not stop within {}s (last reported state: {state:?})",
                timeout.as_secs()
            );
        }
        std::thread::sleep(STOP_POLL_INTERVAL);
    }
}

fn admin_only_roots() -> Vec<PathBuf> {
    ["ProgramFiles", "ProgramFiles(x86)", "SystemRoot"]
        .iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .collect()
}

fn normalize_for_prefix(path: &Path) -> String {
    let s = path
        .to_string_lossy()
        .to_ascii_lowercase()
        .replace('/', "\\");
    match s.strip_prefix("\\\\?\\") {
        Some(rest) => rest.to_string(),
        None => s,
    }
}

/// True when `binary` sits under one of `roots`, matched on a component
/// boundary so `C:\Program Files Evil` does not count as `C:\Program Files`.
fn is_under_any(binary: &Path, roots: &[PathBuf]) -> bool {
    let binary = normalize_for_prefix(binary);
    roots.iter().any(|root| {
        let root = normalize_for_prefix(root);
        let root = root.trim_end_matches('\\');
        !root.is_empty()
            && binary
                .strip_prefix(root)
                .is_some_and(|rest| rest.starts_with('\\'))
    })
}

fn is_admin_only(path: &Path) -> bool {
    is_under_any(path, &admin_only_roots())
}

fn warn_unless_binary_is_admin_only(binary: &Path) {
    if is_admin_only(binary) {
        return;
    }
    eprintln!(
        "warning: {} is outside Program Files and the Windows directory, so it may be replaceable \
         by a non-administrator; the service runs as LocalSystem, so that would hand out \
         SYSTEM. Copy the binary somewhere only administrators can write and install from \
         there.",
        binary.display()
    );
}

fn warn_unless_config_is_admin_only(config: &Path) {
    if is_admin_only(config) {
        return;
    }
    eprintln!(
        "warning: {} is outside Program Files and the Windows directory, so it may be writable by a \
         non-administrator; the service re-reads it as LocalSystem on every start, so that \
         would let a non-administrator choose what a SYSTEM process exposes. Move the config \
         somewhere only administrators can write and install with --config pointing there.",
        config.display()
    );
}

fn service_name(role: &str) -> String {
    format!("iroh-tunnel-{role}")
}

fn display_name(role: &str) -> String {
    format!("Iroh Tunnel ({role})")
}

fn launch_arguments(role: &str, config: &Path) -> Vec<OsString> {
    vec![
        OsString::from(role),
        OsString::from("run"),
        OsString::from("--config"),
        config.as_os_str().to_os_string(),
        OsString::from("--service"),
    ]
}

fn failure_actions(policy: &RestartPolicy) -> ServiceFailureActions {
    let action = if policy.on_failure {
        ScAction {
            action_type: ServiceActionType::Restart,
            delay: Duration::from_secs(policy.delay_secs),
        }
    } else {
        ScAction {
            action_type: ServiceActionType::None,
            delay: Duration::ZERO,
        }
    };
    ServiceFailureActions {
        reset_period: windows_service::service::ServiceFailureResetPeriod::After(FAILURE_RESET),
        reboot_msg: None,
        command: None,
        actions: Some(vec![action.clone(), action.clone(), action]),
    }
}

fn require_system_scope(scope: ServiceScope) -> Result<()> {
    if scope == ServiceScope::User {
        bail!(
            "Windows has no per-user services: the Service Control Manager is machine-wide. \
             Re-run with --system from an elevated prompt."
        );
    }
    Ok(())
}

fn manager(access: ServiceManagerAccess) -> Result<ServiceManager> {
    ServiceManager::local_computer(None::<&OsStr>, access)
        .context("failed to open the Service Control Manager")
}

fn control_context(role: &str) -> String {
    format!(
        "failed to open service {} (it may not be installed, or controlling it requires an \
         elevated prompt)",
        service_name(role)
    )
}

pub fn install(role: &str, scope: ServiceScope, config: &Path) -> Result<()> {
    require_system_scope(scope)?;
    let binary: PathBuf = resolve_binary()?;
    warn_unless_binary_is_admin_only(&binary);
    warn_unless_config_is_admin_only(config);
    let mgr = manager(ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE)?;

    let info = ServiceInfo {
        name: OsString::from(service_name(role)),
        display_name: OsString::from(display_name(role)),
        service_type: SERVICE_TYPE,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: binary,
        launch_arguments: launch_arguments(role, config),
        dependencies: vec![],
        account_name: None, // LocalSystem
        account_password: None,
    };

    let service = mgr
        .create_service(
            &info,
            ServiceAccess::CHANGE_CONFIG | ServiceAccess::START | ServiceAccess::DELETE,
        )
        .with_context(|| {
            format!(
                "failed to create service {} (it may already exist, or installing requires \
                 an elevated prompt)",
                service_name(role)
            )
        })?;

    if let Err(e) = configure_and_start(&service) {
        return Err(rollback_install(&service, role, e));
    }
    println!("installed and started {}", service_name(role));
    Ok(())
}

fn configure_and_start(service: &Service) -> Result<()> {
    service
        .update_failure_actions(failure_actions(&RestartPolicy::DEFAULT))
        .context("failed to set service recovery actions")?;
    service
        .set_failure_actions_on_non_crash_failures(RestartPolicy::DEFAULT.on_failure)
        .context("failed to enable recovery actions for non-crash exits")?;
    service
        .start(&[] as &[&OsStr])
        .context("the service was created but failed to start")
}

// A half-created service blocks the next install with a name conflict.
fn rollback_install(service: &Service, role: &str, err: anyhow::Error) -> anyhow::Error {
    if let Err(e) = service.delete() {
        eprintln!(
            "warning: could not remove the partly created service {}: {e}",
            service_name(role)
        );
    }
    err
}

pub fn uninstall(role: &str, scope: ServiceScope) -> Result<()> {
    require_system_scope(scope)?;
    let mgr = manager(ServiceManagerAccess::CONNECT)?;
    let service = mgr
        .open_service(
            service_name(role),
            ServiceAccess::STOP | ServiceAccess::DELETE | ServiceAccess::QUERY_STATUS,
        )
        .with_context(|| control_context(role))?;

    ensure_stopped(&service)?;
    service.delete().context("failed to delete the service")?;
    println!("uninstalled {}", service_name(role));
    Ok(())
}

pub fn start(role: &str, scope: ServiceScope) -> Result<()> {
    require_system_scope(scope)?;
    let mgr = manager(ServiceManagerAccess::CONNECT)?;
    mgr.open_service(service_name(role), ServiceAccess::START)
        .with_context(|| control_context(role))?
        .start(&[] as &[&OsStr])
        .context("failed to start the service")?;
    Ok(())
}

pub fn stop(role: &str, scope: ServiceScope) -> Result<()> {
    require_system_scope(scope)?;
    let mgr = manager(ServiceManagerAccess::CONNECT)?;
    let service = mgr
        .open_service(
            service_name(role),
            ServiceAccess::STOP | ServiceAccess::QUERY_STATUS,
        )
        .with_context(|| control_context(role))?;
    ensure_stopped(&service)
}

pub fn restart(role: &str, scope: ServiceScope) -> Result<()> {
    stop(role, scope)?;
    start(role, scope)
}

pub fn status(role: &str, scope: ServiceScope) -> Result<()> {
    require_system_scope(scope)?;
    let mgr = manager(ServiceManagerAccess::CONNECT)?;
    let status = mgr
        .open_service(service_name(role), ServiceAccess::QUERY_STATUS)
        .with_context(|| {
            format!(
                "failed to open service {} (it may not be installed)",
                service_name(role)
            )
        })?
        .query_status()
        .context("failed to query service status")?;
    println!("{}: {:?}", service_name(role), status.current_state);
    Ok(())
}

const STOP_WAIT_HINT: Duration = Duration::from_secs(10);

const STOP_PROGRESS_INTERVAL: Duration = Duration::from_secs(2);

// A Win32(0) stop reads to the SCM as a clean shutdown, so the recovery actions
// `install` configures would never run.
const SERVICE_FAILURE_EXIT: ServiceExitCode = ServiceExitCode::ServiceSpecific(1);

fn service_status(
    state: ServiceState,
    accepted: ServiceControlAccept,
    wait_hint: Duration,
) -> ServiceStatus {
    ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: state,
        controls_accepted: accepted,
        exit_code: ServiceExitCode::NO_ERROR,
        checkpoint: 0,
        wait_hint,
        process_id: None,
    }
}

static SERVICE_ARGS: OnceLock<(StatusRole, PathBuf)> = OnceLock::new();

define_windows_service!(ffi_service_main, service_main);

pub fn run_as_service(role: StatusRole, config: &Path) -> Result<()> {
    SERVICE_ARGS
        .set((role, config.to_path_buf()))
        .map_err(|_| anyhow::anyhow!("service arguments were already set"))?;
    service_dispatcher::start(service_name(role.name()), ffi_service_main)
        .context("failed to connect to the service control dispatcher")?;
    // The dispatcher returns once `service_main` has, so its failure is ours.
    match SERVICE_OUTCOME
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
    {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

static SERVICE_OUTCOME: Mutex<Option<anyhow::Error>> = Mutex::new(None);

fn service_main(_args: Vec<OsString>) {
    let Err(err) = service_body() else { return };
    tracing::error!("service exited with an error: {err:#}");
    *SERVICE_OUTCOME.lock().unwrap_or_else(|e| e.into_inner()) = Some(err);
}

// The SCM reads a checkpoint that stops advancing as a hung service. Holding
// the lock across the report keeps a stale StopPending from landing after the
// final Stopped.
fn report_stop_progress(handle: ServiceStatusHandle, drained: &Mutex<bool>) {
    let mut checkpoint = 1;
    loop {
        std::thread::sleep(STOP_PROGRESS_INTERVAL);
        let done = drained.lock().unwrap_or_else(|e| e.into_inner());
        if *done {
            return;
        }
        let mut status = service_status(
            ServiceState::StopPending,
            ServiceControlAccept::empty(),
            STOP_WAIT_HINT,
        );
        status.checkpoint = checkpoint;
        let _ = handle.set_service_status(status);
        checkpoint += 1;
    }
}

fn service_body() -> Result<()> {
    let (role, config) = SERVICE_ARGS
        .get()
        .context("service started without captured arguments")?;

    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let stop_tx = Mutex::new(Some(stop_tx));

    let status_handle: Arc<OnceLock<ServiceStatusHandle>> = Arc::new(OnceLock::new());
    let handler_handle = Arc::clone(&status_handle);
    let drained = Arc::new(Mutex::new(false));
    let handler_drained = Arc::clone(&drained);

    let handle =
        service_control_handler::register(
            service_name(role.name()),
            move |control| match control {
                ServiceControl::Stop | ServiceControl::Shutdown => {
                    let mut slot = stop_tx.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(tx) = slot.take() {
                        let _ = tx.send(());
                        if let Some(&h) = handler_handle.get() {
                            let _ = h.set_service_status(service_status(
                                ServiceState::StopPending,
                                ServiceControlAccept::empty(),
                                STOP_WAIT_HINT,
                            ));
                            let drained = Arc::clone(&handler_drained);
                            std::thread::spawn(move || report_stop_progress(h, &drained));
                        }
                    }
                    ServiceControlHandlerResult::NoError
                }
                ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
                _ => ServiceControlHandlerResult::NotImplemented,
            },
        )
        .context("failed to register the service control handler")?;
    let _ = status_handle.set(handle);

    let outcome = run_until_stopped(handle, stop_rx, *role, config);

    let mut final_status = service_status(
        ServiceState::Stopped,
        ServiceControlAccept::empty(),
        Duration::ZERO,
    );
    if outcome.is_err() {
        final_status.exit_code = SERVICE_FAILURE_EXIT;
    }
    let mut done = drained.lock().unwrap_or_else(|e| e.into_inner());
    *done = true;
    if let Err(e) = handle.set_service_status(final_status) {
        tracing::error!("failed to report the final service status: {e}");
    }
    outcome
}

// Split out so every `Result` error between StartPending and the roles
// returning funnels into service_body's single Stopped report.
fn run_until_stopped(
    handle: ServiceStatusHandle,
    stop_rx: tokio::sync::oneshot::Receiver<()>,
    role: StatusRole,
    config: &Path,
) -> Result<()> {
    handle.set_service_status(service_status(
        ServiceState::StartPending,
        ServiceControlAccept::empty(),
        Duration::from_secs(30),
    ))?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to start the Tokio runtime")?;

    handle.set_service_status(service_status(
        ServiceState::Running,
        ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        Duration::ZERO,
    ))?;

    rt.block_on(async {
        let shutdown = async {
            let _ = stop_rx.await;
        };
        match role {
            StatusRole::Serve => crate::serve::run_with_shutdown(config, shutdown).await,
            StatusRole::Access => crate::access::run_with_shutdown(config, shutdown).await,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_and_display_names_are_role_scoped() {
        assert_eq!(service_name("serve"), "iroh-tunnel-serve");
        assert_eq!(display_name("access"), "Iroh Tunnel (access)");
    }

    #[test]
    fn launch_arguments_carry_role_config_and_service_flag() {
        let args = launch_arguments("serve", Path::new(r"C:\cfg\serve.toml"));
        let args: Vec<_> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            vec![
                "serve",
                "run",
                "--config",
                r"C:\cfg\serve.toml",
                "--service"
            ]
        );
    }

    #[test]
    fn launch_arguments_end_with_the_service_flag() {
        let args = launch_arguments("access", Path::new("cfg.toml"));
        assert_eq!(args.last().unwrap(), "--service");
    }

    #[test]
    fn default_policy_maps_to_three_restart_actions() {
        let actions = failure_actions(&RestartPolicy::DEFAULT).actions.unwrap();
        assert_eq!(actions.len(), 3);
        for a in &actions {
            assert_eq!(a.action_type, ServiceActionType::Restart);
            assert_eq!(
                a.delay,
                Duration::from_secs(RestartPolicy::DEFAULT.delay_secs)
            );
        }
    }

    #[test]
    fn disabled_policy_maps_to_no_action() {
        let policy = RestartPolicy {
            on_failure: false,
            delay_secs: 5,
        };
        let actions = failure_actions(&policy).actions.unwrap();
        for a in &actions {
            assert_eq!(a.action_type, ServiceActionType::None);
        }
    }

    #[test]
    fn admin_only_match_is_case_insensitive_and_component_aligned() {
        let roots = vec![PathBuf::from(r"C:\Program Files")];
        assert!(is_under_any(
            Path::new(r"c:\program files\iroh-tunnel\iroh-tunnel.exe"),
            &roots
        ));
        assert!(!is_under_any(
            Path::new(r"C:\Program Files Evil\iroh-tunnel.exe"),
            &roots
        ));
    }

    #[test]
    fn build_trees_and_profiles_are_not_admin_only() {
        let roots = vec![
            PathBuf::from(r"C:\Program Files"),
            PathBuf::from(r"C:\Windows"),
        ];
        assert!(!is_under_any(
            Path::new(r"E:\rust-target\release\iroh-tunnel.exe"),
            &roots
        ));
        assert!(!is_under_any(
            Path::new(r"C:\Users\h\dev\iroh-tunnel.exe"),
            &roots
        ));
    }

    #[test]
    fn prefix_match_tolerates_extended_length_paths_and_forward_slashes() {
        let roots = vec![PathBuf::from(r"C:\Program Files")];
        assert!(is_under_any(
            Path::new(r"\\?\C:\Program Files\iroh-tunnel\iroh-tunnel.exe"),
            &roots
        ));
        assert!(is_under_any(
            Path::new("C:/Program Files/iroh-tunnel/iroh-tunnel.exe"),
            &roots
        ));
    }

    #[test]
    fn user_scope_is_rejected_with_an_actionable_message() {
        let err = require_system_scope(ServiceScope::User)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("--system"),
            "message should name the fix: {err}"
        );
        assert!(require_system_scope(ServiceScope::System).is_ok());
    }

    #[test]
    fn service_status_reports_success_by_default() {
        let status = service_status(
            ServiceState::Running,
            ServiceControlAccept::STOP,
            Duration::ZERO,
        );
        assert_eq!(status.exit_code, ServiceExitCode::NO_ERROR);
        assert_eq!(status.checkpoint, 0);
    }

    #[test]
    fn failure_exit_code_never_reads_as_a_clean_stop() {
        assert_ne!(SERVICE_FAILURE_EXIT, ServiceExitCode::NO_ERROR);
        assert_ne!(SERVICE_FAILURE_EXIT, ServiceExitCode::Win32(0));
    }
}
