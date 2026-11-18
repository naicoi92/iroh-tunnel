//! `service install`: decide, check, then write the registration, enable and
//! start it.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::backend::Backend;
use super::decide_install_scope::decide_install_scope;
use super::execute::execute;
use super::native::system_runner;
#[cfg(windows)]
use super::native::OutputMode;
use super::native::{Native, Runner, ServiceCall, ServiceNote};
use super::pin::Pin;
use super::platform;
use super::policy::{ensure_privilege, ensure_root_run_privilege, ensure_supported};
use super::registration::RegistrationInput;
use super::service_binary::ServiceBinary;
use super::types::{
    AppliedEffect, Environment, InstallNeed, InstallOutcome, InstallRequest, RestartPolicy,
    ScopeDecision, ServiceFailure, ServiceScope,
};
use crate::error::CliError;
use crate::ownership::{self, Liveness, LockDirs, LockResolver, Tier};
use crate::role::Role;

/// The argv (after `sudo`) that repeats an install with absolute paths.
pub(crate) fn install_rerun(
    req: &InstallRequest,
    scope: ServiceScope,
    config: &Path,
) -> Vec<String> {
    let mut argv = req.invocation.rerun_prefix(req.role.name());
    argv.extend([
        "service".to_owned(),
        "install".to_owned(),
        super::verb::flag(scope).to_owned(),
        "--config".to_owned(),
        config.to_string_lossy().into_owned(),
    ]);
    argv
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
    install_via(req, env, call, &mut runner, &LockDirs::resolve)
}

/// [`install`] over an injected command runner.
pub(crate) fn install_via(
    req: &InstallRequest,
    env: Environment,
    call: &mut ServiceCall,
    runner: &mut Runner<'_>,
    lock_dirs: LockResolver<'_>,
) -> Result<InstallOutcome, ServiceFailure> {
    let planned = {
        let mut probe = other_scope_probe(&mut *runner);
        plan_install(req, env, &mut probe, lock_dirs)
    };
    let plan = match planned.map_err(ServiceFailure::before_effects)? {
        Planned::Needs(need) => return Ok(InstallOutcome::Needs(need)),
        Planned::Ready(plan) => plan,
    };
    let role = req.role.name();
    let root_run = platform::root_run(plan.scope, platform::system_wide_only());
    let report = execute(call, runner, |native, notes| {
        install_steps(
            native,
            notes,
            role,
            &plan,
            materialize_root_only,
            |native, notes, binary| {
                Backend::select(plan.scope)?.install(
                    native,
                    notes,
                    &RegistrationInput {
                        role,
                        invocation: &req.invocation,
                        scope: plan.scope,
                        binary,
                        config: &plan.config,
                        state: &plan.state,
                        pin: plan.pin.as_ref(),
                        policy: &RestartPolicy::DEFAULT,
                    },
                )
            },
            |native| super::service_binary::release_unreferenced_copy(native, role, root_run),
        )
    })?;
    Ok(InstallOutcome::Installed(report))
}

/// The binary the registration of `scope` runs.
///
/// A root-run registration on Linux or macOS runs the canonical resolved
/// binary when `verify` (the root-only binary check) accepts it, otherwise a
/// managed copy of the running executable. Every other registration runs the
/// resolved binary as given. Writes nothing.
///
/// # Errors
///
/// [`CliError::Config`] when the resolved binary of a root-run registration
/// cannot be canonicalized.
fn plan_binary(
    req: &InstallRequest,
    scope: ServiceScope,
    system_wide_only: bool,
    verify: &dyn Fn(&Path) -> Result<()>,
) -> Result<ServiceBinary> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if platform::root_run(scope, system_wide_only) {
        return super::service_binary::choose(
            &req.tunnel_executable,
            &req.running_executable,
            super::service_binary::managed_copy_path(req.role.name()),
            verify,
        );
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let _ = (scope, system_wide_only, verify);
    Ok(ServiceBinary::Direct(req.tunnel_executable.clone()))
}

/// Write the binary, then the registration that runs it.
///
/// The managed copy (if any) is written and recorded first, announced with a
/// [`ServiceNote::ManagedBinary`] naming the plan's absolute config, and then
/// `register` runs with the path of the plan's binary. After a
/// [`ServiceBinary::Direct`] registration succeeds,
/// `release_after_direct` removes a managed copy nothing references any more.
///
/// # Errors
///
/// The first failing step's error; effects already recorded stay in
/// `native`'s applied list.
fn install_steps(
    native: &mut Native,
    notes: &mut dyn FnMut(&ServiceNote),
    role: &str,
    plan: &InstallPlan,
    materialize: impl FnOnce(&ServiceBinary) -> Result<Option<AppliedEffect>>,
    register: impl FnOnce(&mut Native, &mut dyn FnMut(&ServiceNote), &Path) -> Result<()>,
    release_after_direct: impl FnOnce(&mut Native) -> Result<Option<AppliedEffect>>,
) -> Result<()> {
    let binary = &plan.binary;
    if let Some(effect) = materialize(binary)? {
        native.applied(effect);
    }
    if let ServiceBinary::Managed { source, copy } = binary {
        notes(&ServiceNote::ManagedBinary {
            role: role.to_owned(),
            source: source.display.clone(),
            copy: copy.clone(),
            config: plan.config.clone(),
        });
    }
    register(native, notes, binary.path())?;
    if matches!(binary, ServiceBinary::Direct(_)) {
        if let Some(effect) = release_after_direct(native)? {
            native.applied(effect);
        }
    }
    Ok(())
}

/// Publish the managed copy of `binary`, trusting root only.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn materialize_root_only(binary: &ServiceBinary) -> Result<Option<AppliedEffect>> {
    super::service_binary::materialize(binary, super::trust::TrustedOwners::RootOnly)
}

/// No managed copy exists off Linux and macOS.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn materialize_root_only(_binary: &ServiceBinary) -> Result<Option<AppliedEffect>> {
    Ok(None)
}

/// The root-only binary check of a root-run registration.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn verify_root_only(path: &Path) -> Result<()> {
    super::trust::verify_binary(path, super::trust::TrustedOwners::RootOnly)
}

/// Never consulted off Linux and macOS: [`plan_binary`] registers the
/// resolved binary there.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn verify_root_only(_path: &Path) -> Result<()> {
    Ok(())
}

/// Asks whether `role` is registered in the scope *other* than the requested
/// one (a user task for `--system`, an SCM service for `--user`). A failed
/// query is an error, never "absent".
type OtherScopeProbe<'a> = dyn FnMut(ServiceScope, Role) -> Result<bool> + 'a;

/// The real probe: `schtasks` listing for `--system`, the SCM for `--user`.
/// Neither needs administrator rights.
#[cfg(windows)]
fn other_scope_probe<'a>(
    runner: &'a mut Runner<'a>,
) -> impl FnMut(ServiceScope, Role) -> Result<bool> + 'a {
    move |requested, role| match requested {
        ServiceScope::System => {
            let mut native = Native::new(OutputMode::Capture, &mut *runner);
            super::backend::windows::task_registered(&mut native, role.name())
        }
        ServiceScope::User => super::backend::windows_scm::service_registered(
            &mut super::backend::windows_scm_impl::Scm,
            role.name(),
        ),
    }
}

/// No other scope is consulted on targets without Windows cross-scope rules.
#[cfg(not(windows))]
fn other_scope_probe<'a>(
    _runner: &'a mut Runner<'a>,
) -> impl FnMut(ServiceScope, Role) -> Result<bool> + 'a {
    |_, _| Ok(false)
}

/// Refuse to install `requested` while the same role is registered in the
/// other scope: two registrations would fight over one node identity and
/// state directory. Pure; `other_registered` comes from the platform probe.
///
/// # Errors
///
/// [`CliError::Service`] naming the uninstall command for the other scope.
#[cfg_attr(
    all(not(test), not(windows)),
    expect(dead_code, reason = "only the Windows backend checks the other scope")
)]
pub(crate) fn cross_scope_conflict(
    requested: ServiceScope,
    other_registered: bool,
    role: Role,
) -> Result<()> {
    if !other_registered {
        return Ok(());
    }
    let role = role.name();
    let (other, other_flag, kind) = match requested {
        ServiceScope::System => ("user", "--user", "a user task"),
        ServiceScope::User => ("system", "--system", "a system service"),
    };
    Err(CliError::Service(format!(
        "iroh-tunnel-{role} is already installed as {kind}; remove it first with \
         `iroh-tunnel {role} service uninstall {other_flag}`, then install the {requested_name} \
         {requested_kind}. Nothing was changed ({other} registration left in place).",
        requested_name = match requested {
            ServiceScope::System => "system",
            ServiceScope::User => "user",
        },
        requested_kind = match requested {
            ServiceScope::System => "service",
            ServiceScope::User => "task",
        },
    ))
    .into())
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
    /// What the registration runs.
    binary: ServiceBinary,
}

