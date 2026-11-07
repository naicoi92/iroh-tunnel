//! Windows Service definition and SCM orchestration (pure part).
//!
//! Everything here compiles and is tested on every platform: the service
//! definition ([`ServiceSpec`], [`FailurePlan`]), the [`ScmPort`] seam and the
//! orchestration functions generic over it. The real adapter on the
//! `windows-service` crate is added separately and implements [`ScmPort`].
//! Time and the role-lock wait are injected so tests never sleep.

#![cfg_attr(not(windows), allow(dead_code))]
// Nothing consumes this module until the SCM adapter and verb wiring land.
#![cfg_attr(windows, allow(dead_code))]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::error::CliError;
use crate::service::trust_windows::ENV_TRUSTED_SID;
use crate::service::types::RestartPolicy;
use crate::service::{Pin, PinOwner, ENV_SCM};
use crate::status::writer::ENV_STATE_DIR;

/// Poll interval while waiting for a state change.
const POLL: Duration = Duration::from_millis(100);
/// Install waits this long for `Running`.
const RUNNING_TIMEOUT: Duration = Duration::from_secs(30);
/// Stop waits this long for `Stopped`.
const STOPPED_TIMEOUT: Duration = Duration::from_secs(20);
/// Delete waits this long until the service is gone.
const GONE_TIMEOUT: Duration = Duration::from_secs(10);
/// Restart waits this long for the role lock to be released.
pub(crate) const LOCK_TIMEOUT: Duration = Duration::from_secs(15);
/// The failure counter resets after this long without a failure.
const FAILURE_RESET: Duration = Duration::from_secs(86_400);

/// SCM name of the role's service (plus the test-only instance suffix).
pub(crate) fn service_name(role: &str) -> String {
    format!(
        "iroh-tunnel-{role}{}",
        crate::service::registration::instance_suffix()
    )
}

/// Failure actions of a service: restart after a crash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FailurePlan {
    /// Delay before the SCM restarts the service.
    pub restart_after: Duration,
    /// Time without failures after which the failure counter resets.
    pub reset_after: Duration,
    /// Also act on a non-zero exit that is not a crash.
    pub on_non_crash_failures: bool,
}

impl FailurePlan {
    /// The plan for `policy`; `None` when it asks for no restart.
    pub(crate) fn from_policy(policy: &RestartPolicy) -> Option<Self> {
        policy.on_failure.then(|| Self {
            restart_after: Duration::from_secs(policy.delay_secs),
            reset_after: FAILURE_RESET,
            on_non_crash_failures: true,
        })
    }
}

/// Everything the SCM needs to create the service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServiceSpec {
    pub name: String,
    pub display_name: String,
    pub executable: PathBuf,
    /// Raw arguments, unquoted: the `windows-service` crate quotes them when
    /// it builds the SCM command line.
    pub launch_arguments: Vec<OsString>,
    /// Service environment, in fixed order.
    pub env: Vec<(String, String)>,
    /// The pin record, so a later install can detect retargeting.
    pub description: String,
    pub failure: Option<FailurePlan>,
}

impl ServiceSpec {
    /// Render the definition of the `role` service.
    ///
    /// # Errors
    ///
    /// [`CliError::Service`] when `pin` is not a SID pin.
    pub(crate) fn render(
        role: &str,
        executable: &Path,
        config: &Path,
        state: &Path,
        pin: &Pin,
        policy: &RestartPolicy,
    ) -> Result<Self> {
        let PinOwner::Sid(sid) = &pin.owner else {
            return Err(CliError::Service(
                "internal: Windows service registration needs a SID pin".into(),
            )
            .into());
        };
        Ok(Self {
            name: service_name(role),
            display_name: format!("iroh-tunnel {role}"),
            executable: executable.to_path_buf(),
            launch_arguments: vec![
                "serve".into(),
                "run".into(),
                "--config".into(),
                config.as_os_str().to_owned(),
            ],
            env: vec![
                (ENV_STATE_DIR.into(), state.to_string_lossy().into_owned()),
                (ENV_TRUSTED_SID.into(), sid.clone()),
                (ENV_SCM.into(), "1".into()),
            ],
            description: pin.record(),
            failure: FailurePlan::from_policy(policy),
        })
    }
}

/// Coarse state of a service as the orchestration sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScmState {
    Stopped,
    StartPending,
    StopPending,
    Running,
    /// Paused, continue-pending or any other state.
    Other,
}

