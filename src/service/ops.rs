//! Public service operations: policy first, then the platform backend.
//!
//! Every operation decides what it is allowed to do before it touches the
//! host: platform support, privilege (with the rerun guidance the CLI always
//! printed), the user whose files a system service trusts, and the installable
//! config. A decision only a person can make comes back as an [`InstallNeed`]
//! instead of a prompt, so a caller without a terminal can answer it its own
//! way and call again.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::native::{Native, NativeOutput, Runner, ServiceCall, ServiceNote};
use super::pin::{Pin, RetargetConsent};
use super::scope::{self, Environment, ScopeDecision};
use super::ServiceScope;
use crate::error::CliError;
use crate::status::Role;

#[allow(unused_imports)] // unused on targets without a service backend
use super::native::system_runner;

/// A change an operation made to the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppliedEffect {
    RegistrationWritten(PathBuf),
    Enabled,
    Started,
    Stopped,
    Disabled,
    RegistrationRemoved(PathBuf),
}

/// What a successful operation did and what the native tools printed.
#[derive(Debug)]
pub struct ServiceReport {
    pub applied: Vec<AppliedEffect>,
    pub native: Vec<NativeOutput>,
}

/// A failed operation, with whatever it had already changed.
#[derive(Debug)]
pub struct ServiceFailure {
    pub applied: Vec<AppliedEffect>,
    pub native: Vec<NativeOutput>,
    pub error: anyhow::Error,
}

impl ServiceFailure {
    /// A failure before anything reached the host.
    pub(super) fn before_effects(error: anyhow::Error) -> Self {
        Self {
            applied: Vec::new(),
            native: Vec::new(),
            error,
        }
    }
}

impl From<ServiceFailure> for anyhow::Error {
    /// The underlying error, unchanged, so `CliError` downcasts still work.
    fn from(failure: ServiceFailure) -> Self {
        failure.error
    }
}

/// A fact about the service that may be unobtainable without failing the
/// whole report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fact<T> {
    Known(T),
    /// Why the fact could not be determined.
    Unavailable(String),
}

impl<T> Fact<T> {
    fn from_result(result: Result<T>) -> Self {
        match result {
            Ok(value) => Fact::Known(value),
            Err(error) => Fact::Unavailable(error.to_string()),
        }
    }
}

/// Whether a unit, plist, init script or task is registered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Registration {
    Present { location: String },
    Absent,
}

/// What is known about a role's service, each fact from its own source:
/// the registration from the host's files, liveness and owner from the role
/// lock, and the native tool's output carried verbatim (never parsed).
#[derive(Debug)]
pub struct ServiceStatusReport {
    pub registration: Fact<Registration>,
    pub liveness: Fact<crate::ownership::Liveness>,
    pub owner: Fact<crate::ownership::OwnerRecord>,
    pub native: Option<NativeOutput>,
}

/// Everything `install` needs from its caller.
#[derive(Debug, Clone)]
pub struct InstallRequest {
    pub role: Role,
    /// `None` lets [`scope::decide_install_scope`] decide.
    pub scope: Option<ServiceScope>,
    pub config: Option<PathBuf>,
    /// The `iroh-tunnel` executable the registration runs. Must be absolute.
    pub tunnel_executable: PathBuf,
    /// `SUDO_USER`: an input to choosing whose files a system service uses.
    pub sudo_user: Option<String>,
    /// The user a person named when asked ([`InstallNeed::InvokingUser`]).
    pub invoking_user: Option<String>,
    /// `IROH_TUNNEL_STATE_DIR`, if set.
    pub state_dir_override: Option<PathBuf>,
    /// Consent to replace another owner's system registration.
    pub retarget: Option<RetargetConsent>,
}

/// What `install` did, or the decision it needs first.
#[derive(Debug)]
pub enum InstallOutcome {
    Installed(ServiceReport),
    Needs(InstallNeed),
}