/// Every check that precedes the write, in the order the CLI always ran them:
/// scope, platform support, then the scope's own privilege and trust checks.
fn plan_install(
    req: &InstallRequest,
    env: Environment,
    probe: &mut OtherScopeProbe<'_>,
    lock_dirs: LockResolver<'_>,
) -> Result<Planned> {
    if !req.tunnel_executable.is_absolute() {
        return Err(CliError::Service(format!(
            "the tunnel executable {} is not an absolute path; the service must not depend on \
             its working directory",
            req.tunnel_executable.display()
        ))
        .into());
    }
    let scope = match decide_install_scope(req.scope, env) {
        ScopeDecision::Chosen(scope) => scope,
        ScopeDecision::NeedsChoice => return Ok(Planned::Needs(InstallNeed::ScopeChoice)),
    };
    ensure_supported(scope)?;
    if scope == ServiceScope::System {
        return plan_system_install(req, env, probe, lock_dirs);
    }
    plan_user_install(req, env, probe, lock_dirs, platform::system_wide_only())
}

/// The user-scope half of [`plan_install`]. `system_wide_only` hosts (BusyBox)
/// register a root-run service even for `--user`, so an unprivileged caller is
/// refused first, before any path, trust or binary decision.
#[cfg_attr(
    not(windows),
    expect(unused_variables, reason = "the probe is Windows-only")
)]
fn plan_user_install(
    req: &InstallRequest,
    env: Environment,
    probe: &mut OtherScopeProbe<'_>,
    lock_dirs: LockResolver<'_>,
    system_wide_only: bool,
) -> Result<Planned> {
    let (role, scope) = (req.role, ServiceScope::User);
    ensure_root_run_privilege(platform::root_run(scope, system_wide_only), env.elevated)?;
    // Persist absolute paths: the installed service runs with a different CWD
    // and home, so nothing may stay relative (SM-02).
    let config = absolutize_config_path(&crate::config::resolve_config_path(
        role,
        req.config.as_deref(),
    )?)?;
    let state = pinned_state_dir(req.state_dir_override.as_deref())?;
    let rerun = install_rerun(req, scope, &config);
    ensure_privilege(scope, env.elevated, &rerun)?;
    #[cfg(windows)]
    cross_scope_conflict(scope, probe(scope, role)?, role)?;
    let registering = effective_scope_for(scope, system_wide_only);
    ensure_no_cross_scope_conflict(role, registering, None, lock_dirs)?;
    check_installable_config(role, &config)?;
    // Last: a BusyBox registration is root-run even for `--user`.
    let binary = plan_binary(req, scope, system_wide_only, &verify_root_only)?;
    Ok(Planned::Ready(InstallPlan {
        scope,
        config,
        state,
        pin: None,
        binary,
    }))
}

/// Install a system service for the invoking user's files (SM-01, SM-02,
/// SM-04). Unprivileged callers are refused with rerun guidance; an elevated
/// caller must name a real user, never root's own home.
#[cfg(unix)]
fn plan_system_install(
    req: &InstallRequest,
    env: Environment,
    _probe: &mut OtherScopeProbe<'_>,
    lock_dirs: LockResolver<'_>,
) -> Result<Planned> {
    plan_system_install_with(
        req,
        env,
        lock_dirs,
        &|role| platform::existing_registration(role, ServiceScope::System),
        &verify_root_only,
    )
}

