//! Real Service Control Manager adapter: [`Scm`] implements
//! [`ScmPort`](super::windows_scm::ScmPort) on the `windows-service` crate.
//!
//! Registry layout (all under `HKLM\SYSTEM\CurrentControlSet\Services\<name>`):
//! `Description` (REG_SZ, written by `set_description`, read back here) and
//! `Environment` (REG_MULTI_SZ of `K=V` strings, written here; the SCM hands
//! it to the service process as its environment block).

#![allow(dead_code)] // TODO(Task 9 commit C): remove once wired

use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::ptr;

use anyhow::{Context, Result};
use windows_service::service::{
    Service, ServiceAccess, ServiceAction, ServiceActionType, ServiceErrorControl,
    ServiceFailureActions, ServiceFailureResetPeriod, ServiceInfo, ServiceStartType, ServiceState,
    ServiceType,
};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::Error as ScmError;
use windows_sys::Win32::System::Registry::{
    RegGetValueW, RegSetKeyValueW, HKEY_LOCAL_MACHINE, REG_MULTI_SZ, RRF_RT_REG_SZ,
};

use super::windows_scm::{ScmPort, ScmState, ServiceSpec};

/// `ERROR_FILE_NOT_FOUND`.
const ERROR_FILE_NOT_FOUND: u32 = 2;
/// `ERROR_PATH_NOT_FOUND`.
const ERROR_PATH_NOT_FOUND: u32 = 3;
/// `ERROR_SERVICE_DOES_NOT_EXIST`: the service is not registered.
const ERROR_SERVICE_DOES_NOT_EXIST: i32 = 1060;
/// `ERROR_SERVICE_MARKED_FOR_DELETE`: deleted but still held open or running.
const ERROR_SERVICE_MARKED_FOR_DELETE: i32 = 1072;
/// Restarts configured for the first, second and subsequent failures.
const RESTART_ACTIONS: usize = 3;

/// The real SCM on the local computer.
pub(crate) struct Scm;

/// Registry sub-key of the service `name`.
pub(crate) fn service_key(name: &str) -> Vec<u16> {
    wide(&format!(r"SYSTEM\CurrentControlSet\Services\{name}"))
}

/// NUL-terminated UTF-16 of `s`.
fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain([0]).collect()
}

/// REG_MULTI_SZ payload: each `K=V` NUL-terminated, plus a final NUL.
pub(crate) fn multi_sz(env: &[(String, String)]) -> Vec<u16> {
    let mut data: Vec<u16> = env
        .iter()
        .flat_map(|(k, v)| {
            let entry: Vec<u16> = format!("{k}={v}").encode_utf16().chain([0]).collect();
            entry
        })
        .collect();
    data.push(0);
    data
}

/// Raw OS error of a `windows-service` error, if it carries one.
fn raw_os_error(err: &ScmError) -> Option<i32> {
    match err {
        ScmError::Winapi(io) => io.raw_os_error(),
        _ => None,
    }
}

fn connect(access: ServiceManagerAccess) -> Result<ServiceManager> {
    ServiceManager::local_computer(None::<&str>, access)
        .context("cannot connect to the Service Control Manager")
}

/// Open `name`; `Ok(None)` when it is not registered.
fn open(name: &str, access: ServiceAccess) -> Result<Option<Service>> {
    let manager = connect(ServiceManagerAccess::CONNECT)?;
    match manager.open_service(name, access) {
        Ok(service) => Ok(Some(service)),
        Err(e) if raw_os_error(&e) == Some(ERROR_SERVICE_DOES_NOT_EXIST) => Ok(None),
        Err(e) => Err(e).with_context(|| format!("cannot open service `{name}`")),
    }
}

/// Open `name`, failing when it does not exist.
fn open_existing(name: &str, access: ServiceAccess) -> Result<Service> {
    open(name, access)?.with_context(|| format!("service `{name}` does not exist"))
}

fn map_state(state: ServiceState) -> ScmState {
    match state {
        ServiceState::Stopped => ScmState::Stopped,
        ServiceState::StartPending => ScmState::StartPending,
        ServiceState::StopPending => ScmState::StopPending,
        ServiceState::Running => ScmState::Running,
        ServiceState::ContinuePending | ServiceState::PausePending | ServiceState::Paused => {
            ScmState::Other
        }
    }
}

/// Write `Environment` for the service `name`.
fn write_environment(name: &str, env: &[(String, String)]) -> Result<()> {
    let key = service_key(name);
    let value = wide("Environment");
    let data = multi_sz(env);
    let bytes = u32::try_from(data.len() * 2).context("environment block too large")?;
    // SAFETY: `key` and `value` are NUL-terminated UTF-16 buffers and `data`
    // holds `bytes` readable bytes; all outlive the call, which copies them.
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value.as_ptr(),
            REG_MULTI_SZ,
            data.as_ptr().cast(),
            bytes,
        )
    };
    if status != 0 {
        return Err(std::io::Error::from_raw_os_error(status as i32))
            .with_context(|| format!("cannot write the environment of service `{name}`"));
    }
    Ok(())
}