/// Result of [`status_with`]; the adapter maps it to `ServiceStatusReport`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScmStatus {
    /// No such service.
    Absent,
    /// The service exists in this state.
    Installed(ScmState),
}

/// The Service Control Manager operations the orchestration needs.
pub(crate) trait ScmPort {
    /// State of `name`; `None` when it does not exist.
    fn state(&mut self, name: &str) -> Result<Option<ScmState>>;
    fn create(&mut self, spec: &ServiceSpec) -> Result<()>;
    fn delete(&mut self, name: &str) -> Result<()>;
    fn start(&mut self, name: &str) -> Result<()>;
    fn stop(&mut self, name: &str) -> Result<()>;
    /// Description of `name` (the pin record), if any.
    fn description(&mut self, name: &str) -> Result<Option<String>>;
    /// Executable of `name`'s command line, if any.
    fn executable(&mut self, name: &str) -> Result<Option<PathBuf>>;
}

/// Monotonic time source and sleeper, injected so tests need no real time.
pub(crate) trait Clock {
    /// Time elapsed since an arbitrary fixed origin.
    fn now(&mut self) -> Duration;
    fn sleep(&mut self, d: Duration);
}

/// The wall clock.
pub(crate) struct SystemClock(Instant);

impl SystemClock {
    pub(crate) fn new() -> Self {
        Self(Instant::now())
    }
}

impl Clock for SystemClock {
    fn now(&mut self) -> Duration {
        self.0.elapsed()
    }
    fn sleep(&mut self, d: Duration) {
        std::thread::sleep(d);
    }
}

fn not_installed(name: &str, role: &str) -> anyhow::Error {
    CliError::Service(format!(
        "service {name} is not installed; run `iroh-tunnel {role} service install --system`"
    ))
    .into()
}

/// Poll until `done(state)` holds or `timeout` passes. `Ok(Err(last))` carries
/// the last observed state on timeout.
fn wait_for(
    port: &mut dyn ScmPort,
    clock: &mut dyn Clock,
    name: &str,
    timeout: Duration,
    done: impl Fn(Option<ScmState>) -> bool,
) -> Result<std::result::Result<(), Option<ScmState>>> {
    let deadline = clock.now() + timeout;
    loop {
        let state = port.state(name)?;
        if done(state) {
            return Ok(Ok(()));
        }
        if clock.now() >= deadline {
            return Ok(Err(state));
        }
        clock.sleep(POLL);
    }
}

fn wait_or_fail(
    port: &mut dyn ScmPort,
    clock: &mut dyn Clock,
    name: &str,
    timeout: Duration,
    wanted: &str,
    done: impl Fn(Option<ScmState>) -> bool,
) -> Result<()> {
    match wait_for(port, clock, name, timeout, done)? {
        Ok(()) => Ok(()),
        Err(last) => {
            let reached = last.map_or_else(|| "absent".to_string(), |s| format!("{s:?}"));
            Err(CliError::Service(format!(
                "service {name} did not become {wanted} within {}s (state: {reached})",
                timeout.as_secs()
            ))
            .into())
        }
    }
}

fn wait_running(port: &mut dyn ScmPort, clock: &mut dyn Clock, name: &str) -> Result<()> {
    wait_or_fail(port, clock, name, RUNNING_TIMEOUT, "Running", |s| {
        s == Some(ScmState::Running)
    })
}

/// Stop `name` if it runs and wait for `Stopped`. A missing service is fine.
fn stop_existing(port: &mut dyn ScmPort, clock: &mut dyn Clock, name: &str) -> Result<()> {
    match port.state(name)? {
        None | Some(ScmState::Stopped) => return Ok(()),
        Some(ScmState::StopPending) => {}
        Some(_) => port.stop(name)?,
    }
    wait_or_fail(port, clock, name, STOPPED_TIMEOUT, "Stopped", |s| {
        matches!(s, None | Some(ScmState::Stopped))
    })
}

fn delete_and_wait(port: &mut dyn ScmPort, clock: &mut dyn Clock, name: &str) -> Result<()> {
    port.delete(name)?;
    wait_or_fail(port, clock, name, GONE_TIMEOUT, "gone", |s| s.is_none())
}

