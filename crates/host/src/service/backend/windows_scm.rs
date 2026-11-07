//! Windows Service definition and SCM orchestration (pure part).
//!
//! Everything here compiles and is tested on every platform: the service
//! definition ([`ServiceSpec`], [`FailurePlan`]), the [`ScmPort`] seam and the
//! orchestration functions generic over it. The real adapter on the
//! `windows-service` crate is added separately and implements [`ScmPort`].
//! Time and the role-lock wait are injected so tests never sleep.

// Pure and compiled everywhere for its tests, but only Windows consumes it.
#![cfg_attr(not(windows), allow(dead_code))]

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
    /// [`CliError::Service`] when `role` is not `serve`/`access` or `pin` is
    /// not a SID pin.
    pub(crate) fn render(
        role: &str,
        executable: &Path,
        config: &Path,
        state: &Path,
        pin: &Pin,
        policy: &RestartPolicy,
    ) -> Result<Self> {
        if !matches!(role, "serve" | "access") {
            return Err(CliError::Service(format!(
                "internal: unknown service role `{role}` (expected serve or access)"
            ))
            .into());
        }
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
                role.into(),
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

impl std::fmt::Display for ScmState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Stopped => "stopped",
            Self::StartPending => "start pending",
            Self::StopPending => "stop pending",
            Self::Running => "running",
            Self::Other => "other",
        })
    }
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