/// [`plan_system_install`] over the `existing` registration of a role and
/// the root-only binary check `verify`.
#[cfg(unix)]
fn plan_system_install_with(
    req: &InstallRequest,
    env: Environment,
    lock_dirs: LockResolver<'_>,
    existing: &dyn Fn(&str) -> Option<String>,
    verify: &dyn Fn(&Path) -> Result<()>,
) -> Result<Planned> {
    use super::{system, trust};

    let role = req.role;
    if !env.elevated {
        // Same guidance as before: absolute path, no elevation, no fallback.
        let config = absolutize_config_path(&crate::config::resolve_config_path(
            role,
            req.config.as_deref(),
        )?)?;
        let rerun = install_rerun(req, ServiceScope::System, &config);
        ensure_privilege(ServiceScope::System, false, &rerun)?;
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
    ensure_no_cross_scope_conflict(
        role,
        ServiceScope::System,
        Some(&target.account.home),
        lock_dirs,
    )?;
    // Fail closed before registering anything root would later refuse to run.
    trust::verify_dir(&target.state, target.account.uid)?;
    let text = trust::read_verified(&target.config, target.account.uid)?;
    check_installable_text(role, &text, &target.config)?;
    // Read the registration last, right before the write, so a consent given
    // for an earlier look at it is checked against the current one.
    let existing = existing(role.name());
    if let system::RetargetDecision::Needs { previous, proposed } =
        system::retarget_decision(existing.as_deref(), &target, req.retarget.as_ref())
    {
        return Ok(Planned::Needs(InstallNeed::Retarget { previous, proposed }));
    }
    // Last: only a plan nothing is pending on decides the binary.
    let binary = plan_binary(req, ServiceScope::System, false, verify)?;
    Ok(Planned::Ready(InstallPlan {
        scope: ServiceScope::System,
        pin: Some(target.pin()),
        config: target.config,
        state: target.state,
        binary,
    }))
}

/// Install a Windows service for the installing (token) user's files (SM-04).
/// Unprivileged callers are refused with rerun guidance; every file the
/// service will trust is checked before anything is registered.
#[cfg(windows)]
fn plan_system_install(
    req: &InstallRequest,
    env: Environment,
    probe: &mut OtherScopeProbe<'_>,
    lock_dirs: LockResolver<'_>,
) -> Result<Planned> {
    let paths = WindowsSystemPaths::of(req)?;
    ensure_windows_system_install_allowed(req, env, &paths, probe, lock_dirs)?;
    let target = resolve_windows_target(req.role, &paths)?;
    verify_windows_trust(req, &target)?;
    // Read the registration last, right before the write, so a consent given
    // for an earlier look at it is checked against the current one.
    if let Some(need) = windows_retarget_need(req, &target)? {
        return Ok(Planned::Needs(need));
    }
    Ok(Planned::Ready(InstallPlan {
        scope: ServiceScope::System,
        pin: Some(target.pin()),
        config: target.config,
        state: target.state,
        // Windows trusts the binary through `verify_windows_trust` above.
        binary: plan_binary(req, ServiceScope::System, false, &verify_root_only)?,
    }))
}

/// The explicit paths of a Windows system install, made absolute before
/// anything reads them: the service runs elsewhere, and the trust checks
/// refuse a relative path.
#[cfg(windows)]
struct WindowsSystemPaths {
    /// `--config`, absolute and without a verbatim prefix.
    config: Option<PathBuf>,
    /// `IROH_TUNNEL_STATE_DIR`, absolute.
    state: Option<PathBuf>,
}

#[cfg(windows)]
impl WindowsSystemPaths {
    fn of(req: &InstallRequest) -> Result<Self> {
        let config = req
            .config
            .as_deref()
            .map(windows_absolute_config)
            .transpose()?;
        let state = req
            .state_dir_override
            .as_deref()
            .filter(|d| !d.as_os_str().is_empty())
            .map(|dir| {
                std::path::absolute(dir).with_context(|| {
                    format!("cannot make state directory {} absolute", dir.display())
                })
            })
            .transpose()?;
        Ok(Self { config, state })
    }
}

/// `path` made absolute (a bare filename has no parent to read) and without
/// a verbatim prefix, as a Windows registration pins it.
#[cfg(windows)]
fn windows_absolute_config(path: &Path) -> Result<PathBuf> {
    absolutize_config_path(path).map(strip_verbatim_prefix)
}

/// Refuse a Windows system install without elevation (with rerun guidance)
/// or while the role is registered or running in the user scope.
#[cfg(windows)]
fn ensure_windows_system_install_allowed(
    req: &InstallRequest,
    env: Environment,
    paths: &WindowsSystemPaths,
    probe: &mut OtherScopeProbe<'_>,
    lock_dirs: LockResolver<'_>,
) -> Result<()> {
    let role = req.role;
    if !env.elevated {
        let config = match &paths.config {
            Some(config) => config.clone(),
            None => windows_absolute_config(&crate::config::resolve_config_path(role, None)?)?,
        };
        let rerun = install_rerun(req, ServiceScope::System, &config);
        ensure_privilege(ServiceScope::System, false, &rerun)?;
    }
    // Before any trust check or registration: the user task is the other scope.
    cross_scope_conflict(
        ServiceScope::System,
        probe(ServiceScope::System, role)?,
        role,
    )?;
    // A user process of the role holds the user tier: refused like on Unix.
    ensure_no_cross_scope_conflict(role, ServiceScope::System, None, lock_dirs)
}

/// The account and files a Windows system service of `role` is for: the
/// installing token's user, with default paths from its LocalAppData.
#[cfg(windows)]
fn resolve_windows_target(
    role: Role,
    paths: &WindowsSystemPaths,
) -> Result<super::system_windows::WinTarget> {
    use super::system_windows::{resolve_target, WinRequest};
    use super::trust_windows::{account_of, owner_of, token_user};

    let token = token_user()?;
    // The token user is the elevated process's own user: the shell resolves
    // its LocalAppData with the current token.
    resolve_target(
        &WinRequest {
            role: role.name(),
            config: paths.config.as_deref(),
            state: paths.state.as_deref(),
        },
        &token,
        &owner_of,
        &|_| crate::paths::local_app_data(),
        &account_of,
    )
}

/// Fail closed before registering anything LocalSystem would later refuse:
/// the machine directory, the target's state directory and config, and the
/// binary must all be trusted.
#[cfg(windows)]
fn verify_windows_trust(
    req: &InstallRequest,
    target: &super::system_windows::WinTarget,
) -> Result<()> {
    use super::trust_windows::model::TrustedSet;
    use super::trust_windows::{read_verified, verify_binary, verify_dir, verify_machine_dir};

    if let Some(machine) = crate::ownership::machine_dir()? {
        if machine.verify && machine.path.exists() {
            verify_machine_dir(&machine.path).map_err(|e| machine_dir_refusal(e, &machine.path))?;
        }
    }
    let trusted = TrustedSet::for_user_files(&target.sid);
    verify_dir(&target.state, &trusted)?;
    let text = read_verified(&target.config, &trusted)?;
    verify_config_confidential(&target.config, &super::trust_windows::verify_confidential)?;
    check_installable_text(req.role, &text, &target.config)?;
    verify_binary(&req.tunnel_executable)
}

/// Refuse a config whose directory or file is readable by a non-admin
/// principal: LocalSystem would run it and the key it holds must stay
/// confidential. `verify` is the admin-only check (injected for tests); a
/// directory is checked even before the config exists.
#[cfg(windows)]
fn verify_config_confidential(config: &Path, verify: &dyn Fn(&Path) -> Result<()>) -> Result<()> {
    if let Some(dir) = config.parent().filter(|d| !d.as_os_str().is_empty()) {
        verify(dir)?;
    }
    verify(config)
}

/// The retarget decision needed before replacing the role's current Windows
/// service, if it belongs to someone else.
#[cfg(windows)]
fn windows_retarget_need(
    req: &InstallRequest,
    target: &super::system_windows::WinTarget,
) -> Result<Option<InstallNeed>> {
    use super::backend::windows_scm::{existing_registration_body, service_name};
    use super::backend::windows_scm_impl::Scm;
    use super::pin::{retarget_decision_for, RetargetDecision};

    let existing = existing_registration_body(&mut Scm, &service_name(req.role.name()))?;
    Ok(
        match retarget_decision_for(existing.as_deref(), target.pin(), req.retarget.as_ref()) {
            RetargetDecision::Needs { previous, proposed } => {
                Some(InstallNeed::Retarget { previous, proposed })
            }
            RetargetDecision::Proceed => None,
        },
    )
}

/// Keep a `Permission` refusal from the machine-directory check as it is
/// (its text already says nothing was changed); wrap anything else into one.
#[cfg(windows)]
fn machine_dir_refusal(err: anyhow::Error, dir: &Path) -> anyhow::Error {
    let context = format!(
        "the machine directory {} is not safe to trust",
        dir.display()
    );
    if matches!(
        err.downcast_ref::<CliError>(),
        Some(CliError::Permission(_))
    ) {
        return err.context(context);
    }
    CliError::Permission(format!("{context}: {err:#}. Nothing was changed.")).into()
}

/// Drop a leading `\\?\` (verbatim) prefix that `canonicalize` adds, so the
/// pinned and displayed path matches what the user would type. A verbatim UNC
/// path (`\\?\UNC\…`) is left alone: the trust walk refuses it.
#[cfg_attr(
    all(not(test), not(windows)),
    expect(dead_code, reason = "only Windows paths carry a verbatim prefix")
)]
fn strip_verbatim_prefix(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with(r"UNC\") => PathBuf::from(rest),
        _ => path,
    }
}

#[cfg(not(any(unix, windows)))]
fn plan_system_install(
    _req: &InstallRequest,
    _env: Environment,
    _probe: &mut OtherScopeProbe<'_>,
    _lock_dirs: LockResolver<'_>,
) -> Result<Planned> {
    Err(CliError::Service("system services are not supported on this platform".into()).into())
}

/// The scope an install actually registers in: every registration is
/// system-wide where the host has no per-user services.
fn effective_scope_for(requested: ServiceScope, system_wide_only: bool) -> ServiceScope {
    if system_wide_only {
        ServiceScope::System
    } else {
        requested
    }
}

/// Refuse an install while the role runs at the lock tier of the scope it will
/// *not* register in: the service started there would collide with the new
/// registration's.
///
/// The check compares a tier instead of calling `ensure_not_running`, because
/// re-installing over the same scope's own running service is supported and
/// specified. The tier is the effective scope's (see [`effective_scope_for`]):
/// BusyBox has no per-user services, so a user request registers system-wide.
/// The directories are injected so tests never probe the developer's real
/// machine lock directory.
///
/// Known gaps, none of which this check sees: a foreground run in the same
/// tier, a job loaded outside `service install`, a stopped service registered
/// in the other scope (running roles are checked, not registrations), a
/// shared machine-lock holder in another account, and a BusyBox user-scope
/// request as root, where the non-registering tier is the calling process's
/// own per-user directory rather than an invoking user's.
///
/// # Errors
///
/// [`CliError::Conflict`] when the other tier holds the role, or the error of
/// resolving the lock directories.
fn ensure_no_cross_scope_conflict(
    role: Role,
    registering: ServiceScope,
    account_home: Option<&Path>,
    resolve: LockResolver<'_>,
) -> Result<()> {
    let dirs = resolve(account_home)?;
    ensure_no_cross_scope_conflict_in(role, registering, &dirs)
}

/// [`ensure_no_cross_scope_conflict`] over already resolved directories.
fn ensure_no_cross_scope_conflict_in(
    role: Role,
    registering: ServiceScope,
    dirs: &LockDirs,
) -> Result<()> {
    let (other_tier, other_scope) = match registering {
        ServiceScope::User => (Tier::Machine, ServiceScope::System),
        ServiceScope::System => (Tier::User, ServiceScope::User),
    };
    if let Liveness::Running { tier, owner } = ownership::tier_liveness(role, other_tier, dirs)? {
        let flag = super::verb::flag(other_scope);
        return Err(CliError::Conflict(format!(
            "{role} is already running ({}; {}); {}; to remove that service run \
             `iroh-tunnel {role} service uninstall {flag}`, then install again",
            tier.label(),
            ownership::describe_owner(owner.as_ref()),
            ownership::stop_hint_scoped(role, flag),
            role = role.name(),
        ))
        .into());
    }
    Ok(())
}