/// A decision a person must make before `install` can continue. Nothing has
/// been changed when one is returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallNeed {
    /// User or system service.
    ScopeChoice,
    /// Whose files a system service should use.
    InvokingUser,
    /// Replace an existing system registration that belongs to someone else
    /// (`previous: None` when its owner cannot be told).
    Retarget {
        previous: Option<Pin>,
        proposed: Pin,
    },
}

#[derive(Debug, Clone, Copy)]
pub(super) enum Verb {
    Uninstall,
    Start,
    Stop,
    Restart,
}

impl Verb {
    fn word(self) -> &'static str {
        match self {
            Verb::Uninstall => "uninstall",
            Verb::Start => "start",
            Verb::Stop => "stop",
            Verb::Restart => "restart",
        }
    }
}

fn flag(scope: ServiceScope) -> &'static str {
    match scope {
        ServiceScope::System => "--system",
        ServiceScope::User => "--user",
    }
}

/// `service install`: decide, check, then write the registration, enable and
/// start it.
///
/// # Errors
///
/// A [`ServiceFailure`] whose `error` is a [`CliError`] for policy refusals
/// (unsupported scope, missing privilege, unknown user, unusable config) and
/// whose `applied` lists what changed before a backend step failed.
pub fn install(
    req: &InstallRequest,
    env: Environment,
    call: &mut ServiceCall,
) -> Result<InstallOutcome, ServiceFailure> {
    let mut runner = system_runner;
    install_via(req, env, call, &mut runner)
}

/// `service status`: the registration, liveness and owner facts, plus the
/// native tool's output (inherited by the terminal in
/// [`OutputMode::Inherit`](super::OutputMode::Inherit), returned otherwise).
///
/// # Errors
///
/// A [`ServiceFailure`] for policy refusals and when the native tool cannot
/// run. A fact that cannot be read is [`Fact::Unavailable`], not an error.
pub fn status(
    role: Role,
    scope: ServiceScope,
    env: Environment,
    call: &mut ServiceCall,
) -> Result<ServiceStatusReport, ServiceFailure> {
    let mut runner = system_runner;
    status_via(role, scope, env, call, &mut runner)
}

/// `service uninstall`: stop, disable, and remove the registration.
///
/// # Errors
///
/// A [`ServiceFailure`]; see [`install`].
pub fn uninstall(
    role: Role,
    scope: ServiceScope,
    env: Environment,
    call: &mut ServiceCall,
) -> Result<ServiceReport, ServiceFailure> {
    let mut runner = system_runner;
    verb_via(Verb::Uninstall, role, scope, env, call, &mut runner)
}

/// `service start`; refused while the role already runs.
///
/// # Errors
///
/// A [`ServiceFailure`]; [`CliError::Conflict`] when the role runs.
pub fn start(
    role: Role,
    scope: ServiceScope,
    env: Environment,
    call: &mut ServiceCall,
) -> Result<ServiceReport, ServiceFailure> {
    let mut runner = system_runner;
    verb_via(Verb::Start, role, scope, env, call, &mut runner)
}

/// `service stop`.
///
/// # Errors
///
/// A [`ServiceFailure`]; see [`install`].
pub fn stop(
    role: Role,
    scope: ServiceScope,
    env: Environment,
    call: &mut ServiceCall,
) -> Result<ServiceReport, ServiceFailure> {
    let mut runner = system_runner;
    verb_via(Verb::Stop, role, scope, env, call, &mut runner)
}

/// `service restart`.
///
/// # Errors
///
/// A [`ServiceFailure`]; see [`install`].
pub fn restart(
    role: Role,
    scope: ServiceScope,
    env: Environment,
    call: &mut ServiceCall,
) -> Result<ServiceReport, ServiceFailure> {
    let mut runner = system_runner;
    verb_via(Verb::Restart, role, scope, env, call, &mut runner)
}

