//! `service install`: decide, check, then write the registration, enable and
//! start it.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::decide_install_scope::decide_install_scope;
use super::execute::execute;
#[allow(unused_imports)] // unused on targets without a service backend
use super::native::system_runner;
use super::native::{Native, Runner, ServiceCall, ServiceNote};
use super::pin::Pin;
use super::policy::{ensure_privilege, ensure_supported};
use super::types::{
    Environment, InstallNeed, InstallOutcome, InstallRequest, ScopeDecision, ServiceFailure,
    ServiceScope,
};
use crate::error::CliError;
use crate::role::Role;

/// The argv (after `sudo`) that repeats an install with absolute paths.
pub(crate) fn install_rerun(role: Role, scope: ServiceScope, config: &Path) -> Vec<String> {
    [
        "iroh-tunnel",
        role.name(),
        "service",
        "install",
        super::verb::flag(scope),
        "--config",
    ]
    .into_iter()
    .map(str::to_owned)
    .chain([config.to_string_lossy().into_owned()])
    .collect()
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

/// [`install`] over an injected command runner.
pub(crate) fn install_via(
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
    let scope = match decide_install_scope(requested, env) {
        ScopeDecision::Chosen(scope) => scope,
        ScopeDecision::NeedsChoice => return Ok(Planned::Needs(InstallNeed::ScopeChoice)),
    };
    ensure_supported(scope)?;
    if scope == ServiceScope::System {
        return plan_system_install(req, env);
    }
    // Persist absolute paths: the installed service runs with a different CWD
    // and home, so nothing may stay relative (SM-02).
    let config = absolutize_config_path(&crate::config::resolve_config_path(
        role,
        req.config.as_deref(),
    )?)?;
    let state = pinned_state_dir(req.state_dir_override.as_deref())?;
    let rerun = install_rerun(role, scope, &config);
    ensure_privilege(scope, env.elevated, &rerun)?;
    check_installable_config(role, &config)?;
    Ok(Planned::Ready(InstallPlan {
        scope,
        config,
        state,
        pin: None,
    }))
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
        let config = absolutize_config_path(&crate::config::resolve_config_path(
            role,
            req.config.as_deref(),
        )?)?;
        let rerun = install_rerun(role, ServiceScope::System, &config);
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
    // Fail closed before registering anything root would later refuse to run.
    trust::verify_dir(&target.state, target.account.uid)?;
    let text = trust::read_verified(&target.config, target.account.uid)?;
    check_installable_text(role, &text, &target.config)?;
    // Read the registration last, right before the write, so a consent given
    // for an earlier look at it is checked against the current one.
    let existing = super::platform::existing_registration(role.name(), ServiceScope::System);
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
    ensure_supported(ServiceScope::System)?;
    Err(CliError::Service("system services are not supported on this platform".into()).into())
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
        if super::platform::linux_is_systemd() {
            super::backend::systemd::install(native, notes, role, scope, binary, config, state, pin)
        } else {
            super::backend::busybox::install(native, notes, role, scope, binary, config, state, pin)
        }
    }
    #[cfg(target_os = "macos")]
    {
        super::backend::launchd::install(native, notes, role, scope, binary, config, state, pin)
    }
    #[cfg(windows)]
    {
        super::backend::windows::install(native, notes, role, scope, binary, config, state)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        super::platform::unsupported()
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::{commands, request, run_install, BATCH};
    #[cfg(unix)]
    use super::super::fixtures::{in_isolated_home, write_valid_config, CAN_ANSWER};
    #[cfg(unix)]
    use super::super::platform::existing_registration;
    #[cfg(target_os = "linux")]
    use super::super::platform::linux_is_systemd;
    #[cfg(unix)]
    use super::super::types::AppliedEffect;
    use super::*;
    use std::path::PathBuf;

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
            // Linux: daemon-reload ok, `enable` fails. macOS: `bootout` ok,
            // `bootstrap` fails.
            let responses = if cfg!(target_os = "linux") {
                vec![
                    crate::service::native::fake::output(0, "", ""),
                    crate::service::native::fake::output(1, "", "boom"),
                ]
            } else {
                vec![
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

    #[cfg(windows)]
    #[test]
    fn windows_system_scope_is_service_error() {
        let env = Environment {
            elevated: true,
            can_answer: false,
        };
        let (result, _) = run_install(&request(Some(ServiceScope::System)), env, vec![]);
        let err = anyhow::Error::from(result.expect_err("must be refused"));
        let Some(CliError::Service(msg)) = err.downcast_ref::<CliError>() else {
            panic!("expected Service, got {err:?}");
        };
        assert!(msg.contains("--user"), "{msg}");
    }
}