/// Make `path` absolute for embedding in a system service file.
///
/// Installed services run with a different working directory, so a relative
/// `--config` path must be resolved against the current CWD before it is
/// written into the unit/plist. Falls back to joining the CWD if the path
/// can't be canonicalized (e.g. it doesn't exist yet).
pub(crate) fn absolutize_config_path(path: &Path) -> Result<PathBuf> {
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
        Role::Serve => {
            crate::config::load_role_config::<crate::config::ServeConfig>(config).map(drop)
        }
        Role::Access => {
            crate::config::load_role_config::<crate::config::AccessConfig>(config).map(drop)
        }
    };
    installable(role, config, loaded)
}

/// [`check_installable_config`] for text already read from a verified handle.
#[cfg(any(unix, windows))]
fn check_installable_text(role: Role, text: &str, config: &Path) -> Result<()> {
    let loaded = match role {
        Role::Serve => {
            crate::config::parse_role_config::<crate::config::ServeConfig>(text, config).map(drop)
        }
        Role::Access => {
            crate::config::parse_role_config::<crate::config::AccessConfig>(text, config).map(drop)
        }
    };
    installable(role, config, loaded)
}

fn installable(role: Role, config: &Path, loaded: Result<()>) -> Result<()> {
    let advice = format!(
        "Create or fix the config first (`iroh-tunnel {role} config keygen --config {path}`), \
         or pass --config",
        role = role.name(),
        path = config.display()
    );
    loaded.map_err(|e| {
        // A config error is re-wrapped by its (already sanitized) message so
        // the category prefix is not repeated. The loaders' config errors are
        // leaf diagnostics without a source, so nothing is lost; other
        // failures (I/O) stay the source, so their `io::Error` remains
        // reachable.
        if let Some(CliError::Config(msg)) = e.downcast_ref::<CliError>() {
            return CliError::Config(format!("cannot install the service: {msg}. {advice}")).into();
        }
        e.context(CliError::Config(format!(
            "cannot install the service ({advice})"
        )))
    })
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::super::fixtures::run_install_in;
    use super::super::fixtures::{commands, request, run_install, BATCH};
    #[cfg(unix)]
    use super::super::fixtures::{in_isolated_home, write_valid_config, CAN_ANSWER};
    #[cfg(unix)]
    use super::super::platform::existing_registration;
    #[cfg(target_os = "linux")]
    use super::super::platform::linux_is_systemd;
    #[cfg(unix)]
    use super::super::types::AppliedEffect;
    use super::super::types::ServiceInvocation;
    use super::*;
    use crate::ownership::{hold_for_test, LockKind};
    use std::path::PathBuf;

    fn dirs(user: &tempfile::TempDir, machine: &tempfile::TempDir) -> LockDirs {
        LockDirs {
            user: user.path().into(),
            machine: Some(machine.path().into()),
        }
    }

    #[test]
    fn effective_scope_follows_the_host_capability() {
        assert_eq!(
            effective_scope_for(ServiceScope::User, false),
            ServiceScope::User
        );
        assert_eq!(
            effective_scope_for(ServiceScope::User, true),
            ServiceScope::System
        );
        assert_eq!(
            effective_scope_for(ServiceScope::System, true),
            ServiceScope::System
        );
    }

    #[test]
    fn a_user_install_is_refused_while_the_machine_tier_holds_the_role() {
        let (user, machine) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let _hold = hold_for_test(Role::Serve, machine.path(), LockKind::Exclusive).unwrap();
        let err = ensure_no_cross_scope_conflict_in(
            Role::Serve,
            ServiceScope::User,
            &dirs(&user, &machine),
        )
        .expect_err("a machine-tier hold must refuse a user install");
        let text = format!("{err:#}");
        assert!(
            matches!(err.downcast_ref::<CliError>(), Some(CliError::Conflict(_))),
            "{err:?}"
        );
        assert!(text.contains("service uninstall --system"), "{text}");
        assert!(text.contains("service stop --system"), "{text}");
        assert!(text.contains("Ctrl+C"), "{text}");
        assert!(text.contains("nothing is stopped automatically"), "{text}");
        assert!(text.contains("machine-wide"), "{text}");
        assert!(!text.contains("start refused"), "{text}");
    }

    #[test]
    fn a_system_install_is_refused_while_the_user_tier_holds_the_role() {
        let (user, machine) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let _hold = hold_for_test(Role::Serve, user.path(), LockKind::Exclusive).unwrap();
        let err = ensure_no_cross_scope_conflict_in(
            Role::Serve,
            ServiceScope::System,
            &dirs(&user, &machine),
        )
        .expect_err("a user-tier hold must refuse a system install");
        let text = format!("{err:#}");
        assert!(
            matches!(err.downcast_ref::<CliError>(), Some(CliError::Conflict(_))),
            "{err:?}"
        );
        assert!(text.contains("service uninstall --user"), "{text}");
        assert!(text.contains("service stop --user"), "{text}");
        assert!(text.contains("Ctrl+C"), "{text}");
        assert!(text.contains("nothing is stopped automatically"), "{text}");
        assert!(text.contains("this user"), "{text}");
        assert!(!text.contains("start refused"), "{text}");
    }

    #[test]
    fn a_hold_at_the_registering_tier_is_permitted() {
        for (scope, held_user) in [(ServiceScope::User, true), (ServiceScope::System, false)] {
            let (user, machine) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
            let held = if held_user { &user } else { &machine };
            let _hold = hold_for_test(Role::Serve, held.path(), LockKind::Exclusive).unwrap();
            ensure_no_cross_scope_conflict_in(Role::Serve, scope, &dirs(&user, &machine))
                .unwrap_or_else(|e| panic!("{scope:?} re-install must pass: {e:#}"));
        }
    }

    #[test]
    fn a_registered_but_stopped_service_does_not_refuse() {
        let (user, machine) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        drop(hold_for_test(Role::Serve, machine.path(), LockKind::Exclusive).unwrap());
        ensure_no_cross_scope_conflict_in(Role::Serve, ServiceScope::User, &dirs(&user, &machine))
            .expect("an unheld lock file is not a running role");
    }

    #[cfg(unix)]
    #[test]
    fn a_system_install_probes_the_resolved_accounts_home() {
        let account = tempfile::tempdir().unwrap();
        let state = crate::paths::state_dir_in(account.path());
        std::fs::create_dir_all(&state).unwrap(); // create_lock_file does not create parents
        let _hold = hold_for_test(Role::Serve, &state, LockKind::Exclusive).unwrap();
        ensure_no_cross_scope_conflict(
            Role::Serve,
            ServiceScope::System,
            Some(account.path()),
            &LockDirs::resolve,
        )
        .expect_err("the resolved account's per-user tier must refuse a system install");
        let other = tempfile::tempdir().unwrap();
        ensure_no_cross_scope_conflict(
            Role::Serve,
            ServiceScope::System,
            Some(other.path()),
            &LockDirs::resolve,
        )
        .expect("another account's empty home must not be refused");
    }

    #[cfg(unix)]
    #[test]
    fn a_conflicting_install_changes_nothing() {
        const NAME: &str = "service::install::tests::a_conflicting_install_changes_nothing";
        in_isolated_home(NAME, |home| {
            #[cfg(target_os = "linux")]
            if !linux_is_systemd() {
                return; // BusyBox registers system-wide and needs real root
            }
            let config = write_valid_config(home);
            let mut req = request(Some(ServiceScope::User));
            req.config = Some(config);
            req.state_dir_override = Some(home.join("state"));
            let (user, machine) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
            let _hold = hold_for_test(Role::Serve, machine.path(), LockKind::Exclusive).unwrap();
            let resolve = |_: Option<&Path>| Ok(dirs(&user, &machine));
            let (result, log) = run_install_in(&req, BATCH, vec![], &resolve);
            let failure = result.expect_err("the machine-tier hold must refuse");
            assert!(failure.applied.is_empty(), "{:?}", failure.applied);
            assert!(
                commands(&log).is_empty(),
                "the refusal precedes every command"
            );
            assert_eq!(existing_registration("serve", ServiceScope::User), None);
            let err = anyhow::Error::from(failure);
            assert!(
                matches!(err.downcast_ref::<CliError>(), Some(CliError::Conflict(_))),
                "{err:?}"
            );
        });
    }

    #[cfg(unix)]
    #[test]
    fn privilege_is_refused_before_a_cross_scope_conflict() {
        if crate::service::is_elevated() {
            return;
        }
        let (user, machine) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let _hold = hold_for_test(Role::Serve, user.path(), LockKind::Exclusive).unwrap();
        let mut req = request(Some(ServiceScope::System));
        req.config = Some(PathBuf::from("/tmp/iroh-tunnel-test/serve.toml"));
        let resolve = |_: Option<&Path>| Ok(dirs(&user, &machine));
        let (result, log) = run_install_in(&req, BATCH, vec![], &resolve);
        let err = anyhow::Error::from(result.expect_err("must be refused"));
        assert!(
            matches!(
                err.downcast_ref::<CliError>(),
                Some(CliError::Permission(_))
            ),
            "{err:?}"
        );
        assert!(commands(&log).is_empty(), "{log:?}");
    }

    #[cfg(unix)]
    #[test]
    fn install_without_scope_returns_scope_choice_need() {
        let (result, log) = run_install(&request(None), CAN_ANSWER, vec![]);
        assert!(matches!(
            result,
            Ok(InstallOutcome::Needs(InstallNeed::ScopeChoice))
        ));
        assert!(commands(&log).is_empty(), "{log:?}");
    }

    #[cfg(unix)]
    #[test]
    fn unprivileged_system_install_is_refused_with_guidance() {
        let mut req = request(Some(ServiceScope::System));
        req.config = Some(PathBuf::from("/tmp/iroh-tunnel-test/serve.toml"));
        let (result, log) = run_install(&req, BATCH, vec![]);
        let err = anyhow::Error::from(result.expect_err("must be refused"));
        let Some(CliError::Permission(msg)) = err.downcast_ref::<CliError>() else {
            panic!("expected Permission, got {err:?}");
        };
        assert!(msg.contains("sudo"), "{msg}");
        assert!(msg.contains("serve service install --system"), "{msg}");
        assert!(msg.contains("/tmp/iroh-tunnel-test/serve.toml"), "{msg}");
        assert!(commands(&log).is_empty(), "{log:?}");
    }

    #[test]
    fn standalone_install_privilege_guidance_uses_invoked_binary_and_absolute_config() {
        for role in [Role::Serve, Role::Access] {
            let mut req = request(Some(ServiceScope::System));
            req.role = role;
            req.tunnel_executable = PathBuf::from(if cfg!(windows) {
                r"C:\Program Files\role bin\renamed.exe"
            } else {
                "/opt/role bin/renamed"
            });
            req.invocation = ServiceInvocation::Standalone {
                executable: req.tunnel_executable.clone(),
            };
            req.config = Some(PathBuf::from("relative config.toml"));
            let (result, log) = run_install(&req, BATCH, vec![]);
            let err = anyhow::Error::from(result.expect_err("must refuse"));
            let message = err.to_string();
            let command = if cfg!(windows) {
                r"& 'C:\Program Files\role bin\renamed.exe' service install --system --config"
            } else {
                "sudo '/opt/role bin/renamed' service install --system --config"
            };
            assert!(message.contains(command), "{message}");
            assert!(matches!(
                err.downcast_ref::<CliError>(),
                Some(CliError::Permission(_))
            ));
            assert!(
                message.contains(
                    &std::env::current_dir()
                        .unwrap()
                        .join("relative config.toml")
                        .display()
                        .to_string()
                ),
                "{message}"
            );
            assert!(commands(&log).is_empty());
        }
    }

    fn conflict_text(requested: ServiceScope, role: Role) -> String {
        let err = cross_scope_conflict(requested, true, role).expect_err("must be refused");
        let Some(CliError::Service(msg)) = err.downcast_ref::<CliError>() else {
            panic!("expected Service, got {err:?}");
        };
        msg.clone()
    }

    #[test]
    fn system_install_refused_when_user_task_exists() {
        let msg = conflict_text(ServiceScope::System, Role::Serve);
        assert!(
            msg.contains("iroh-tunnel serve service uninstall --user"),
            "{msg}"
        );
        assert!(msg.contains("user task"), "{msg}");
    }

    #[test]
    fn user_install_refused_when_system_service_exists() {
        let msg = conflict_text(ServiceScope::User, Role::Access);
        assert!(
            msg.contains("iroh-tunnel access service uninstall --system"),
            "{msg}"
        );
        assert!(msg.contains("system service"), "{msg}");
    }

    #[test]
    fn no_conflict_when_other_absent() {
        for scope in [ServiceScope::System, ServiceScope::User] {
            assert!(cross_scope_conflict(scope, false, Role::Serve).is_ok());
        }
    }

    #[test]
    fn relative_tunnel_executable_is_rejected() {
        let mut req = request(Some(ServiceScope::User));
        req.tunnel_executable = PathBuf::from("iroh-tunnel");
        let (result, log) = run_install(&req, BATCH, vec![]);
        let err = anyhow::Error::from(result.expect_err("must be rejected"));
        assert!(
            matches!(err.downcast_ref::<CliError>(), Some(CliError::Service(_))),
            "{err:?}"
        );
        assert!(commands(&log).is_empty(), "{log:?}");
    }

    #[cfg(unix)]
    #[test]
    fn install_renders_supplied_tunnel_executable() {
        const NAME: &str = "service::install::tests::install_renders_supplied_tunnel_executable";
        in_isolated_home(NAME, |home| {
            #[cfg(target_os = "linux")]
            if !linux_is_systemd() {
                return; // the BusyBox backend needs real root
            }
            let config = write_valid_config(home);
            let mut req = request(Some(ServiceScope::User));
            req.config = Some(config);
            req.state_dir_override = Some(home.join("state"));
            let (result, _log) = run_install(&req, BATCH, vec![]);
            let Ok(InstallOutcome::Installed(report)) = result else {
                panic!("install failed");
            };
            let Some(AppliedEffect::RegistrationWritten(path)) = report.applied.first() else {
                panic!("no registration written: {:?}", report.applied);
            };
            let body = std::fs::read_to_string(path).unwrap();
            assert!(body.contains("/opt/x/iroh-tunnel"), "{body}");
        });
    }

    #[cfg(unix)]
    #[test]
    fn partial_install_reports_written_registration() {
        const NAME: &str = "service::install::tests::partial_install_reports_written_registration";
        in_isolated_home(NAME, |home| {
            #[cfg(target_os = "linux")]
            if !linux_is_systemd() {
                return;
            }
            let config = write_valid_config(home);
            let mut req = request(Some(ServiceScope::User));
            req.config = Some(config);
            req.state_dir_override = Some(home.join("state"));
            // Linux: daemon-reload ok, `enable` fails. macOS: `enable` and
            // `bootout` ok, `bootstrap` fails.
            let responses = if cfg!(target_os = "linux") {
                vec![
                    crate::service::native::fake::output(0, "", ""),
                    crate::service::native::fake::output(1, "", "boom"),
                ]
            } else {
                vec![
                    crate::service::native::fake::output(0, "", ""),
                    crate::service::native::fake::output(0, "", ""),
                    crate::service::native::fake::output(5, "", "boom"),
                ]
            };
            let (result, log) = run_install(&req, BATCH, responses);
            let failure = result.expect_err("the manager step must fail");
            assert!(
                matches!(
                    failure.applied.as_slice(),
                    [AppliedEffect::RegistrationWritten(_)]
                ),
                "{:?}",
                failure.applied
            );
            let programs: Vec<String> = commands(&log).iter().map(|e| format!("{e:?}")).collect();
            assert!(
                !programs
                    .iter()
                    .any(|c| c.contains("\"stop\"") || c.contains("\"disable\"")),
                "no rollback command may run: {programs:?}"
            );
        });
    }

    #[cfg(unix)]
    #[test]
    fn abandoned_install_at_decision_changes_nothing() {
        const NAME: &str = "service::install::tests::abandoned_install_at_decision_changes_nothing";
        in_isolated_home(NAME, |home| {
            let config = write_valid_config(home);
            let before = std::fs::read(&config).unwrap();
            let registration_before = existing_registration("serve", ServiceScope::System);
            let env = Environment {
                elevated: true,
                can_answer: true,
            };
            let (result, log) = run_install(&request(Some(ServiceScope::System)), env, vec![]);
            assert!(matches!(
                result,
                Ok(InstallOutcome::Needs(InstallNeed::InvokingUser))
            ));
            assert!(commands(&log).is_empty(), "{log:?}");
            assert_eq!(std::fs::read(&config).unwrap(), before);
            assert_eq!(
                existing_registration("serve", ServiceScope::System),
                registration_before
            );
        });
    }

    #[test]
    fn install_request_on_windows_no_longer_forces_user_scope() {
        // Neither a TTY nor elevation: user. Elevated: system, never forced to user.
        for (elevated, can_answer, expected) in [
            (true, true, ScopeDecision::Chosen(ServiceScope::System)),
            (false, true, ScopeDecision::NeedsChoice),
        ] {
            let env = Environment {
                elevated,
                can_answer,
            };
            assert_eq!(decide_install_scope(request(None).scope, env), expected);
        }
    }

    #[test]
    fn uninstallable_config_names_the_cause_once() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("serve.toml");
        std::fs::write(&config, "not = [valid").unwrap();
        let err = check_installable_config(Role::Serve, &config).unwrap_err();
        let Some(CliError::Config(msg)) = err.downcast_ref::<CliError>() else {
            panic!("expected CliError::Config, got {err:?}");
        };
        assert!(
            msg.starts_with("cannot install the service: failed to parse config: "),
            "{msg}"
        );
    }

    #[test]
    fn uninstallable_config_keeps_an_io_cause_in_the_chain() {
        let dir = tempfile::tempdir().unwrap();
        // A directory where the config file should be: reading it fails with
        // an I/O error, not a parse error.
        let err = check_installable_config(Role::Serve, dir.path()).unwrap_err();
        assert!(
            matches!(err.downcast_ref::<CliError>(), Some(CliError::Config(_))),
            "{err:?}"
        );
        assert!(
            err.chain()
                .any(<dyn std::error::Error>::is::<std::io::Error>),
            "the I/O error must stay reachable: {err:?}"
        );
    }

    /// A `verify` that must never run.
    #[expect(
        clippy::panic_in_result_fn,
        reason = "test double: reaching it is the failure under test"
    )]
    fn no_verify(path: &Path) -> Result<()> {
        panic!("the binary check must not run for {}", path.display())
    }

    #[test]
    fn plan_binary_user_scope_is_resolved_path() {
        let req = request(Some(ServiceScope::User));
        let planned = plan_binary(&req, ServiceScope::User, false, &no_verify).unwrap();
        assert_eq!(
            planned,
            ServiceBinary::Direct(req.tunnel_executable.clone())
        );
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    #[test]
    fn plan_binary_non_unix_is_direct() {
        let req = request(Some(ServiceScope::System));
        for (scope, system_wide_only) in [
            (ServiceScope::System, false),
            (ServiceScope::User, true),
            (ServiceScope::User, false),
        ] {
            let planned = plan_binary(&req, scope, system_wide_only, &no_verify).unwrap();
            assert_eq!(
                planned,
                ServiceBinary::Direct(req.tunnel_executable.clone())
            );
        }
    }

    #[test]
    fn install_steps_writes_copy_then_note_then_registers_copy() {
        let copy = PathBuf::from("/m/iroh-tunnel/bin/serve/iroh-tunnel");
        let binary = ServiceBinary::Managed {
            source: request(None).running_executable,
            copy: copy.clone(),
        };
        let run = run_steps(&binary, Ok(()));
        run.result.unwrap();
        assert_eq!(
            run.order,
            vec![
                "materialize".to_owned(),
                format!("note {}", copy.display()),
                format!("register {}", copy.display()),
            ]
        );
        assert_eq!(
            run.applied,
            vec![AppliedEffect::ManagedBinaryWritten(copy.clone())]
        );
        assert_eq!(
            run.notes,
            vec![ServiceNote::ManagedBinary {
                role: "serve".into(),
                source: request(None).running_executable.display,
                copy,
                config: PathBuf::from(STEPS_CONFIG),
            }]
        );
    }

    #[test]
    fn install_steps_register_failure_keeps_copy_in_applied() {
        let copy = PathBuf::from("/m/iroh-tunnel/bin/serve/iroh-tunnel");
        let binary = ServiceBinary::Managed {
            source: request(None).running_executable,
            copy: copy.clone(),
        };
        let run = run_steps(&binary, Err("unit write failed"));
        assert_eq!(run.result.unwrap_err().to_string(), "unit write failed");
        assert_eq!(run.applied, vec![AppliedEffect::ManagedBinaryWritten(copy)]);
        assert!(
            !run.order.iter().any(|step| step == "release"),
            "{:?}",
            run.order
        );
    }

    #[test]
    fn install_steps_direct_releases_copy() {
        let direct = PathBuf::from("/usr/local/bin/iroh-tunnel");
        let run = run_steps(&ServiceBinary::Direct(direct.clone()), Ok(()));
        run.result.unwrap();
        assert_eq!(
            run.order,
            vec![
                "materialize".to_owned(),
                format!("register {}", direct.display()),
                "release".to_owned(),
            ]
        );
        assert!(run.notes.is_empty(), "{:?}", run.notes);
        assert_eq!(
            run.applied,
            vec![AppliedEffect::ManagedBinaryRemoved(PathBuf::from(
                "/m/iroh-tunnel/bin/serve/iroh-tunnel"
            ))]
        );

        // A failed registration releases nothing.
        let run = run_steps(&ServiceBinary::Direct(direct), Err("boom"));
        assert!(run.result.is_err());
        assert!(!run.order.iter().any(|step| step == "release"));
        assert!(run.applied.is_empty(), "{:?}", run.applied);
    }

    /// The config every [`run_steps`] registration pins.
    const STEPS_CONFIG: &str = "/home/a/serve.toml";

    /// What one [`install_steps`] run did.
    struct StepsRun {
        result: Result<()>,
        order: Vec<String>,
        notes: Vec<ServiceNote>,
        applied: Vec<AppliedEffect>,
    }

    /// Run [`install_steps`] with recording effects: `materialize` writes the
    /// managed copy of a `Managed` binary, `register` returns `registered`
    /// and `release_after_direct` removes a fixed copy.
    fn run_steps(binary: &ServiceBinary, registered: Result<(), &str>) -> StepsRun {
        use super::super::native::fake::{Events, FakeRunner};
        use super::super::native::OutputMode;
        use std::cell::RefCell;

        let order = RefCell::new(Vec::new());
        let mut notes_seen = Vec::new();
        let events = Events::default();
        let mut fake = FakeRunner::new(&events, Vec::new());
        let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
        let mut native = Native::new(OutputMode::Capture, &mut runner);
        let mut notes = |note: &ServiceNote| {
            if let ServiceNote::ManagedBinary { copy, .. } = note {
                order.borrow_mut().push(format!("note {}", copy.display()));
            }
            notes_seen.push(note.clone());
        };
        let result = install_steps(
            &mut native,
            &mut notes,
            "serve",
            &InstallPlan {
                scope: ServiceScope::System,
                config: PathBuf::from(STEPS_CONFIG),
                state: PathBuf::from("/home/a/state"),
                pin: None,
                binary: binary.clone(),
            },
            |binary| {
                order.borrow_mut().push("materialize".into());
                Ok(match binary {
                    ServiceBinary::Direct(_) => None,
                    ServiceBinary::Managed { copy, .. } => {
                        Some(AppliedEffect::ManagedBinaryWritten(copy.clone()))
                    }
                })
            },
            |_, _, path| {
                order
                    .borrow_mut()
                    .push(format!("register {}", path.display()));
                registered.map_err(|why| anyhow::anyhow!("{why}"))
            },
            |_| {
                order.borrow_mut().push("release".into());
                Ok(Some(AppliedEffect::ManagedBinaryRemoved(PathBuf::from(
                    "/m/iroh-tunnel/bin/serve/iroh-tunnel",
                ))))
            },
        );
        let (applied, _) = native.finish();
        StepsRun {
            result,
            order: order.into_inner(),
            notes: notes_seen,
            applied,
        }
    }

    #[test]
    fn verbatim_prefix_is_stripped_but_not_for_unc() {
        let strip = |p: &str| strip_verbatim_prefix(PathBuf::from(p));
        assert_eq!(strip(r"\\?\C:\a\b.toml"), PathBuf::from(r"C:\a\b.toml"));
        assert_eq!(strip(r"C:\a\b.toml"), PathBuf::from(r"C:\a\b.toml"));
        assert_eq!(
            strip(r"\\?\UNC\srv\share\b.toml"),
            PathBuf::from(r"\\?\UNC\srv\share\b.toml")
        );
    }
}