/// Create `spec`, replacing an existing service, and wait for `Running`.
/// A failed start rolls the registration back and returns the original error.
pub(crate) fn install_with(
    port: &mut dyn ScmPort,
    clock: &mut dyn Clock,
    spec: &ServiceSpec,
) -> Result<()> {
    let name = &spec.name;
    if port.state(name)?.is_some() {
        stop_existing(port, clock, name)?;
        delete_and_wait(port, clock, name)?;
    }
    port.create(spec)?;
    let started = port
        .start(name)
        .and_then(|()| wait_running(port, clock, name));
    if let Err(err) = started {
        // Best effort: the start failure is the error worth reporting.
        let _ = delete_and_wait(port, clock, name);
        return Err(err);
    }
    Ok(())
}

/// Stop the role's service; already stopped is fine.
///
/// # Errors
///
/// [`CliError::Service`] when the service is absent or does not stop in time.
pub(crate) fn stop_with(port: &mut dyn ScmPort, clock: &mut dyn Clock, role: &str) -> Result<()> {
    let name = service_name(role);
    if port.state(&name)?.is_none() {
        return Err(not_installed(&name, role));
    }
    stop_existing(port, clock, &name)
}

/// Stop and delete the role's service; absent is fine.
pub(crate) fn uninstall_with(
    port: &mut dyn ScmPort,
    clock: &mut dyn Clock,
    role: &str,
) -> Result<()> {
    let name = service_name(role);
    if port.state(&name)?.is_none() {
        return Ok(());
    }
    stop_existing(port, clock, &name)?;
    delete_and_wait(port, clock, &name)
}

/// Start the role's service and wait for `Running`.
///
/// # Errors
///
/// [`CliError::Service`] when the service is absent or never runs.
pub(crate) fn start_with(port: &mut dyn ScmPort, clock: &mut dyn Clock, role: &str) -> Result<()> {
    let name = service_name(role);
    if port.state(&name)?.is_none() {
        return Err(not_installed(&name, role));
    }
    port.start(&name)?;
    wait_running(port, clock, &name)
}

/// Stop, wait for the role lock via `wait_released(LOCK_TIMEOUT)`, then start.
/// When the wait fails the service is not started.
pub(crate) fn restart_with(
    port: &mut dyn ScmPort,
    clock: &mut dyn Clock,
    role: &str,
    wait_released: impl FnOnce(Duration) -> Result<()>,
) -> Result<()> {
    stop_with(port, clock, role)?;
    wait_released(LOCK_TIMEOUT)?;
    start_with(port, clock, role)
}