/// [`status`] over an injected command runner.
pub(super) fn status_via(
    role: Role,
    scope: ServiceScope,
    env: Environment,
    call: &mut ServiceCall,
    runner: &mut Runner<'_>,
) -> Result<ServiceStatusReport, ServiceFailure> {
    check_verb_policy("status", role, scope, env).map_err(ServiceFailure::before_effects)?;
    let liveness = crate::ownership::probe(role);
    let owner = match &liveness {
        Ok(crate::ownership::Liveness::Running {
            owner: Some(owner), ..
        }) => Fact::Known(owner.clone()),
        Ok(crate::ownership::Liveness::Running { owner: None, .. }) => {
            Fact::Unavailable("the owner record is missing or unreadable".to_owned())
        }
        Ok(crate::ownership::Liveness::NotRunning) => {
            Fact::Unavailable(format!("{} is not running", role.name()))
        }
        Err(error) => Fact::Unavailable(error.to_string()),
    };
    let mut native = Native::new(call.output, runner);
    let output = backend_status(&mut native, &mut *call.notes, role.name(), scope);
    let (applied, outputs) = native.finish();
    let output = output.map_err(|error| ServiceFailure {
        applied,
        native: outputs,
        error,
    })?;
    // On Windows the task backend refuses an unregistered task, so reaching
    // here means it exists.
    #[cfg(windows)]
    let registration = Fact::Known(Registration::Present {
        location: "scheduled task".to_owned(),
    });
    #[cfg(not(windows))]
    let registration = Fact::from_result(registration_of(role.name(), scope));
    Ok(ServiceStatusReport {
        registration,
        liveness: Fact::from_result(liveness),
        owner,
        native: Some(output),
    })
}

/// Whether the platform's registration file exists (Unix backends).
#[cfg(not(windows))]
#[allow(unused_variables)] // unused on targets without a service backend
fn registration_of(role: &str, scope: ServiceScope) -> Result<Registration> {
    #[cfg(target_os = "linux")]
    let path = if super::linux_is_systemd() {
        super::systemd::registration_path(role, scope)?
    } else {
        super::busybox::registration_path(role)
    };
    #[cfg(target_os = "macos")]
    let path = super::launchd::registration_path(role, scope)?;
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let path = PathBuf::new();
    Ok(if path.exists() {
        Registration::Present {
            location: path.display().to_string(),
        }
    } else {
        Registration::Absent
    })
}

/// Run a backend step with a [`Native`] over `runner`, keeping the effects and
/// command outputs it recorded whether it succeeded or not.
fn execute(
    call: &mut ServiceCall,
    runner: &mut Runner<'_>,
    step: impl FnOnce(&mut Native, &mut dyn FnMut(&ServiceNote)) -> Result<()>,
) -> Result<ServiceReport, ServiceFailure> {
    let mut native = Native::new(call.output, runner);
    let result = step(&mut native, &mut *call.notes);
    let (applied, outputs) = native.finish();
    match result {
        Ok(()) => Ok(ServiceReport {
            applied,
            native: outputs,
        }),
        Err(error) => Err(ServiceFailure {
            applied,
            native: outputs,
            error,
        }),
    }
}

/// [`install`] over an injected command runner.
pub(super) fn install_via(
    req: &InstallRequest,
    env: Environment,
    call: &mut ServiceCall,
    runner: &mut Runner<'_>,
) -> Result<InstallOutcome, ServiceFailure> {
    let plan = match plan_install(req, env).map_err(ServiceFailure::before_effects)? {
        Planned::Needs(need) => return Ok(InstallOutcome::Needs(need)),
        Planned::Ready(plan) => plan,
    };
    let role = req.role.name();
    let report = execute(call, runner, |native, notes| {
        backend_install(
            native,
            notes,
            role,
            plan.scope,
            &req.tunnel_executable,
            &plan.config,
            &plan.state,
            plan.pin.as_ref(),
        )
    })?;
    Ok(InstallOutcome::Installed(report))
}