/// The binary decision of root-run registrations on Linux and macOS.
#[cfg(test)]
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix_binary_tests {
    use super::super::fixtures::{in_isolated_home, request, write_valid_config};
    use super::super::pin::{Pin, PinOwner};
    use super::super::service_binary::managed_copy_path;
    #[cfg(all(feature = "test-utils", target_os = "linux"))]
    use super::super::types::ServiceInvocation;
    use super::*;

    fn refuse(path: &Path) -> Result<()> {
        Err(CliError::Permission(format!("{} is not root-only", path.display())).into())
    }

    /// A request whose resolved binary exists, so it can be canonicalized.
    fn request_with_binary(scope: ServiceScope) -> (tempfile::TempDir, InstallRequest) {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("iroh-tunnel");
        std::fs::write(&binary, "bin").unwrap();
        let mut req = request(Some(scope));
        req.tunnel_executable = binary;
        (dir, req)
    }

    #[test]
    fn plan_binary_system_scope_managed_when_refused() {
        let (_dir, req) = request_with_binary(ServiceScope::System);
        let planned = plan_binary(&req, ServiceScope::System, false, &refuse).unwrap();
        assert_eq!(
            planned,
            ServiceBinary::Managed {
                source: req.running_executable.clone(),
                copy: managed_copy_path("serve"),
            }
        );

        let canonical = req.tunnel_executable.canonicalize().unwrap();
        let planned = plan_binary(&req, ServiceScope::System, false, &|path| {
            assert_eq!(path, canonical, "the canonical path is checked");
            Ok(())
        })
        .unwrap();
        assert_eq!(planned, ServiceBinary::Direct(canonical));
    }

    #[test]
    fn unprivileged_root_run_user_install_is_refused_before_any_decision() {
        let (_dir, req) = request_with_binary(ServiceScope::User);
        let env = Environment {
            elevated: false,
            can_answer: false,
        };
        let err = plan_user_install(
            &req,
            env,
            &mut |_, _| panic!("no probe before the privilege refusal"),
            &|_| panic!("no lock directories before the privilege refusal"),
            true,
        )
        .err()
        .expect("must be refused");
        let Some(CliError::Permission(msg)) = err.downcast_ref::<CliError>() else {
            panic!("expected Permission, got {err:?}");
        };
        assert!(msg.contains("Re-run with sudo"), "{msg}");
        assert!(msg.contains("Nothing was changed"), "{msg}");
    }

    #[test]
    fn plan_binary_busybox_user_scope_is_root_run() {
        let (_dir, req) = request_with_binary(ServiceScope::User);
        let planned = plan_binary(&req, ServiceScope::User, true, &refuse).unwrap();
        assert_eq!(
            planned,
            ServiceBinary::Managed {
                source: req.running_executable.clone(),
                copy: managed_copy_path("serve"),
            }
        );
        let planned = plan_binary(&req, ServiceScope::User, true, &|_| Ok(())).unwrap();
        assert_eq!(
            planned,
            ServiceBinary::Direct(req.tunnel_executable.canonicalize().unwrap())
        );
    }

    #[cfg(all(feature = "test-utils", target_os = "linux"))]
    #[test]
    fn managed_standalone_selection_materializes_and_registers_both_roles() {
        use super::super::native::fake::{Events, FakeRunner};
        use super::super::native::OutputMode;
        use super::super::trust::TrustedOwners;
        use std::os::unix::fs::PermissionsExt;
        const NAME: &str = "service::install::unix_binary_tests::managed_standalone_selection_materializes_and_registers_both_roles";
        in_isolated_home(NAME, |home| {
            let base = home.join("managed path");
            std::fs::create_dir(&base).unwrap();
            std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::env::set_var("IROH_TUNNEL_TEST_MANAGED_BIN_DIR", &base);
            for role in [Role::Serve, Role::Access] {
                let source = home.join(format!("renamed-{}", role.name()));
                std::fs::write(
                    &source,
                    format!("#!/bin/sh\nprintf '%s:%s' '{}' \"$1\"\n", role.name()),
                )
                .unwrap();
                std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o755)).unwrap();
                let mut req = request(Some(ServiceScope::System));
                req.role = role;
                req.invocation = ServiceInvocation::Standalone {
                    executable: source.clone(),
                };
                req.tunnel_executable = source.clone();
                req.running_executable = super::super::RunningExecutable {
                    display: source.clone(),
                    read_from: source.clone(),
                };
                // The writable source must fail the real root-only trust check.
                std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o777)).unwrap();
                assert!(verify_root_only(&source).is_err());
                let chosen =
                    plan_binary(&req, ServiceScope::System, false, &verify_root_only).unwrap();
                assert!(matches!(chosen, ServiceBinary::Managed { .. }));
                let events = Events::default();
                let mut fake = FakeRunner::new(&events, vec![]);
                let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
                let mut native = Native::new(OutputMode::Capture, &mut runner);
                let config = home.join("config");
                let plan = InstallPlan {
                    scope: ServiceScope::System,
                    config: config.clone(),
                    state: home.join("state"),
                    pin: None,
                    binary: chosen.clone(),
                };
                install_steps(
                    &mut native,
                    &mut |_| {},
                    role.name(),
                    &plan,
                    |binary| {
                        super::super::service_binary::materialize(
                            binary,
                            TrustedOwners::RootAnd(unsafe { libc::geteuid() }),
                        )
                    },
                    |native, notes, path| {
                        super::super::backend::systemd::install(
                            native,
                            notes,
                            &RegistrationInput {
                                invocation: &req.invocation,
                                ..super::super::fixtures::registration(
                                    role.name(),
                                    path,
                                    &config,
                                    &home.join("state"),
                                )
                            },
                        )
                    },
                    |_| panic!("managed copy must not be released"),
                )
                .unwrap();
                assert_eq!(
                    std::fs::read(chosen.path()).unwrap(),
                    std::fs::read(&source).unwrap()
                );
                let unit = std::fs::read_to_string(home.join(format!(
                    ".config/systemd/user/iroh-tunnel-{}.service",
                    role.name()
                )))
                .unwrap();
                let expected = format!("ExecStart=\"{}\" run --config", chosen.path().display());
                assert!(unit.contains(&expected), "{unit}");
                let output = std::process::Command::new(chosen.path())
                    .arg("run")
                    .output()
                    .unwrap();
                assert!(output.status.success());
                assert_eq!(
                    String::from_utf8(output.stdout).unwrap(),
                    format!("{}:run", role.name())
                );
            }
        });
    }

    #[cfg(feature = "test-utils")]
    #[test]
    fn declined_retarget_writes_no_copy() {
        const NAME: &str = "service::install::unix_binary_tests::declined_retarget_writes_no_copy";
        in_isolated_home(NAME, |home| {
            let managed = tempfile::tempdir().unwrap();
            std::env::set_var("IROH_TUNNEL_TEST_MANAGED_BIN_DIR", managed.path());
            let config = write_valid_config(home);
            let state = home.join("state");
            std::fs::create_dir(&state).unwrap();
            let (_bin, mut req) = request_with_binary(ServiceScope::System);
            req.config = Some(config);
            req.state_dir_override = Some(state);
            // SAFETY: geteuid(2) has no preconditions and cannot fail.
            let me = unsafe { libc::geteuid() };
            let other = Pin {
                config: PathBuf::from("/home/other/serve.toml"),
                owner: PinOwner::Uid(me.wrapping_add(1)),
            };
            let body = format!("# {}\n", other.record());
            let (user, machine) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
            let resolve = |_: Option<&Path>| {
                Ok(LockDirs {
                    user: user.path().into(),
                    machine: Some(machine.path().into()),
                })
            };
            let env = Environment {
                elevated: true,
                can_answer: true,
            };
            let planned =
                plan_system_install_with(&req, env, &resolve, &|_| Some(body.clone()), &|path| {
                    panic!("no binary decision while a retarget is pending: {path:?}")
                })
                .unwrap_or_else(|e| panic!("plan failed: {e:#}"));
            assert!(
                matches!(
                    planned,
                    Planned::Needs(InstallNeed::Retarget {
                        previous: Some(_),
                        ..
                    })
                ),
                "a retarget must be asked first"
            );
            let entries: Vec<_> = std::fs::read_dir(managed.path()).unwrap().collect();
            assert!(entries.is_empty(), "{entries:?}");
        });
    }
}