/// State of the role's service.
pub(crate) fn status_with(port: &mut dyn ScmPort, role: &str) -> Result<ScmStatus> {
    Ok(port
        .state(&service_name(role))?
        .map_or(ScmStatus::Absent, ScmStatus::Installed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::PinOwner;
    use std::cell::RefCell;
    use std::rc::Rc;

    const SID: &str = "S-1-5-21-1-2-3-1001";

    fn pin(config: &Path) -> Pin {
        Pin {
            config: config.to_path_buf(),
            owner: PinOwner::Sid(SID.into()),
        }
    }

    fn spec() -> ServiceSpec {
        ServiceSpec::render(
            "serve",
            Path::new(r"C:\bin\iroh-tunnel.exe"),
            Path::new(r"C:\cfg\serve.toml"),
            Path::new(r"C:\ProgramData\iroh-tunnel"),
            &pin(Path::new(r"C:\cfg\serve.toml")),
            &RestartPolicy::DEFAULT,
        )
        .unwrap()
    }

    #[derive(Default)]
    struct FakeScm {
        state: Option<ScmState>,
        calls: Rc<RefCell<Vec<String>>>,
        /// State after `start`; `StartPending` simulates a service that never runs.
        after_start: Option<ScmState>,
        fail_start: bool,
    }

    impl FakeScm {
        fn with(state: Option<ScmState>) -> Self {
            Self {
                state,
                after_start: Some(ScmState::Running),
                ..Self::default()
            }
        }
    }

    impl ScmPort for FakeScm {
        fn state(&mut self, _: &str) -> Result<Option<ScmState>> {
            Ok(self.state)
        }
        fn create(&mut self, _: &ServiceSpec) -> Result<()> {
            self.calls.borrow_mut().push("create".into());
            self.state = Some(ScmState::Stopped);
            Ok(())
        }
        fn delete(&mut self, _: &str) -> Result<()> {
            self.calls.borrow_mut().push("delete".into());
            self.state = None;
            Ok(())
        }
        fn start(&mut self, _: &str) -> Result<()> {
            self.calls.borrow_mut().push("start".into());
            if self.fail_start {
                return Err(CliError::Service("start boom".into()).into());
            }
            self.state = self.after_start;
            Ok(())
        }
        fn stop(&mut self, _: &str) -> Result<()> {
            self.calls.borrow_mut().push("stop".into());
            self.state = Some(ScmState::Stopped);
            Ok(())
        }
        fn description(&mut self, _: &str) -> Result<Option<String>> {
            Ok(None)
        }
        fn executable(&mut self, _: &str) -> Result<Option<PathBuf>> {
            Ok(None)
        }
    }

    #[derive(Default)]
    struct FakeClock {
        now: Duration,
    }

    impl Clock for FakeClock {
        fn now(&mut self) -> Duration {
            self.now
        }
        fn sleep(&mut self, d: Duration) {
            self.now += d;
        }
    }

    fn service_msg(err: anyhow::Error) -> String {
        match err.downcast_ref::<CliError>() {
            Some(CliError::Service(m)) => m.clone(),
            other => panic!("expected Service, got {other:?}"),
        }
    }

    #[test]
    fn service_spec_for_serve() {
        let s = spec();
        assert_eq!(s.name, "iroh-tunnel-serve");
        assert_eq!(s.display_name, "iroh-tunnel serve");
        assert_eq!(
            s.launch_arguments,
            ["serve", "run", "--config", r"C:\cfg\serve.toml"].map(OsString::from)
        );
        assert_eq!(
            s.env,
            vec![
                (
                    "IROH_TUNNEL_STATE_DIR".to_string(),
                    r"C:\ProgramData\iroh-tunnel".to_string()
                ),
                ("IROH_TUNNEL_TRUSTED_SID".to_string(), SID.to_string()),
                ("IROH_TUNNEL_SCM".to_string(), "1".to_string()),
            ]
        );
        assert_eq!(s.description, pin(Path::new(r"C:\cfg\serve.toml")).record());
        assert!(s.failure.is_some());
    }

    #[test]
    fn service_spec_keeps_paths_with_spaces_and_unicode() {
        let exe = Path::new(r"C:\Program Files\iroh-tunnel\iroh-tunnel.exe");
        let cfg = Path::new(r"C:\Users\Nguyễn Văn\AppData\Local\iroh-tunnel\config\serve.toml");
        let s = ServiceSpec::render(
            "serve",
            exe,
            cfg,
            Path::new(r"C:\ProgramData\iroh-tunnel"),
            &pin(cfg),
            &RestartPolicy::DEFAULT,
        )
        .unwrap();
        assert_eq!(s.executable, exe);
        assert_eq!(s.launch_arguments[3], OsString::from(cfg));
    }

    #[test]
    fn failure_plan_from_default_policy() {
        assert_eq!(
            FailurePlan::from_policy(&RestartPolicy::DEFAULT),
            Some(FailurePlan {
                restart_after: Duration::from_secs(5),
                reset_after: Duration::from_secs(86_400),
                on_non_crash_failures: true,
            })
        );
    }

    #[test]
    fn failure_plan_disabled_is_none() {
        let p = RestartPolicy {
            on_failure: false,
            delay_secs: 5,
        };
        assert_eq!(FailurePlan::from_policy(&p), None);
    }

    #[test]
    fn render_requires_a_sid_pin() {
        let cfg = Path::new("/etc/cfg.toml");
        let uid = Pin {
            config: cfg.to_path_buf(),
            owner: PinOwner::Uid(1000),
        };
        let err =
            ServiceSpec::render("serve", cfg, cfg, cfg, &uid, &RestartPolicy::DEFAULT).unwrap_err();
        assert!(service_msg(err).contains("needs a SID pin"));
    }

    #[test]
    fn install_replaces_existing() {
        let mut port = FakeScm::with(Some(ScmState::Running));
        install_with(&mut port, &mut FakeClock::default(), &spec()).unwrap();
        assert_eq!(*port.calls.borrow(), ["stop", "delete", "create", "start"]);
        assert_eq!(port.state, Some(ScmState::Running));
    }

    #[test]
    fn install_rolls_back_when_start_fails() {
        let mut port = FakeScm::with(None);
        port.fail_start = true;
        let err = install_with(&mut port, &mut FakeClock::default(), &spec()).unwrap_err();
        assert_eq!(service_msg(err), "start boom");
        assert_eq!(*port.calls.borrow(), ["create", "start", "delete"]);
        assert_eq!(port.state, None);
    }

    #[test]
    fn install_rolls_back_when_never_running() {
        let mut port = FakeScm::with(None);
        port.after_start = Some(ScmState::StartPending);
        let mut clock = FakeClock::default();
        let err = install_with(&mut port, &mut clock, &spec()).unwrap_err();
        let msg = service_msg(err);
        assert!(
            msg.contains("iroh-tunnel-serve") && msg.contains("StartPending"),
            "{msg}"
        );
        assert!(clock.now >= Duration::from_secs(30));
        assert_eq!(
            port.calls.borrow().last().map(String::as_str),
            Some("delete")
        );
        assert_eq!(port.state, None);
    }

    #[test]
    fn stop_of_stopped_is_ok() {
        let mut port = FakeScm::with(Some(ScmState::Stopped));
        stop_with(&mut port, &mut FakeClock::default(), "serve").unwrap();
        assert!(port.calls.borrow().is_empty());
    }

    #[test]
    fn stop_stops_a_running_service() {
        let mut port = FakeScm::with(Some(ScmState::Running));
        stop_with(&mut port, &mut FakeClock::default(), "serve").unwrap();
        assert_eq!(*port.calls.borrow(), ["stop"]);
    }

    #[test]
    fn stop_absent_is_install_hint() {
        let mut port = FakeScm::with(None);
        let err = stop_with(&mut port, &mut FakeClock::default(), "serve").unwrap_err();
        assert!(service_msg(err).contains("service install --system"));
    }

    #[test]
    fn uninstall_absent_is_ok() {
        let mut port = FakeScm::with(None);
        uninstall_with(&mut port, &mut FakeClock::default(), "serve").unwrap();
        assert!(port.calls.borrow().is_empty());
    }

    #[test]
    fn uninstall_removes_a_running_service() {
        let mut port = FakeScm::with(Some(ScmState::Running));
        uninstall_with(&mut port, &mut FakeClock::default(), "serve").unwrap();
        assert_eq!(*port.calls.borrow(), ["stop", "delete"]);
    }

    #[test]
    fn start_absent_is_service_error_with_install_hint() {
        let mut port = FakeScm::with(None);
        let err = start_with(&mut port, &mut FakeClock::default(), "serve").unwrap_err();
        assert!(service_msg(err).contains("service install --system"));
    }

    #[test]
    fn start_waits_for_running() {
        let mut port = FakeScm::with(Some(ScmState::Stopped));
        start_with(&mut port, &mut FakeClock::default(), "serve").unwrap();
        assert_eq!(*port.calls.borrow(), ["start"]);
    }

    #[test]
    fn status_reports_absent_and_state() {
        let mut port = FakeScm::with(None);
        assert_eq!(status_with(&mut port, "serve").unwrap(), ScmStatus::Absent);
        port.state = Some(ScmState::Running);
        assert_eq!(
            status_with(&mut port, "serve").unwrap(),
            ScmStatus::Installed(ScmState::Running)
        );
    }

    #[test]
    fn restart_waits_for_lock_then_starts() {
        let mut port = FakeScm::with(Some(ScmState::Running));
        let calls = Rc::clone(&port.calls);
        let mut seen_at_wait = None;
        restart_with(&mut port, &mut FakeClock::default(), "serve", |t| {
            assert_eq!(t, Duration::from_secs(15));
            seen_at_wait = Some(calls.borrow().clone());
            Ok(())
        })
        .unwrap();
        assert_eq!(seen_at_wait.unwrap(), ["stop"]);
        assert_eq!(*port.calls.borrow(), ["stop", "start"]);
    }

    #[test]
    fn restart_times_out_without_starting() {
        let mut port = FakeScm::with(Some(ScmState::Running));
        let err = restart_with(&mut port, &mut FakeClock::default(), "serve", |_| {
            Err(CliError::Service("still running".into()).into())
        })
        .unwrap_err();
        assert_eq!(service_msg(err), "still running");
        assert_eq!(*port.calls.borrow(), ["stop"]);
    }
}