/// Read the REG_SZ `value` under the service key; `None` when absent.
pub(crate) fn read_service_string(name: &str, value: &str) -> Result<Option<String>> {
    let key = service_key(name);
    let value_w = wide(value);
    let mut bytes: u32 = 0;
    // SAFETY: both names are NUL-terminated; a null data pointer with a
    // valid size pointer only queries the required size.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value_w.as_ptr(),
            RRF_RT_REG_SZ,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut bytes,
        )
    };
    if matches!(status, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) {
        return Ok(None);
    }
    if status != 0 {
        return Err(std::io::Error::from_raw_os_error(status as i32))
            .with_context(|| format!("cannot read `{value}` of service `{name}`"));
    }
    let mut buf = vec![0u16; (bytes as usize).div_ceil(2)];
    let mut capacity = u32::try_from(buf.len() * 2).context("registry value too large")?;
    // SAFETY: `buf` provides `capacity` writable bytes; `capacity` is updated
    // with the number of bytes written.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value_w.as_ptr(),
            RRF_RT_REG_SZ,
            ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut capacity,
        )
    };
    if status != 0 {
        return Err(std::io::Error::from_raw_os_error(status as i32))
            .with_context(|| format!("cannot read `{value}` of service `{name}`"));
    }
    let units = (capacity as usize / 2).min(buf.len());
    let text = OsString::from_wide(&buf[..units]);
    Ok(Some(
        text.to_string_lossy().trim_end_matches('\0').to_string(),
    ))
}

/// Description, environment and failure actions of a just-created service.
fn configure(service: &Service, spec: &ServiceSpec) -> Result<()> {
    service
        .set_description(&spec.description)
        .context("cannot set the service description")?;
    write_environment(&spec.name, &spec.env)?;
    if let Some(plan) = &spec.failure {
        service
            .update_failure_actions(ServiceFailureActions {
                reset_period: ServiceFailureResetPeriod::After(plan.reset_after),
                reboot_msg: None,
                command: None,
                actions: Some(vec![
                    ServiceAction {
                        action_type: ServiceActionType::Restart,
                        delay: plan.restart_after,
                    };
                    RESTART_ACTIONS
                ]),
            })
            .context("cannot set the failure actions")?;
        service
            .set_failure_actions_on_non_crash_failures(plan.on_non_crash_failures)
            .context("cannot enable failure actions on non-crash failures")?;
    }
    Ok(())
}

impl ScmPort for Scm {
    /// A service marked for delete cannot be opened (error 1072) but still
    /// exists until it stops and every handle closes; report it as `Other`
    /// so the orchestration's "wait until gone" keeps polling.
    fn state(&mut self, name: &str) -> Result<Option<ScmState>> {
        let manager = connect(ServiceManagerAccess::CONNECT)?;
        let service = match manager.open_service(name, ServiceAccess::QUERY_STATUS) {
            Ok(service) => service,
            Err(e) => {
                return match raw_os_error(&e) {
                    Some(ERROR_SERVICE_DOES_NOT_EXIST) => Ok(None),
                    Some(ERROR_SERVICE_MARKED_FOR_DELETE) => Ok(Some(ScmState::Other)),
                    _ => Err(e).with_context(|| format!("cannot open service `{name}`")),
                };
            }
        };
        let status = service
            .query_status()
            .with_context(|| format!("cannot query service `{name}`"))?;
        Ok(Some(map_state(status.current_state)))
    }

    fn create(&mut self, spec: &ServiceSpec) -> Result<()> {
        let manager =
            connect(ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE)?;
        let info = ServiceInfo {
            name: spec.name.clone().into(),
            display_name: spec.display_name.clone().into(),
            service_type: ServiceType::OWN_PROCESS,
            start_type: ServiceStartType::AutoStart,
            error_control: ServiceErrorControl::Normal,
            executable_path: spec.executable.clone(),
            launch_arguments: spec.launch_arguments.clone(),
            dependencies: vec![],
            account_name: None,
            account_password: None,
        };
        let access = ServiceAccess::CHANGE_CONFIG
            | ServiceAccess::START
            | ServiceAccess::STOP
            | ServiceAccess::DELETE
            | ServiceAccess::QUERY_STATUS
            | ServiceAccess::QUERY_CONFIG;
        let service = manager
            .create_service(&info, access)
            .with_context(|| format!("cannot create service `{}`", spec.name))?;
        if let Err(e) = configure(&service, spec) {
            // Never leave a half-configured service behind; keep the
            // original error even when the cleanup fails.
            let _ = service.delete();
            return Err(e.context(format!("service `{}` was not created", spec.name)));
        }
        Ok(())
    }