/// Windows plan against the real token; gated like the other service tests.
#[cfg(test)]
#[cfg(windows)]
mod windows_plan_tests {
    use super::super::fixtures::request;
    use super::super::pin::PinOwner;
    use super::super::trust_windows::token_user;
    use super::*;

    /// The privilege refusal (exit 3) comes before the cross-scope probe runs.
    #[test]
    fn unprivileged_system_install_still_exits_3_before_cross_scope() {
        let mut req = request(Some(ServiceScope::System));
        req.config = Some(PathBuf::from(r"C:\iroh-tunnel-test\serve.toml"));
        let env = Environment {
            elevated: false,
            can_answer: false,
        };
        let mut probed = false;
        let err = plan_install(
            &req,
            env,
            &mut |_, _| {
                probed = true;
                Ok(true)
            },
            &|_| anyhow::bail!("the lock directories must not be resolved"),
        )
        .err()
        .expect("must be refused");
        assert!(!probed, "the probe must not run before the privilege check");
        assert!(
            matches!(
                err.downcast_ref::<CliError>(),
                Some(CliError::Permission(_))
            ),
            "{err:?}"
        );
    }

    #[test]
    fn system_install_plan_pins_the_token_sid() {
        if std::env::var_os("IROH_TUNNEL_E2E_SERVICE").is_none_or(|v| v != "1") {
            return;
        }
        let root = crate::paths::local_app_data()
            .unwrap()
            .join("iroh-tunnel-plan-test");
        let config_dir = root.join("config");
        std::fs::create_dir_all(&config_dir).unwrap();
        let config = config_dir.join("serve.toml");
        std::fs::write(
            &config,
            "[node]\nrelay_urls = []\n\n[[services]]\nname = \"echo\"\nprotocol = \"tcp\"\nport = 9\n",
        )
        .unwrap();
        let mut req = request(Some(ServiceScope::System));
        req.config = Some(config);
        req.state_dir_override = Some(root.join("state"));
        std::fs::create_dir_all(root.join("state")).unwrap();
        // The plan only reads the binary's trust, it never runs it. A test
        // binary in a user-owned build tree is refused by `verify_binary`, so
        // use a system one.
        let system_root = std::env::var_os("SystemRoot").expect("SystemRoot is set");
        req.tunnel_executable = PathBuf::from(system_root).join("System32\\cmd.exe");
        let env = Environment {
            elevated: true,
            can_answer: false,
        };
        let user = tempfile::tempdir().unwrap();
        let resolve = |_: Option<&Path>| {
            Ok(LockDirs {
                user: user.path().to_path_buf(),
                machine: None,
            })
        };
        let planned = plan_install(&req, env, &mut |_, _| Ok(false), &resolve);
        let _ = std::fs::remove_dir_all(&root);
        let Ok(Planned::Ready(plan)) = planned else {
            panic!("expected a ready plan");
        };
        let pin = plan.pin.expect("a system plan pins its owner");
        assert_eq!(pin.owner, PinOwner::Sid(token_user().unwrap().0));
    }

