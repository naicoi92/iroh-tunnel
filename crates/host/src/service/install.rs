//! `service install`: decide, check, then write the registration, enable and
//! start it.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::decide_install_scope::decide_install_scope;
use super::execute::execute;
#[allow(unused_imports)] // unused on targets without a service backend
use super::native::system_runner;
#[cfg(windows)]
use super::native::InstalledAt;
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
    let scope = match decide_install_scope(req.scope, env) {
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

/// Install a Windows service for the installing (token) user's files (SM-04).
/// Unprivileged callers are refused with rerun guidance; every file the
/// service will trust is checked before anything is registered.
#[cfg(windows)]
fn plan_system_install(req: &InstallRequest, env: Environment) -> Result<Planned> {
    use super::backend::windows_scm::{existing_registration_body, service_name};
    use super::backend::windows_scm_impl::Scm;
    use super::pin::{retarget_decision_for, RetargetDecision};
    use super::system_windows::{resolve_target, WinRequest};
    use super::trust_windows::model::TrustedSet;
    use super::trust_windows::{
        account_of, owner_of, read_verified, token_user, verify_binary, verify_dir,
        verify_machine_dir,
    };

    let role = req.role;
    // Absolute before anything reads its parent: a bare filename has none.
    let absolute = |path: &Path| absolutize_config_path(path).map(strip_verbatim_prefix);
    let config_abs = req.config.as_deref().map(absolute).transpose()?;
    // A relative override is resolved here, like `--config`: the service runs
    // elsewhere and `verify_dir` refuses a relative path.
    let state_abs = req
        .state_dir_override
        .as_deref()
        .filter(|d| !d.as_os_str().is_empty())
        .map(|dir| {
            std::path::absolute(dir)
                .with_context(|| format!("cannot make state directory {} absolute", dir.display()))
        })
        .transpose()?;
    if !env.elevated {
        let config = match &config_abs {
            Some(config) => config.clone(),
            None => absolute(&crate::config::resolve_config_path(role, None)?)?,
        };
        let rerun = install_rerun(role, ServiceScope::System, &config);
        ensure_privilege(ServiceScope::System, false, &rerun)?;
    }

    let token = token_user()?;
    // The token user is the elevated process's own user: the shell resolves
    // its LocalAppData with the current token.
    let target = resolve_target(
        &WinRequest {
            role: role.name(),
            config: config_abs.as_deref(),
            state: state_abs.as_deref(),
        },
        &token,
        &|dir| owner_of(dir).ok(),
        &|_| crate::paths::local_app_data(),
        &account_of,
    )?;

    if let Some(machine) = crate::ownership::machine_dir() {
        if machine.verify && machine.path.exists() {
            verify_machine_dir(&machine.path).map_err(|e| machine_dir_refusal(e, &machine.path))?;
        }
    }

    // Fail closed before registering anything LocalSystem would later refuse.
    let trusted = TrustedSet::for_user_files(&target.sid);
    verify_dir(&target.state, &trusted)?;
    let text = read_verified(&target.config, &trusted)?;
    check_installable_text(role, &text, &target.config)?;
    verify_binary(&req.tunnel_executable)?;

    // Read the registration last, right before the write, so a consent given
    // for an earlier look at it is checked against the current one.
    let existing = existing_registration_body(&mut Scm, &service_name(role.name()))?;
    if let RetargetDecision::Needs { previous, proposed } =
        retarget_decision_for(existing.as_deref(), target.pin(), req.retarget.as_ref())
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
#[cfg_attr(not(windows), allow(dead_code))]
fn strip_verbatim_prefix(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with(r"UNC\") => PathBuf::from(rest),
        _ => path,
    }
}

#[cfg(not(any(unix, windows)))]
fn plan_system_install(_req: &InstallRequest, _env: Environment) -> Result<Planned> {
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
#[cfg(any(unix, windows))]
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
        match scope {
            ServiceScope::User => {
                super::backend::windows::install(native, notes, role, scope, binary, config, state)
            }
            ServiceScope::System => {
                install_windows_service(notes, role, binary, config, state, pin)
            }
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        super::platform::unsupported()
    }
}

/// Register, start and announce the SCM service of `role` (system scope).
#[cfg(windows)]
fn install_windows_service(
    notes: &mut dyn FnMut(&ServiceNote),
    role: &str,
    binary: &Path,
    config: &Path,
    state: &Path,
    pin: Option<&Pin>,
) -> Result<()> {
    use super::backend::windows_scm::{install_with, ServiceSpec, SystemClock};
    use super::backend::windows_scm_impl::Scm;
    use super::types::RestartPolicy;

    let pin = pin.context("internal: a Windows service needs a pin")?;
    let spec = ServiceSpec::render(role, binary, config, state, pin, &RestartPolicy::DEFAULT)?;
    install_with(&mut Scm, &mut SystemClock::new(), &spec)?;
    notes(&ServiceNote::Installed {
        name: spec.name,
        at: InstalledAt::WindowsService,
    });
    Ok(())
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

/// Windows plan against the real token; gated like the other service tests.
#[allow(clippy::mixed_attributes_style)]
#[cfg(test)]
mod windows_plan_tests {
    #![cfg(windows)]
    use super::super::fixtures::request;
    use super::super::pin::PinOwner;
    use super::super::trust_windows::token_user;
    use super::*;

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
        let planned = plan_install(&req, env);
        let _ = std::fs::remove_dir_all(&root);
        let Ok(Planned::Ready(plan)) = planned else {
            panic!("expected a ready plan");
        };
        let pin = plan.pin.expect("a system plan pins its owner");
        assert_eq!(pin.owner, PinOwner::Sid(token_user().unwrap().0));
    }
}