/// [`uninstall`]/[`start`]/[`stop`]/[`restart`] over an injected runner.
pub(super) fn verb_via(
    verb: Verb,
    role: Role,
    scope: ServiceScope,
    env: Environment,
    call: &mut ServiceCall,
    runner: &mut Runner<'_>,
) -> Result<ServiceReport, ServiceFailure> {
    check_verb_policy(verb.word(), role, scope, env)
        .and_then(|()| {
            if matches!(verb, Verb::Start) {
                // Refuse before the manager: the unit would otherwise start and
                // exit after systemd/launchd already reported success (RO-03).
                crate::ownership::ensure_not_running(role)?;
            }
            Ok(())
        })
        .map_err(ServiceFailure::before_effects)?;
    execute(call, runner, |native, notes| {
        backend_verb(verb, native, notes, role.name(), scope)
    })
}

/// Platform support and privilege for a verb on an existing service, with the
/// rerun guidance the CLI always printed.
pub(super) fn check_verb_policy(
    verb: &str,
    role: Role,
    scope: ServiceScope,
    env: Environment,
) -> Result<()> {
    scope::ensure_supported(scope)?;
    let rerun = ["iroh-tunnel", role.name(), "service", verb, flag(scope)].map(str::to_owned);
    scope::ensure_privilege(scope, env.elevated, &rerun)
}

/// What `plan_install` decided.
enum Planned {
    Needs(InstallNeed),
    Ready(InstallPlan),
}

/// The checked inputs of one registration write.
struct InstallPlan {
    scope: ServiceScope,
    config: PathBuf,
    state: PathBuf,
    /// Recorded in a system registration; `None` for a user service.
    pin: Option<Pin>,
}

/// Every check that precedes the write, in the order the CLI always ran them:
/// scope, platform support, then the scope's own privilege and trust checks.
fn plan_install(req: &InstallRequest, env: Environment) -> Result<Planned> {
    if !req.tunnel_executable.is_absolute() {
        return Err(CliError::Service(format!(
            "the tunnel executable {} is not an absolute path; the service must not depend on \
             its working directory",
            req.tunnel_executable.display()
        ))
        .into());
    }
    let role = req.role;
    // Windows has only the user scope, so there is nothing to ask.
    let requested = req.scope.or(cfg!(windows).then_some(ServiceScope::User));
    let scope = match scope::decide_install_scope(requested, env) {
        ScopeDecision::Chosen(scope) => scope,
        ScopeDecision::NeedsChoice => return Ok(Planned::Needs(InstallNeed::ScopeChoice)),
    };
    scope::ensure_supported(scope)?;
    if scope == ServiceScope::System {
        return plan_system_install(req, env);
    }
    // Persist absolute paths: the installed service runs with a different CWD
    // and home, so nothing may stay relative (SM-02).
    let config = absolutize_config_path(&crate::config_cmd::resolve_config_path(
        role,
        req.config.as_deref(),
    )?)?;
    let state = pinned_state_dir(req.state_dir_override.as_deref())?;
    let rerun = install_rerun(role, scope, &config);
    scope::ensure_privilege(scope, env.elevated, &rerun)?;
    check_installable_config(role, &config)?;
    Ok(Planned::Ready(InstallPlan {
        scope,
        config,
        state,
        pin: None,
    }))
}

/// The argv (after `sudo`) that repeats an install with absolute paths.
fn install_rerun(role: Role, scope: ServiceScope, config: &Path) -> Vec<String> {
    [
        "iroh-tunnel",
        role.name(),
        "service",
        "install",
        flag(scope),
        "--config",
    ]
    .into_iter()
    .map(str::to_owned)
    .chain([config.to_string_lossy().into_owned()])
    .collect()
}