    fn icacls(args: &[&str]) -> Vec<u8> {
        let out = std::process::Command::new("icacls")
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "icacls {args:?}");
        out.stdout
    }

    /// Admins plus the test user, as `verify_confidential` with a test owner.
    fn check(path: &Path) -> Result<()> {
        let trusted =
            super::super::trust_windows::model::TrustedSet::admin_with_test_owner(token_user()?);
        super::super::trust_windows::verify_confidential_with(path, &trusted)
    }

    fn refused_untouched(dir: &Path, config: &Path, grant: &str) {
        let dir_text = dir.to_str().unwrap();
        icacls(&[dir_text, "/grant", grant]);
        let before = icacls(&[dir_text]);
        let err = verify_config_confidential(config, &check).unwrap_err();
        assert!(
            matches!(
                err.downcast_ref::<CliError>(),
                Some(CliError::Permission(m)) if m.contains("S-1-5-32-545")
            ),
            "{err:#}"
        );
        assert_eq!(icacls(&[dir_text]), before, "DACL unchanged");
    }

    /// Serializes the tests that set the test-owner variable.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    const OWNER_VAR: &str = "IROH_TUNNEL_TEST_TRUSTED_OWNER";
    const USERS_SID: &str = "S-1-5-32-545";
    const KEY_CONFIG: &str = "[node]\nsecret_key = \"x\"\nrelay_urls = []\n\n[[services]]\nname = \"echo\"\nprotocol = \"tcp\"\nport = 9\n";

    /// Plan a system install of `config` (under the token user's LocalAppData)
    /// with the token user trusted as an admin principal; the result of
    /// `plan_install`, which never registers anything itself.
    #[expect(
        clippy::unwrap_in_result,
        reason = "test setup; only the plan's result is under test"
    )]
    fn plan_for(root: &Path, config: PathBuf) -> Result<Planned> {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::env::set_var(OWNER_VAR, token_user().unwrap().0);
        let mut req = request(Some(ServiceScope::System));
        req.config = Some(config);
        req.state_dir_override = Some(root.join("state"));
        let system_root = std::env::var_os("SystemRoot").expect("SystemRoot is set");
        req.tunnel_executable = PathBuf::from(system_root).join("System32\\cmd.exe");
        let env = Environment {
            elevated: true,
            can_answer: false,
        };
        let user = tempfile::tempdir().unwrap();
        let resolve = |_: Option<&Path>| {
            Ok(LockDirs {
                user: user.path().to_path_buf(),
                machine: None,
            })
        };
        let planned = plan_install(&req, env, &mut |_, _| Ok(false), &resolve);
        std::env::remove_var(OWNER_VAR);
        planned
    }

    /// A config under a fresh root with `KEY_CONFIG`; `grant` is applied to
    /// `target` (the directory or the file) before planning.
    fn assert_plan_refuses(name: &str, on_file: bool, grant: &str) {
        let root = crate::paths::local_app_data().unwrap().join(name);
        let dir = root.join("config");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(root.join("state")).unwrap();
        let config = dir.join("serve.toml");
        std::fs::write(&config, KEY_CONFIG).unwrap();
        let target = if on_file { &config } else { &dir };
        icacls(&[target.to_str().unwrap(), "/grant", grant]);
        let (dir_acl, file_acl) = (
            icacls(&[dir.to_str().unwrap()]),
            icacls(&[config.to_str().unwrap()]),
        );
        let planned = plan_for(&root, config.clone());
        let bytes = std::fs::read(&config).unwrap();
        let (dir_after, file_after) = (
            icacls(&[dir.to_str().unwrap()]),
            icacls(&[config.to_str().unwrap()]),
        );
        let _ = std::fs::remove_dir_all(&root);
        let err = planned.err().expect("a readable config must be refused");
        assert!(
            matches!(
                err.downcast_ref::<CliError>(),
                Some(CliError::Permission(m)) if m.contains(USERS_SID)
            ),
            "{err:#}"
        );
        assert_eq!(bytes, KEY_CONFIG.as_bytes(), "config bytes unchanged");
        assert_eq!(dir_after, dir_acl, "directory DACL unchanged");
        assert_eq!(file_after, file_acl, "file DACL unchanged");
    }

    fn e2e_enabled() -> bool {
        std::env::var_os("IROH_TUNNEL_E2E_SERVICE").is_some_and(|v| v == "1")
    }

    /// `plan_install` returns a plan or an error; registration only happens
    /// when the caller executes a returned plan, so an `Err` means the
    /// registration step is never reached.
    #[test]
    fn plan_install_refuses_a_user_readable_config_dir() {
        if e2e_enabled() {
            assert_plan_refuses("iroh-tunnel-plan-dir", false, "*S-1-5-32-545:(OI)(CI)R");
        }
    }

    #[test]
    fn plan_install_refuses_a_user_readable_config_file_with_a_key() {
        if e2e_enabled() {
            assert_plan_refuses("iroh-tunnel-plan-file", true, "*S-1-5-32-545:R");
        }
    }

    #[test]
    fn install_refuses_a_user_readable_config_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path().join("serve.toml");
        refused_untouched(tmp.path(), &config, "*S-1-5-32-545:(OI)(CI)R");
        assert!(!config.exists());
    }

    #[test]
    fn install_refuses_a_config_dir_with_inheritable_only_read() {
        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path().join("serve.toml");
        refused_untouched(tmp.path(), &config, "*S-1-5-32-545:(OI)(IO)R");
        assert!(!config.exists());
    }
}