pub(crate) fn not_installed(name: &str, role: &str) -> anyhow::Error {
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

/// Wait for `Running`; fail at once when the service falls back to `Stopped`
/// (it crashed on start) or disappears.
fn wait_running(port: &mut dyn ScmPort, clock: &mut dyn Clock, name: &str) -> Result<()> {
    let deadline = clock.now() + RUNNING_TIMEOUT;
    loop {
        let state = port.state(name)?;
        match state {
            Some(ScmState::Running) => return Ok(()),
            None | Some(ScmState::Stopped) => {
                let seen = state.map_or_else(|| "absent".to_string(), |s| format!("{s:?}"));
                return Err(CliError::Service(format!(
                    "service {name} stopped right after start (state: {seen}); \
                     check its configuration and the Windows event log"
                ))
                .into());
            }
            Some(_) if clock.now() >= deadline => {
                return Err(CliError::Service(format!(
                    "service {name} did not become Running within {}s (state: {state:?})",
                    RUNNING_TIMEOUT.as_secs()
                ))
                .into());
            }
            Some(_) => clock.sleep(POLL),
        }
    }
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

/// Stop (waiting for `Stopped`), then delete and wait until gone: a delete of
/// a running service only marks it, so the stop must come first.
fn stop_and_delete(port: &mut dyn ScmPort, clock: &mut dyn Clock, name: &str) -> Result<()> {
    stop_existing(port, clock, name)?;
    delete_and_wait(port, clock, name)
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
        stop_and_delete(port, clock, name)?;
    }
    port.create(spec)?;
    let started = port
        .start(name)
        .and_then(|()| wait_running(port, clock, name));
    if let Err(err) = started {
        // Best effort: the start failure is the error worth reporting.
        let _ = stop_and_delete(port, clock, name);
        return Err(err);
    }
    Ok(())
}

/// Stop the role's service; already stopped is fine. Returns whether the
/// service was already stopped when called.
///
/// # Errors
///
/// [`CliError::Service`] when the service is absent or does not stop in time.
pub(crate) fn stop_with(port: &mut dyn ScmPort, clock: &mut dyn Clock, role: &str) -> Result<bool> {
    let name = service_name(role);
    let Some(state) = port.state(&name)? else {
        return Err(not_installed(&name, role));
    };
    stop_existing(port, clock, &name)?;
    Ok(state == ScmState::Stopped)
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
    stop_and_delete(port, clock, &name)
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
    let _already_stopped = stop_with(port, clock, role)?;
    wait_released(LOCK_TIMEOUT)?;
    start_with(port, clock, role)
}

/// The text a retarget decision reads from an existing registration.
///
/// `None` only when the service is absent. A service that exists without a
/// readable description yields `Some("")`, which carries no pin: an unknown
/// owner, never "nothing installed".
pub(crate) fn existing_registration_body(
    port: &mut dyn ScmPort,
    name: &str,
) -> Result<Option<String>> {
    if port.state(name)?.is_none() {
        return Ok(None);
    }
    Ok(Some(port.description(name)?.unwrap_or_default()))
}

/// Whether the role's SCM service exists (any state, including pending
/// delete). A failed query is an error, never "absent".
pub(crate) fn service_registered(port: &mut dyn ScmPort, role: &str) -> Result<bool> {
    Ok(port.state(&service_name(role))?.is_some())
}

/// State of the role's service.
pub(crate) fn status_with(port: &mut dyn ScmPort, role: &str) -> Result<ScmStatus> {
    Ok(port
        .state(&service_name(role))?
        .map_or(ScmStatus::Absent, ScmStatus::Installed))
}

/// First argument (argv[0]) of a Windows command line, following the
/// `CommandLineToArgvW` rule for the program name: a quoted token runs to the
/// next `"` (no escapes; an unterminated quote takes the rest), an unquoted
/// one to the first whitespace. `None` when the line is empty.
pub(crate) fn first_command_line_arg(cmd: &str) -> Option<String> {
    let cmd = cmd.trim_start();
    if cmd.is_empty() {
        return None;
    }
    if let Some(rest) = cmd.strip_prefix('"') {
        let end = rest.find('"').unwrap_or(rest.len());
        return Some(rest[..end].to_owned());
    }
    let end = cmd.find(char::is_whitespace).unwrap_or(cmd.len());
    Some(cmd[..end].to_owned())
}

/// Scriptable in-memory SCM shared by this module's and the system verb glue's tests.
#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use crate::error::CliError;
    use std::cell::RefCell;
    use std::collections::{HashMap, HashSet, VecDeque};
    use std::rc::Rc;

    pub(crate) const SERVE: &str = "iroh-tunnel-serve";

    /// One registered service in the fake.
    pub(crate) struct Svc {
        pub(crate) current: ScmState,
        /// States returned by successive `state()` polls; the last one sticks.
        pub(crate) script: VecDeque<ScmState>,
        pub(crate) marked_for_delete: bool,
    }

    /// Scriptable SCM: per-name services whose state advances on every poll,
    /// `delete` that only marks a not-stopped service, one-shot failures.
    pub(crate) struct FakeScm {
        pub(crate) services: HashMap<String, Svc>,
        pub(crate) calls: Rc<RefCell<Vec<String>>>,
        /// Script installed by `start`.
        pub(crate) start_script: Vec<ScmState>,
        /// Script installed by `stop`.
        pub(crate) stop_script: Vec<ScmState>,
        /// A marked service never disappears (open handles keep it alive).
        pub(crate) never_gone: bool,
        /// Operations (`"stop"`, `"delete"`, `"create"`, `"start"`) to fail once.
        pub(crate) fail_once: HashSet<&'static str>,
        /// What `description` returns for any existing service.
        pub(crate) description: Option<String>,
        /// What `executable` returns for any existing service.
        pub(crate) exe: Option<PathBuf>,
    }

    impl FakeScm {
        pub(crate) fn with(state: Option<ScmState>) -> Self {
            let mut fake = Self {
                services: HashMap::new(),
                calls: Rc::default(),
                start_script: vec![ScmState::Running],
                stop_script: vec![ScmState::Stopped],
                never_gone: false,
                fail_once: HashSet::new(),
                description: None,
                exe: None,
            };
            if let Some(state) = state {
                fake.add(SERVE, state, &[]);
            }
            fake
        }

        pub(crate) fn add(&mut self, name: &str, current: ScmState, script: &[ScmState]) {
            self.services.insert(
                name.to_string(),
                Svc {
                    current,
                    script: script.iter().copied().collect(),
                    marked_for_delete: false,
                },
            );
        }

        pub(crate) fn record(&self, op: &'static str) -> Result<()> {
            self.calls.borrow_mut().push(op.into());
            if self.fail_once.contains(op) {
                return Err(CliError::Service(format!("{op} boom")).into());
            }
            Ok(())
        }

        pub(crate) fn svc(&mut self, name: &str) -> Result<&mut Svc> {
            self.services
                .get_mut(name)
                .ok_or_else(|| CliError::Service(format!("no such service {name}")).into())
        }

        pub(crate) fn current(&self) -> Option<ScmState> {
            self.services.get(SERVE).map(|s| s.current)
        }
    }

    impl ScmPort for FakeScm {
        fn state(&mut self, name: &str) -> Result<Option<ScmState>> {
            let never_gone = self.never_gone;
            let Some(svc) = self.services.get_mut(name) else {
                return Ok(None);
            };
            if let Some(next) = svc.script.pop_front() {
                svc.current = next;
            }
            let state = svc.current;
            if svc.marked_for_delete && state == ScmState::Stopped && !never_gone {
                self.services.remove(name);
                return Ok(None);
            }
            Ok(Some(state))
        }
        fn create(&mut self, spec: &ServiceSpec) -> Result<()> {
            self.record("create")?;
            if self.services.contains_key(&spec.name) {
                return Err(CliError::Service("already exists".into()).into());
            }
            self.add(&spec.name, ScmState::Stopped, &[]);
            Ok(())
        }
        fn delete(&mut self, name: &str) -> Result<()> {
            self.record("delete")?;
            let never_gone = self.never_gone;
            let svc = self.svc(name)?;
            if svc.current == ScmState::Stopped && svc.script.is_empty() && !never_gone {
                self.services.remove(name);
            } else {
                svc.marked_for_delete = true;
            }
            Ok(())
        }
        fn start(&mut self, name: &str) -> Result<()> {
            self.record("start")?;
            let script = self.start_script.iter().copied().collect();
            self.svc(name)?.script = script;
            Ok(())
        }
        fn stop(&mut self, name: &str) -> Result<()> {
            self.record("stop")?;
            let script = self.stop_script.iter().copied().collect();
            self.svc(name)?.script = script;
            Ok(())
        }
        fn description(&mut self, _: &str) -> Result<Option<String>> {
            Ok(self.description.clone())
        }
        fn executable(&mut self, name: &str) -> Result<Option<PathBuf>> {
            Ok(self
                .services
                .contains_key(name)
                .then(|| self.exe.clone())
                .flatten())
        }
    }

    #[derive(Default)]
    pub(crate) struct FakeClock {
        pub(crate) now: Duration,
    }

    impl Clock for FakeClock {
        fn now(&mut self) -> Duration {
            self.now
        }
        fn sleep(&mut self, d: Duration) {
            self.now += d;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{FakeClock, FakeScm, SERVE};
    use super::*;
    use crate::service::PinOwner;
    use std::rc::Rc;

    const SID: &str = "S-1-5-21-1-2-3-1001";
    fn pin(config: &Path) -> Pin {
        Pin {
            config: config.to_path_buf(),
            owner: PinOwner::Sid(SID.into()),
        }
    }

    fn spec_for(role: &str) -> ServiceSpec {
        let cfg = format!(r"C:\cfg\{role}.toml");
        ServiceSpec::render(
            role,
            Path::new(r"C:\bin\iroh-tunnel.exe"),
            Path::new(&cfg),
            Path::new(r"C:\ProgramData\iroh-tunnel"),
            &pin(Path::new(&cfg)),
            &RestartPolicy::DEFAULT,
        )
        .unwrap()
    }

    fn spec() -> ServiceSpec {
        spec_for("serve")
    }

    #[test]
    fn service_registered_reflects_scm_state() {
        assert!(!service_registered(&mut FakeScm::with(None), "serve").unwrap());
        assert!(service_registered(&mut FakeScm::with(Some(ScmState::Stopped)), "serve").unwrap());
        assert!(service_registered(&mut FakeScm::with(Some(ScmState::Running)), "serve").unwrap());
    }

    #[test]
    fn registration_body_is_none_only_when_absent() {
        let mut port = FakeScm::with(None);
        assert_eq!(existing_registration_body(&mut port, SERVE).unwrap(), None);
    }

    #[test]
    fn registration_body_returns_the_pin_description() {
        let mut port = FakeScm::with(Some(ScmState::Running));
        port.description = Some("iroh-tunnel-pin config=C:/a.toml sid=S-1-5-18".into());
        assert_eq!(
            existing_registration_body(&mut port, SERVE)
                .unwrap()
                .as_deref(),
            Some("iroh-tunnel-pin config=C:/a.toml sid=S-1-5-18")
        );
    }

    #[test]
    fn registration_body_of_a_pinless_service_is_empty_not_absent() {
        let mut port = FakeScm::with(Some(ScmState::Stopped));
        assert_eq!(
            existing_registration_body(&mut port, SERVE)
                .unwrap()
                .as_deref(),
            Some("")
        );
    }

    fn service_msg(err: anyhow::Error) -> String {
        match err.downcast_ref::<CliError>() {
            Some(CliError::Service(m)) => m.clone(),
            other => panic!("expected Service, got {other:?}"),
        }
    }

    fn calls(port: &FakeScm) -> Vec<String> {
        port.calls.borrow().clone()
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
    fn service_spec_for_access() {
        let s = spec_for("access");
        assert_eq!(s.name, "iroh-tunnel-access");
        assert_eq!(s.display_name, "iroh-tunnel access");
        assert_eq!(
            s.launch_arguments,
            ["access", "run", "--config", r"C:\cfg\access.toml"].map(OsString::from)
        );
    }

    #[test]
    fn render_rejects_unknown_role() {
        let cfg = Path::new(r"C:\cfg\x.toml");
        let err = ServiceSpec::render("bogus", cfg, cfg, cfg, &pin(cfg), &RestartPolicy::DEFAULT)
            .unwrap_err();
        assert!(service_msg(err).contains("bogus"));
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
        assert_eq!(calls(&port), ["stop", "delete", "create", "start"]);
        assert_eq!(port.current(), Some(ScmState::Running));
    }

    #[test]
    fn install_waits_through_pending_states() {
        let mut port = FakeScm::with(Some(ScmState::Running));
        port.stop_script = vec![
            ScmState::StopPending,
            ScmState::StopPending,
            ScmState::Stopped,
        ];
        port.start_script = vec![
            ScmState::StartPending,
            ScmState::StartPending,
            ScmState::Running,
        ];
        let mut clock = FakeClock::default();
        install_with(&mut port, &mut clock, &spec()).unwrap();
        assert_eq!(calls(&port), ["stop", "delete", "create", "start"]);
        assert_eq!(port.current(), Some(ScmState::Running));
        assert!(clock.now > Duration::ZERO && clock.now < Duration::from_secs(5));
    }

    #[test]
    fn install_rolls_back_when_start_fails() {
        let mut port = FakeScm::with(None);
        port.fail_once.insert("start");
        let err = install_with(&mut port, &mut FakeClock::default(), &spec()).unwrap_err();
        assert_eq!(service_msg(err), "start boom");
        assert_eq!(calls(&port), ["create", "start", "delete"]);
        assert_eq!(port.current(), None);
    }

    #[test]
    fn install_rolls_back_a_service_stuck_in_start_pending() {
        let mut port = FakeScm::with(None);
        port.start_script = vec![ScmState::StartPending];
        let mut clock = FakeClock::default();
        let err = install_with(&mut port, &mut clock, &spec()).unwrap_err();
        let msg = service_msg(err);
        assert!(msg.contains(SERVE) && msg.contains("StartPending"), "{msg}");
        assert!(clock.now >= Duration::from_secs(30));
        // Stopped before deleted, so the delete really removes it.
        assert_eq!(calls(&port), ["create", "start", "stop", "delete"]);
        assert_eq!(port.current(), None);
        assert!(port.services.is_empty());
    }

    #[test]
    fn install_fails_fast_when_service_stops_after_start() {
        let mut port = FakeScm::with(None);
        port.start_script = vec![ScmState::StartPending, ScmState::Stopped];
        let mut clock = FakeClock::default();
        let err = install_with(&mut port, &mut clock, &spec()).unwrap_err();
        let msg = service_msg(err);
        assert!(msg.contains("Stopped"), "{msg}");
        assert!(clock.now < Duration::from_secs(5), "{:?}", clock.now);
        assert!(port.services.is_empty());
    }

    #[test]
    fn install_returns_original_error_when_rollback_fails() {
        let mut port = FakeScm::with(None);
        port.fail_once.insert("start");
        port.never_gone = true;
        let err = install_with(&mut port, &mut FakeClock::default(), &spec()).unwrap_err();
        assert_eq!(service_msg(err), "start boom");
    }

    #[test]
    fn install_stop_failure_keeps_the_old_service() {
        let mut port = FakeScm::with(Some(ScmState::Running));
        port.fail_once.insert("stop");
        let err = install_with(&mut port, &mut FakeClock::default(), &spec()).unwrap_err();
        assert_eq!(service_msg(err), "stop boom");
        assert_eq!(calls(&port), ["stop"]);
        assert_eq!(port.current(), Some(ScmState::Running));
    }

    #[test]
    fn install_delete_failure_creates_nothing() {
        let mut port = FakeScm::with(Some(ScmState::Running));
        port.fail_once.insert("delete");
        let err = install_with(&mut port, &mut FakeClock::default(), &spec()).unwrap_err();
        assert_eq!(service_msg(err), "delete boom");
        assert_eq!(calls(&port), ["stop", "delete"]);
        assert_eq!(port.services.len(), 1);
    }

    #[test]
    fn install_create_failure_leaves_no_service() {
        let mut port = FakeScm::with(Some(ScmState::Running));
        port.fail_once.insert("create");
        let err = install_with(&mut port, &mut FakeClock::default(), &spec()).unwrap_err();
        assert_eq!(service_msg(err), "create boom");
        assert_eq!(calls(&port), ["stop", "delete", "create"]);
        assert!(port.services.is_empty());
    }

    #[test]
    fn replace_times_out_when_old_service_never_stops() {
        let mut port = FakeScm::with(Some(ScmState::Running));
        port.stop_script = vec![ScmState::StopPending];
        let mut clock = FakeClock::default();
        let err = install_with(&mut port, &mut clock, &spec()).unwrap_err();
        let msg = service_msg(err);
        assert!(
            msg.contains("Stopped") && msg.contains("StopPending"),
            "{msg}"
        );
        assert!(clock.now >= Duration::from_secs(20) && clock.now < Duration::from_secs(21));
        assert_eq!(calls(&port), ["stop"]);
    }

    #[test]
    fn stop_waits_for_an_already_stopping_service() {
        let mut port = FakeScm::with(None);
        port.add(
            SERVE,
            ScmState::StopPending,
            &[ScmState::StopPending, ScmState::Stopped],
        );
        stop_with(&mut port, &mut FakeClock::default(), "serve").unwrap();
        assert!(calls(&port).is_empty());
    }

    #[test]
    fn stop_times_out_after_20_seconds() {
        let mut port = FakeScm::with(Some(ScmState::Running));
        port.stop_script = vec![ScmState::StopPending];
        let mut clock = FakeClock::default();
        let err = stop_with(&mut port, &mut clock, "serve").unwrap_err();
        assert!(service_msg(err).contains("within 20s"));
        assert!(clock.now >= Duration::from_secs(20) && clock.now < Duration::from_secs(21));
    }

    #[test]
    fn delete_times_out_after_10_seconds() {
        let mut port = FakeScm::with(Some(ScmState::Stopped));
        port.never_gone = true;
        let mut clock = FakeClock::default();
        let err = uninstall_with(&mut port, &mut clock, "serve").unwrap_err();
        let msg = service_msg(err);
        assert!(msg.contains("gone") && msg.contains("within 10s"), "{msg}");
        assert!(clock.now >= Duration::from_secs(10) && clock.now < Duration::from_secs(11));
    }

    #[test]
    fn delete_of_running_service_only_marks_it() {
        let mut port = FakeScm::with(Some(ScmState::Running));
        port.delete(SERVE).unwrap();
        assert_eq!(port.state(SERVE).unwrap(), Some(ScmState::Running));
        port.stop(SERVE).unwrap();
        assert_eq!(port.state(SERVE).unwrap(), None);
    }

    #[test]
    fn stop_of_stopped_is_ok() {
        let mut port = FakeScm::with(Some(ScmState::Stopped));
        stop_with(&mut port, &mut FakeClock::default(), "serve").unwrap();
        assert!(calls(&port).is_empty());
    }

    #[test]
    fn stop_stops_a_running_service() {
        let mut port = FakeScm::with(Some(ScmState::Running));
        stop_with(&mut port, &mut FakeClock::default(), "serve").unwrap();
        assert_eq!(calls(&port), ["stop"]);
    }

    #[test]
    fn stop_absent_is_install_hint() {
        let mut port = FakeScm::with(None);
        let err = stop_with(&mut port, &mut FakeClock::default(), "serve").unwrap_err();
        assert!(service_msg(err).contains("service install --system"));
    }

    #[test]
    fn operations_honour_the_service_name() {
        // Only the serve service exists; the access role must not see it.
        let mut port = FakeScm::with(Some(ScmState::Running));
        let err = stop_with(&mut port, &mut FakeClock::default(), "access").unwrap_err();
        assert!(service_msg(err).contains("iroh-tunnel-access"));
        assert!(port.stop("iroh-tunnel-access").is_err());
        assert_eq!(port.current(), Some(ScmState::Running));
    }

    #[test]
    fn uninstall_absent_is_ok() {
        let mut port = FakeScm::with(None);
        uninstall_with(&mut port, &mut FakeClock::default(), "serve").unwrap();
        assert!(calls(&port).is_empty());
    }

    #[test]
    fn uninstall_removes_a_running_service() {
        let mut port = FakeScm::with(Some(ScmState::Running));
        uninstall_with(&mut port, &mut FakeClock::default(), "serve").unwrap();
        assert_eq!(calls(&port), ["stop", "delete"]);
        assert!(port.services.is_empty());
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
        port.start_script = vec![ScmState::StartPending, ScmState::Running];
        start_with(&mut port, &mut FakeClock::default(), "serve").unwrap();
        assert_eq!(calls(&port), ["start"]);
        assert_eq!(port.current(), Some(ScmState::Running));
    }

    #[test]
    fn status_reports_absent_and_state() {
        let mut port = FakeScm::with(None);
        assert_eq!(status_with(&mut port, "serve").unwrap(), ScmStatus::Absent);
        port.add(SERVE, ScmState::Running, &[]);
        assert_eq!(
            status_with(&mut port, "serve").unwrap(),
            ScmStatus::Installed(ScmState::Running)
        );
    }

    #[test]
    fn restart_waits_for_lock_then_starts() {
        let mut port = FakeScm::with(Some(ScmState::Running));
        let log = Rc::clone(&port.calls);
        let mut seen_at_wait = None;
        restart_with(&mut port, &mut FakeClock::default(), "serve", |t| {
            assert_eq!(t, Duration::from_secs(15));
            seen_at_wait = Some(log.borrow().clone());
            Ok(())
        })
        .unwrap();
        assert_eq!(seen_at_wait.unwrap(), ["stop"]);
        assert_eq!(calls(&port), ["stop", "start"]);
    }

    #[test]
    fn restart_times_out_without_starting() {
        let mut port = FakeScm::with(Some(ScmState::Running));
        let err = restart_with(&mut port, &mut FakeClock::default(), "serve", |_| {
            Err(CliError::Service("still running".into()).into())
        })
        .unwrap_err();
        assert_eq!(service_msg(err), "still running");
        assert_eq!(calls(&port), ["stop"]);
    }

    #[test]
    fn first_arg_of_a_quoted_command_line_is_the_program() {
        let line = r#""C:\Program Files\x\iroh-tunnel.exe" serve run --config "C:\a b\c.toml""#;
        assert_eq!(
            first_command_line_arg(line).as_deref(),
            Some(r"C:\Program Files\x\iroh-tunnel.exe")
        );
    }

    #[test]
    fn first_arg_of_an_unquoted_command_line_stops_at_whitespace() {
        assert_eq!(
            first_command_line_arg(r"C:\Windows\System32\cmd.exe /c exit").as_deref(),
            Some(r"C:\Windows\System32\cmd.exe")
        );
    }

    #[test]
    fn first_arg_edge_cases() {
        assert_eq!(first_command_line_arg(""), None);
        assert_eq!(first_command_line_arg("   "), None);
        assert_eq!(
            first_command_line_arg(r#""C:\a b\x.exe --flag"#).as_deref(),
            Some(r"C:\a b\x.exe --flag")
        );
    }
}