/// Install a system service for the invoking user's files (SM-01, SM-02,
/// SM-04). Unprivileged callers are refused with rerun guidance; an elevated
/// caller must name a real user, never root's own home.
#[cfg(unix)]
fn plan_system_install(req: &InstallRequest, env: Environment) -> Result<Planned> {
    use super::{system, trust};

    let role = req.role;
    if !env.elevated {
        // Same guidance as before: absolute path, no elevation, no fallback.
        let config = absolutize_config_path(&crate::config_cmd::resolve_config_path(
            role,
            req.config.as_deref(),
        )?)?;
        let rerun = install_rerun(role, ServiceScope::System, &config);
        scope::ensure_privilege(ServiceScope::System, false, &rerun)?;
    }

    let config_abs = req
        .config
        .as_deref()
        .map(absolutize_config_path)
        .transpose()?;
    let target = match system::resolve_target_account(
        &system::Request {
            role: role.name(),
            config: config_abs.as_deref(),
            state: req.state_dir_override.as_deref(),
            sudo_user: req.sudo_user.as_deref(),
        },
        req.invoking_user.as_deref(),
    )? {
        Some(target) => target,
        None if env.can_answer => return Ok(Planned::Needs(InstallNeed::InvokingUser)),
        None => return Err(system::unknown_user("no SUDO_USER", role.name())),
    };
    // Fail closed before registering anything root would later refuse to run.
    trust::verify_dir(&target.state, target.account.uid)?;
    let text = trust::read_verified(&target.config, target.account.uid)?;
    check_installable_text(role, &text, &target.config)?;
    // Read the registration last, right before the write, so a consent given
    // for an earlier look at it is checked against the current one.
    let existing = super::existing_registration(role.name(), ServiceScope::System);
    if let system::RetargetDecision::Needs { previous, proposed } =
        system::retarget_decision(existing.as_deref(), &target, req.retarget.as_ref())
    {
        return Ok(Planned::Needs(InstallNeed::Retarget { previous, proposed }));
    }
    Ok(Planned::Ready(InstallPlan {
        scope: ServiceScope::System,
        pin: Some(target.pin()),
        config: target.config,
        state: target.state,
    }))
}

#[cfg(not(unix))]
fn plan_system_install(_req: &InstallRequest, _env: Environment) -> Result<Planned> {
    scope::ensure_supported(ServiceScope::System)?;
    Err(CliError::Service("system services are not supported on this platform".into()).into())
}

/// Make `path` absolute for embedding in a system service file.
///
/// Installed services run with a different working directory, so a relative
/// `--config` path must be resolved against the current CWD before it is
/// written into the unit/plist. Falls back to joining the CWD if the path
/// can't be canonicalized (e.g. it doesn't exist yet).
fn absolutize_config_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    if let Ok(abs) = std::fs::canonicalize(path) {
        return Ok(abs);
    }
    let cwd = std::env::current_dir().context("failed to determine current directory")?;
    Ok(cwd.join(path))
}

/// The state directory the installed service will write to, as an absolute
/// path: the override when set, otherwise the platform default.
fn pinned_state_dir(state_dir_override: Option<&Path>) -> Result<PathBuf> {
    let dir = match state_dir_override.filter(|d| !d.as_os_str().is_empty()) {
        Some(dir) => dir.to_path_buf(),
        None => crate::paths::default_state_dir()?,
    };
    std::path::absolute(&dir)
        .with_context(|| format!("cannot make state directory {} absolute", dir.display()))
}

/// Refuse to register a service that could only fail at start: the pinned
/// config must exist and be valid for the role.
fn check_installable_config(role: Role, config: &Path) -> Result<()> {
    let loaded = match role {
        Role::Serve => crate::config::ServeConfig::load(config).map(drop),
        Role::Access => crate::config::AccessConfig::load(config).map(drop),
    };
    installable(role, config, loaded)
}

/// [`check_installable_config`] for text already read from a verified handle.
#[cfg(unix)]
fn check_installable_text(role: Role, text: &str, config: &Path) -> Result<()> {
    use crate::config::RoleDoc;
    let loaded = match role {
        Role::Serve => crate::config::ServeConfig::from_toml(text, config).map(drop),
        Role::Access => crate::config::AccessConfig::from_toml(text, config).map(drop),
    };
    installable(role, config, loaded)
}