    fn delete(&mut self, name: &str) -> Result<()> {
        let service = open_existing(name, ServiceAccess::DELETE)?;
        service
            .delete()
            .with_context(|| format!("cannot delete service `{name}`"))
        // `service` drops here, so our own handle does not keep a delete pending.
    }

    fn start(&mut self, name: &str) -> Result<()> {
        let service = open_existing(name, ServiceAccess::START)?;
        service
            .start::<&OsStr>(&[])
            .with_context(|| format!("cannot start service `{name}`"))
    }

    fn stop(&mut self, name: &str) -> Result<()> {
        let service = open_existing(name, ServiceAccess::STOP | ServiceAccess::QUERY_STATUS)?;
        service
            .stop()
            .map(|_| ())
            .with_context(|| format!("cannot stop service `{name}`"))
    }

    fn description(&mut self, name: &str) -> Result<Option<String>> {
        read_service_string(name, "Description")
    }

    fn executable(&mut self, name: &str) -> Result<Option<PathBuf>> {
        let manager = connect(ServiceManagerAccess::CONNECT)?;
        let service = match manager.open_service(name, ServiceAccess::QUERY_CONFIG) {
            Ok(service) => service,
            Err(e) => {
                return match raw_os_error(&e) {
                    Some(ERROR_SERVICE_DOES_NOT_EXIST | ERROR_SERVICE_MARKED_FOR_DELETE) => {
                        Ok(None)
                    }
                    _ => Err(e).with_context(|| format!("cannot open service `{name}`")),
                };
            }
        };
        let config = service
            .query_config()
            .with_context(|| format!("cannot query the configuration of `{name}`"))?;
        Ok(Some(config.executable_path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::System::Registry::RRF_RT_REG_MULTI_SZ;

    /// Deletes the service on drop so a failing assertion leaves nothing.
    struct Cleanup(String);

    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = Scm.delete(&self.0);
        }
    }

    fn read_environment(name: &str) -> Vec<String> {
        let key = service_key(name);
        let value = wide("Environment");
        let mut bytes: u32 = 0;
        // SAFETY: NUL-terminated names; null data pointer only queries the size.
        let status = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_MULTI_SZ,
                ptr::null_mut(),
                ptr::null_mut(),
                &mut bytes,
            )
        };
        assert_eq!(status, 0, "size query failed");
        let mut buf = vec![0u16; (bytes as usize).div_ceil(2)];
        // SAFETY: `buf` provides `bytes` writable bytes.
        let status = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_MULTI_SZ,
                ptr::null_mut(),
                buf.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        assert_eq!(status, 0, "read failed");
        buf.split(|&u| u == 0)
            .filter(|s| !s.is_empty())
            .map(String::from_utf16_lossy)
            .collect()
    }

    #[test]
    fn multi_sz_terminates_every_entry_and_the_block() {
        let data = multi_sz(&[("A".into(), "1".into()), ("B".into(), "2".into())]);
        let expected: Vec<u16> = "A=1\0B=2\0\0".encode_utf16().collect();
        assert_eq!(data, expected);
    }

    #[test]
    fn scm_impl_round_trips_a_spec() {
        // Needs an elevated Windows session and mutates the SCM.
        if std::env::var("IROH_TUNNEL_E2E_SERVICE").as_deref() != Ok("1") {
            return;
        }
        let name = format!("iroh-tunnel-scmtest-{}", std::process::id());
        let exe = PathBuf::from(r"C:\Windows\System32\cmd.exe");
        let spec = ServiceSpec {
            name: name.clone(),
            display_name: "iroh-tunnel scm test".into(),
            executable: exe.clone(),
            launch_arguments: vec!["/c".into(), "exit".into()],
            env: vec![
                ("IROH_TUNNEL_SCM".into(), "1".into()),
                ("K".into(), "v w".into()),
            ],
            description: "scm test description".into(),
            failure: None,
        };
        let mut scm = Scm;
        assert_eq!(scm.state(&name).unwrap(), None);
        let _cleanup = Cleanup(name.clone());
        scm.create(&spec).unwrap();
        assert_eq!(scm.state(&name).unwrap(), Some(ScmState::Stopped));
        assert_eq!(
            scm.description(&name).unwrap().as_deref(),
            Some("scm test description")
        );
        assert_eq!(scm.executable(&name).unwrap(), Some(exe));
        assert_eq!(read_environment(&name), ["IROH_TUNNEL_SCM=1", "K=v w"]);
        scm.delete(&name).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while scm.state(&name).unwrap().is_some() {
            assert!(
                Instant::now() < deadline,
                "service still present after 10 s"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}