fn installable(role: Role, config: &Path, loaded: Result<()>) -> Result<()> {
    loaded.map_err(|e| {
        CliError::Config(format!(
            "cannot install the service: {e:#}. Create or fix the config first \
             (`iroh-tunnel {role} config keygen --config {path}`), or pass --config",
            role = role.name(),
            path = config.display()
        ))
        .into()
    })
}

// ---------------------------------------------------------------------------
// Platform dispatch. On Linux the choice between systemd and the BusyBox/SysV
// backend is made at runtime (see `linux_is_systemd`); on macOS it is always
// launchd; on Windows the user background task.
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
#[allow(unused_variables)] // unused on targets without a service backend
fn backend_install(
    native: &mut Native,
    notes: &mut dyn FnMut(&ServiceNote),
    role: &str,
    scope: ServiceScope,
    binary: &Path,
    config: &Path,
    state: &Path,
    pin: Option<&Pin>,
) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        if super::linux_is_systemd() {
            super::systemd::install(native, notes, role, scope, binary, config, state, pin)
        } else {
            super::busybox::install(native, notes, role, scope, binary, config, state, pin)
        }
    }
    #[cfg(target_os = "macos")]
    {
        super::launchd::install(native, notes, role, scope, binary, config, state, pin)
    }
    #[cfg(windows)]
    {
        super::windows::install(native, notes, role, scope, binary, config, state)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        super::unsupported()
    }
}

#[allow(unused_variables)] // unused on targets without a service backend
fn backend_status(
    native: &mut Native,
    notes: &mut dyn FnMut(&ServiceNote),
    role: &str,
    scope: ServiceScope,
) -> Result<NativeOutput> {
    #[cfg(target_os = "linux")]
    {
        if super::linux_is_systemd() {
            super::systemd::status(native, notes, role, scope)
        } else {
            super::busybox::status(native, notes, role, scope)
        }
    }
    #[cfg(target_os = "macos")]
    {
        super::launchd::status(native, notes, role, scope)
    }
    #[cfg(windows)]
    {
        super::windows::status(native, notes, role, scope)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        super::unsupported()
    }
}

#[allow(unused_variables)] // unused on targets without a service backend
fn backend_verb(
    verb: Verb,
    native: &mut Native,
    notes: &mut dyn FnMut(&ServiceNote),
    role: &str,
    scope: ServiceScope,
) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        use super::{busybox, systemd};
        if super::linux_is_systemd() {
            match verb {
                Verb::Uninstall => systemd::uninstall(native, notes, role, scope),
                Verb::Start => systemd::start(native, role, scope),
                Verb::Stop => systemd::stop(native, notes, role, scope),
                Verb::Restart => systemd::restart(native, notes, role, scope),
            }
        } else {
            match verb {
                Verb::Uninstall => busybox::uninstall(native, notes, role, scope),
                Verb::Start => busybox::start(native, role, scope),
                Verb::Stop => busybox::stop(native, notes, role, scope),
                Verb::Restart => busybox::restart(native, notes, role, scope),
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        use super::launchd;
        match verb {
            Verb::Uninstall => launchd::uninstall(native, notes, role, scope),
            Verb::Start => launchd::start(native, role, scope),
            Verb::Stop => launchd::stop(native, notes, role, scope),
            Verb::Restart => launchd::restart(native, notes, role, scope),
        }
    }
    #[cfg(windows)]
    {
        use super::windows;
        match verb {
            Verb::Uninstall => windows::uninstall(native, notes, role, scope),
            Verb::Start => windows::start(native, role, scope),
            Verb::Stop => windows::stop(native, notes, role, scope),
            Verb::Restart => windows::restart(native, notes, role, scope),
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        super::unsupported()
    }
}
