//! `service install`: plan in the caller's tier (paths, trust, binary), then
//! write the registration, enable and start it.
//!
//! The tier comes from the caller's [`Environment`]: a non-elevated caller
//! installs a user service from its own files, an elevated one a system
//! service from the machine-wide files (design D1, D2). Neither tier refuses
//! because the role runs in the other one; that is reported as a
//! [`ServiceNote::OtherTierRunning`] (D6, D7).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::backend::Backend;
use super::execute::execute;
use super::native::{system_runner, Native, Runner, ServiceCall, ServiceNote};
use super::platform;
use super::policy::ensure_root_run_privilege;
use super::registration::RegistrationInput;
use super::service_binary::ServiceBinary;
use super::types::{
    AppliedEffect, Environment, InstallRequest, RestartPolicy, ServiceFailure, ServiceReport,
};
use super::EnvLookup;
use crate::error::CliError;
use crate::role::Role;
use crate::Tier;

/// `service install` in the caller's tier: check, then write the
/// registration, enable and start it.
///
/// A system install (elevated caller) uses the system-tier config and state
/// directory unless overridden, creates a missing state directory, pins both
/// in canonical form, verifies them against the admin-only owners and creates
/// the presence file before anything is registered. A user install uses the
/// caller's own files and needs no privilege.
///
/// # Errors
///
/// A [`ServiceFailure`] whose `error` is a [`CliError`] for policy refusals
/// (missing privilege on BusyBox, unusable config, untrusted path) and whose
/// `applied` lists what changed before a backend step failed.
pub fn install(
    req: &InstallRequest,
    env: Environment,
    call: &mut ServiceCall<'_>,
) -> Result<ServiceReport, ServiceFailure> {
    let mut runner = system_runner;
    install_via(req, env, call, &mut runner, &InstallSeams::production())
}

/// The host facts an install plan reads, injected so tests need no root.
pub(crate) struct InstallSeams<'a> {
    /// The process environment (the test-build trusted owner).
    pub env: EnvLookup<'a>,
    /// The root-only binary check of a root-run registration.
    pub verify_binary: &'a dyn Fn(&Path) -> Result<()>,
    /// Make sure the system tier's presence file of a role exists.
    pub ensure_presence: &'a dyn Fn(Role) -> Result<()>,
}

impl InstallSeams<'static> {
    /// The real environment, binary check and presence file.
    fn production() -> Self {
        Self {
            env: &super::process_env,
            verify_binary: &verify_service_binary,
            ensure_presence: &crate::ownership::ensure_presence_file,
        }
    }
}

/// [`install`] over an injected command runner and host seams.
pub(crate) fn install_via(
    req: &InstallRequest,
    env: Environment,
    call: &mut ServiceCall,
    runner: &mut Runner<'_>,
    seams: &InstallSeams<'_>,
) -> Result<ServiceReport, ServiceFailure> {
    let plan = plan_install(req, env, seams).map_err(ServiceFailure::before_effects)?;
    let role = req.role;
    let root_run = platform::root_run(plan.tier, platform::system_wide_only());
    execute(call, runner, |native, notes| {
        super::verb::note_other_tier(role, plan.tier, notes);
        install_steps(
            native,
            notes,
            role.name(),
            &plan,
            materialize_root_only,
            |native, notes, binary| {
                Backend::select(plan.tier)?.install(
                    native,
                    notes,
                    &RegistrationInput {
                        role: role.name(),
                        invocation: &req.invocation,
                        tier: plan.tier,
                        binary,
                        config: &plan.config,
                        state: &plan.state,
                        policy: &RestartPolicy::DEFAULT,
                    },
                )
            },
            |native| {
                super::service_binary::release_unreferenced_copy(native, role.name(), root_run)
            },
        )
    })
}

/// The binary the registration of `tier` runs.
///
/// A root-run registration on Linux or macOS runs the canonical resolved
/// binary when `verify` (the root-only binary check) accepts it, otherwise a
/// managed copy of the running executable. A Windows system service runs the
/// resolved binary only when `verify` (the machine trust check) accepts it.
/// Every other registration runs the resolved binary as given. Writes
/// nothing.
///
/// # Errors
///
/// [`CliError::Config`] when the resolved binary of a root-run registration
/// cannot be canonicalized; `verify`'s refusal for a Windows system service.
fn plan_binary(
    req: &InstallRequest,
    tier: Tier,
    system_wide_only: bool,
    verify: &dyn Fn(&Path) -> Result<()>,
) -> Result<ServiceBinary> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if platform::root_run(tier, system_wide_only) {
        return super::service_binary::choose(
            &req.tunnel_executable,
            &req.running_executable,
            super::service_binary::managed_copy_path(req.role.name()),
            verify,
        );
    }
    #[cfg(windows)]
    if tier == Tier::System {
        verify(&req.tunnel_executable)?;
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let _ = (tier, system_wide_only, verify);
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
fn verify_service_binary(path: &Path) -> Result<()> {
    super::trust::verify_binary(path, super::trust::TrustedOwners::RootOnly)
}

/// The machine trust check of the binary a Windows system service runs.
#[cfg(windows)]
fn verify_service_binary(path: &Path) -> Result<()> {
    super::trust_windows::verify_binary(path)
}

/// No service binary is checked on other targets.
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn verify_service_binary(_path: &Path) -> Result<()> {
    Ok(())
}

/// The checked inputs of one registration write.
struct InstallPlan {
    tier: Tier,
    config: PathBuf,
    state: PathBuf,
    /// What the registration runs.
    binary: ServiceBinary,
}

/// Every check that precedes the write: an absolute executable, privilege
/// where the registration runs as root, the read-only binary decision, then
/// the tier's own paths and trust checks and the presence file (system
/// tier). Nothing is written before every check passed; the managed copy is
/// materialized last.
fn plan_install(
    req: &InstallRequest,
    env: Environment,
    seams: &InstallSeams<'_>,
) -> Result<InstallPlan> {
    if !req.tunnel_executable.is_absolute() {
        return Err(CliError::Service(format!(
            "the tunnel executable {} is not an absolute path; the service must not depend on \
             its working directory",
            req.tunnel_executable.display()
        ))
        .into());
    }
    let tier = env.tier();
    let system_wide_only = platform::system_wide_only();
    ensure_root_run_privilege(platform::root_run(tier, system_wide_only), env.elevated)?;
    // The read-only binary decision precedes every filesystem effect (state
    // directory, presence file); the managed copy is written only after all
    // checks, by `install_steps`.
    let binary = plan_binary(req, tier, system_wide_only, seams.verify_binary)?;
    let (config, state) = match tier {
        Tier::User => plan_user_paths(req)?,
        Tier::System => plan_system_paths(req, seams)?,
    };
    Ok(InstallPlan {
        tier,
        config,
        state,
        binary,
    })
}

/// The user tier's absolute config and state directory, with the config
/// checked as installable. The service runs with another working directory,
/// so nothing may stay relative (SM-02).
fn plan_user_paths(req: &InstallRequest) -> Result<(PathBuf, PathBuf)> {
    // The default follows the injected tier, never the process's own.
    let config = absolutize_config_path(&match req.config.as_deref() {
        Some(path) => path.to_path_buf(),
        None => crate::config::resolve_config_path_for(req.role, Tier::User)?,
    })?;
    let state = absolute_dir(&match non_empty(req.state_dir_override.as_deref()) {
        Some(dir) => dir.to_path_buf(),
        None => crate::paths::default_state_dir_for(Tier::User)?,
    })?;
    check_installable_config(req.role, &config)?;
    Ok((config, state))
}

/// The system tier's config and state directory: the machine-wide defaults
/// unless overridden, never a user's home (D2). The config must exist (else
/// the elevated `config keygen` is named); it is canonicalized, verified
/// against the admin-only owners and parsed from the verified handle. Only
/// then is a missing state directory created, canonicalized and verified.
/// Last, the presence file is created so a user instance started after this
/// install is visible to the system tier.
fn plan_system_paths(req: &InstallRequest, seams: &InstallSeams<'_>) -> Result<(PathBuf, PathBuf)> {
    let role = req.role;
    let (config, state) = system_paths(req)?;
    if !config.exists() {
        return Err(missing_system_config(role, &config));
    }
    let config = canonical_path(&config)?;
    let text = read_system_config(&config, seams.env)?;
    check_installable_text(role, &text, &config)?;
    create_state_dir(&state, seams.env)?;
    let state = canonical_path(&state)?;
    verify_system_state(&state, seams.env)?;
    (seams.ensure_presence)(role)?;
    Ok((config, state))
}

/// The absolute config and state directory a system install pins before
/// they are checked: the overrides, else the system-tier defaults. Reads no
/// `HOME`, `SUDO_USER` or user profile.
fn system_paths(req: &InstallRequest) -> Result<(PathBuf, PathBuf)> {
    let config = match req.config.as_deref() {
        Some(path) => absolutize_config_path(path)?,
        None => crate::config::resolve_config_path_for(req.role, Tier::System)?,
    };
    let state = absolute_dir(&match non_empty(req.state_dir_override.as_deref()) {
        Some(dir) => dir.to_path_buf(),
        None => crate::paths::default_state_dir_for(Tier::System)?,
    })?;
    Ok((config, state))
}

/// `dir` unless empty.
fn non_empty(dir: Option<&Path>) -> Option<&Path> {
    dir.filter(|d| !d.as_os_str().is_empty())
}

/// `dir` made absolute against the current directory.
fn absolute_dir(dir: &Path) -> Result<PathBuf> {
    std::path::absolute(dir)
        .with_context(|| format!("cannot make state directory {} absolute", dir.display()))
}

/// The configuration error for a system install without its config: names
/// the path and the elevated `config keygen` that creates it.
fn missing_system_config(role: Role, config: &Path) -> anyhow::Error {
    let role = role.name();
    let keygen = if cfg!(windows) {
        format!(
            "`iroh-tunnel {role} config keygen` from an elevated terminal (Run as administrator)"
        )
    } else {
        format!("`sudo iroh-tunnel {role} config keygen`")
    };
    CliError::Config(format!(
        "cannot install the system service: {} does not exist. Create it first with {keygen}, \
         or pass --config",
        config.display()
    ))
    .into()
}

/// The form of an existing `path` a system registration pins. Unix: the
/// canonical path (macOS `/var` becomes `/private/var`), so the no-symlink
/// trust walk accepts root-owned system links. Windows: the absolute path
/// without a verbatim prefix, never resolved, so the walk still refuses a
/// junction or symlink in it.
fn canonical_path(path: &Path) -> Result<PathBuf> {
    #[cfg(unix)]
    return super::trust::canonical(path);
    #[cfg(not(unix))]
    std::path::absolute(path)
        .map(crate::fs_util::strip_verbatim_prefix)
        .with_context(|| format!("cannot make {} absolute", path.display()))
}

/// Create the system state directory when it is missing: every missing
/// component `0755`, through verified handles; an existing one is never
/// re-moded.
#[cfg(unix)]
fn create_state_dir(state: &Path, env: EnvLookup<'_>) -> Result<()> {
    if state.exists() {
        return Ok(());
    }
    // The trust walk needs a canonical path: resolve the deepest existing
    // ancestor (macOS `/var` -> `/private/var`) and append the missing rest.
    let existing = state
        .ancestors()
        .find(|a| !a.as_os_str().is_empty() && a.exists())
        .with_context(|| format!("{} has no existing ancestor", state.display()))?;
    let rest = state
        .strip_prefix(existing)
        .with_context(|| format!("cannot split {}", state.display()))?;
    let target = canonical_path(existing)?.join(rest);
    super::trust::ensure_dir(&target, super::admin_trust(env)?, 0o755)
}

/// Create the system state directory when it is missing: below the machine
/// directory it inherits that directory's protected DACL, which is created
/// first; nothing existing is re-DACLed.
#[cfg(windows)]
fn create_state_dir(state: &Path, env: EnvLookup<'_>) -> Result<()> {
    if state.exists() {
        return Ok(());
    }
    // Only a state directory below the machine directory needs it created
    // (protected DACL); an override elsewhere must not create it.
    if let Some(machine) = crate::ownership::machine_dir()? {
        if state.starts_with(&machine.path) {
            crate::ownership::ensure_machine_dir(&machine)?;
        }
    }
    // The existing ancestors are verified, handles held, before any
    // component is created.
    super::trust_windows::ensure_dir(state, &super::admin_trust(env)?)
}

#[cfg(not(any(unix, windows)))]
fn create_state_dir(_state: &Path, _env: EnvLookup<'_>) -> Result<()> {
    Err(CliError::Service("system services are not supported on this platform".into()).into())
}

/// The system config text, read from a handle verified against the
/// admin-only owners.
#[cfg(unix)]
fn read_system_config(config: &Path, env: EnvLookup<'_>) -> Result<String> {
    super::trust::read_verified_with(config, super::admin_trust(env)?)
}

/// The system state directory must be changeable by the admin-only owners
/// only.
#[cfg(unix)]
fn verify_system_state(state: &Path, env: EnvLookup<'_>) -> Result<()> {
    super::trust::verify_dir_with(state, super::admin_trust(env)?)
}

/// Fail closed before registering anything LocalSystem would later refuse:
/// the machine directory and the config (ownership walk, then the
/// confidentiality of the config directory and file) must be trusted. The
/// config text is read from the verified handle.
#[cfg(windows)]
fn read_system_config(config: &Path, env: EnvLookup<'_>) -> Result<String> {
    use super::trust_windows::{
        read_verified, verify_confidential_with, verify_config_confidential, verify_machine_dir,
        MissingObject,
    };

    if let Some(machine) = crate::ownership::machine_dir()? {
        if machine.verify && machine.path.exists() {
            verify_machine_dir(&machine.path).map_err(|e| machine_dir_refusal(e, &machine.path))?;
        }
    }
    let trusted = super::admin_trust(env)?;
    let text = read_verified(config, &trusted)?;
    // The config was just read, so a missing object is an error, not a pass.
    verify_config_confidential(config, MissingObject::Refuse, &|path| {
        verify_confidential_with(path, &trusted)
    })?;
    Ok(text)
}

/// The system state directory must be changeable by the admin principals
/// only.
#[cfg(windows)]
fn verify_system_state(state: &Path, env: EnvLookup<'_>) -> Result<()> {
    super::trust_windows::verify_dir(state, &super::admin_trust(env)?)
}

#[cfg(not(any(unix, windows)))]
fn read_system_config(_config: &Path, _env: EnvLookup<'_>) -> Result<String> {
    Err(CliError::Service("system services are not supported on this platform".into()).into())
}

#[cfg(not(any(unix, windows)))]
fn verify_system_state(_state: &Path, _env: EnvLookup<'_>) -> Result<()> {
    Err(CliError::Service("system services are not supported on this platform".into()).into())
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
    use super::super::fixtures::{commands, request, run_install, BATCH};
    use super::*;
    use std::path::PathBuf;

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

    #[test]
    fn install_steps_writes_copy_then_note_then_registers_copy() {
        let copy = PathBuf::from("/m/iroh-tunnel/bin/serve/iroh-tunnel");
        let binary = ServiceBinary::Managed {
            source: request().running_executable,
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
                source: request().running_executable.display,
                copy,
                config: PathBuf::from(STEPS_CONFIG),
            }]
        );
    }

    #[test]
    fn install_steps_register_failure_keeps_copy_in_applied() {
        let copy = PathBuf::from("/m/iroh-tunnel/bin/serve/iroh-tunnel");
        let binary = ServiceBinary::Managed {
            source: request().running_executable,
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

    #[test]
    fn verbatim_prefix_is_stripped_but_not_for_unc() {
        let strip = |p: &str| crate::fs_util::strip_verbatim_prefix(PathBuf::from(p));
        assert_eq!(strip(r"\\?\C:\a\b.toml"), PathBuf::from(r"C:\a\b.toml"));
        assert_eq!(strip(r"C:\a\b.toml"), PathBuf::from(r"C:\a\b.toml"));
        assert_eq!(
            strip(r"\\?\UNC\srv\share\b.toml"),
            PathBuf::from(r"\\?\UNC\srv\share\b.toml")
        );
    }

    #[test]
    fn relative_tunnel_executable_is_rejected() {
        let mut req = request();
        req.tunnel_executable = PathBuf::from("iroh-tunnel");
        let (result, log) = run_install(&req, BATCH, vec![]);
        let err = anyhow::Error::from(result.expect_err("must be rejected"));
        assert!(
            matches!(err.downcast_ref::<CliError>(), Some(CliError::Service(_))),
            "{err:?}"
        );
        assert!(commands(&log).is_empty(), "{log:?}");
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
    fn plan_binary_user_tier_is_resolved_path() {
        let req = request();
        let planned = plan_binary(&req, Tier::User, false, &no_verify).unwrap();
        assert_eq!(
            planned,
            ServiceBinary::Direct(req.tunnel_executable.clone())
        );
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
                tier: Tier::System,
                config: PathBuf::from(STEPS_CONFIG),
                state: PathBuf::from("/home/a/state"),
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
}

/// Tier-aware install planning and the binary decision (Linux, macOS). The
/// system tier is exercised without root: an explicit `--config` and state
/// directory under a temp dir, and the test-build trusted owner standing in
/// for root.
#[cfg(test)]
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix_tests {
    use std::cell::RefCell;
    use std::ffi::OsString;
    use std::os::unix::fs::PermissionsExt;

    use super::super::fixtures::{
        commands, in_isolated_home, request, run_install, run_install_with, seams,
        write_valid_config, BATCH, ELEVATED,
    };
    use super::super::native::fake::Event;
    use super::super::service_binary::managed_copy_path;
    #[cfg(all(feature = "test-utils", target_os = "linux"))]
    use super::super::types::ServiceInvocation;
    use super::*;

    fn refuse(path: &Path) -> Result<()> {
        Err(CliError::Permission(format!("{} is not root-only", path.display())).into())
    }

    fn me() -> u32 {
        // SAFETY: geteuid(2) has no preconditions and cannot fail.
        unsafe { libc::geteuid() }
    }

    /// An environment naming this process's uid as the trusted owner.
    fn owner_env(name: &str) -> Option<OsString> {
        (name == "IROH_TUNNEL_TEST_TRUSTED_OWNER").then(|| me().to_string().into())
    }

    /// A request whose resolved binary exists, so it can be canonicalized.
    fn request_with_binary() -> (tempfile::TempDir, InstallRequest) {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("iroh-tunnel");
        std::fs::write(&binary, "bin").unwrap();
        let mut req = request();
        req.tunnel_executable = binary;
        (dir, req)
    }

    /// A canonical temp root (0755) holding a valid `serve.toml` (0600) and
    /// the path of a state directory that does not exist yet.
    fn system_files() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
        let config = write_valid_config(&root);
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o600)).unwrap();
        (tmp, config, root.join("state"))
    }

    /// The plan of a system install with the trusted test owner, the binary
    /// check `verify` and a presence step that records its calls.
    fn plan_system(
        req: &InstallRequest,
        verify: &dyn Fn(&Path) -> Result<()>,
        presence: &RefCell<Vec<Role>>,
    ) -> Result<InstallPlan> {
        let record = |role: Role| {
            presence.borrow_mut().push(role);
            Ok(())
        };
        plan_install(req, ELEVATED, &seams(&owner_env, verify, &record))
    }

    fn kind(err: &anyhow::Error) -> Option<&CliError> {
        err.downcast_ref::<CliError>()
    }

    #[test]
    fn system_install_defaults_to_machine_paths() {
        let (config, state) = system_paths(&request()).unwrap();
        #[cfg(target_os = "linux")]
        {
            assert_eq!(config, Path::new("/etc/iroh-tunnel/serve.toml"));
            assert_eq!(state, Path::new("/run/iroh-tunnel"));
        }
        #[cfg(target_os = "macos")]
        {
            assert_eq!(
                config,
                Path::new("/Library/Application Support/iroh-tunnel/config/serve.toml")
            );
            assert_eq!(
                state,
                Path::new("/Library/Application Support/iroh-tunnel/state")
            );
        }
        let home = crate::paths::default_state_dir_for(Tier::User)
            .ok()
            .and_then(|d| d.ancestors().nth(3).map(Path::to_path_buf));
        if let Some(home) = home {
            assert!(!config.starts_with(&home), "{config:?} under {home:?}");
            assert!(!state.starts_with(&home), "{state:?} under {home:?}");
        }
    }

    #[test]
    fn system_install_creates_missing_state_dir_before_canonicalizing() {
        let (_tmp, config, state) = system_files();
        let (_bin, mut req) = request_with_binary();
        req.config = Some(config.clone());
        // Two missing levels: both are created 0755.
        let nested = state.join("run");
        req.state_dir_override = Some(nested.clone());
        let presence = RefCell::new(Vec::new());
        let plan = plan_system(&req, &|_| Ok(()), &presence)
            .unwrap_or_else(|e| panic!("plan failed: {e:#}"));
        assert_eq!(plan.tier, Tier::System);
        assert_eq!(plan.config, config);
        assert_eq!(plan.state, nested);
        for dir in [&state, &nested] {
            let mode = std::fs::metadata(dir).unwrap().permissions().mode() & 0o7777;
            assert_eq!(mode, 0o755, "{}", dir.display());
        }
    }

    #[test]
    fn system_install_pins_canonical_state_dir() {
        let (tmp, config, real) = system_files();
        std::fs::create_dir(&real).unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o755)).unwrap();
        // A link to the state directory stands in for macOS `/var` ->
        // `/private/var`: the pinned path is the resolved one.
        let link = tmp.path().join("link-state");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let (_bin, mut req) = request_with_binary();
        req.config = Some(config);
        req.state_dir_override = Some(link);
        let presence = RefCell::new(Vec::new());
        let plan = plan_system(&req, &|_| Ok(()), &presence)
            .unwrap_or_else(|e| panic!("plan failed: {e:#}"));
        assert_eq!(plan.state, real);
        #[cfg(target_os = "macos")]
        assert_eq!(
            canonical_path(Path::new("/var")).unwrap(),
            Path::new("/private/var")
        );
    }

    #[test]
    fn system_install_missing_config_names_path_and_keygen() {
        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path().join("serve.toml");
        let (_bin, mut req) = request_with_binary();
        req.config = Some(config.clone());
        let presence = RefCell::new(Vec::new());
        let Err(err) = plan_system(&req, &|_| Ok(()), &presence) else {
            panic!("a missing config must be refused");
        };
        let Some(CliError::Config(msg)) = kind(&err) else {
            panic!("expected Config (exit 2), got {err:?}");
        };
        assert!(msg.contains(&config.display().to_string()), "{msg}");
        assert!(
            msg.contains("sudo iroh-tunnel serve config keygen"),
            "{msg}"
        );
        assert!(presence.borrow().is_empty());
        // The default path is named when no --config is given.
        let default = missing_system_config(Role::Serve, &system_paths(&request()).unwrap().0);
        let expected = crate::paths::default_config_path_for("serve", Tier::System).unwrap();
        assert!(
            default
                .to_string()
                .contains(&expected.display().to_string()),
            "{default}"
        );
    }

    #[test]
    fn system_install_refuses_untrusted_config() {
        let (_tmp, config, state) = system_files();
        // Every directory above is trusted (the test owner); the config file
        // itself is writable by others, so only its own check can refuse.
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o666)).unwrap();
        let (_bin, mut req) = request_with_binary();
        req.config = Some(config.clone());
        req.state_dir_override = Some(state.clone());
        let record = |_: Role| panic!("no presence file for a refused install");
        let Err(err) = plan_install(&req, ELEVATED, &seams(&owner_env, &|_| Ok(()), &record))
        else {
            panic!("a config writable by others must be refused");
        };
        let Some(CliError::Permission(msg)) = kind(&err) else {
            panic!("expected Permission (exit 3), got {err:?}");
        };
        assert!(msg.contains(&config.display().to_string()), "{msg}");
        assert!(msg.contains("0666"), "{msg}");
        assert!(!state.exists(), "nothing is created for a refused config");
    }

    #[test]
    fn system_install_creates_presence_file() {
        let (_tmp, config, state) = system_files();
        let (_bin, mut req) = request_with_binary();
        req.config = Some(config);
        req.state_dir_override = Some(state);
        let presence = RefCell::new(Vec::new());
        plan_system(&req, &|_| Ok(()), &presence).unwrap();
        assert_eq!(*presence.borrow(), [Role::Serve]);
        // The production step is the ownership presence file.
        const NAME: &str = "service::install::unix_tests::system_install_creates_presence_file";
        if std::env::var_os("IROH_TUNNEL_TEST_CHILD").is_some() || cfg!(feature = "test-utils") {
            in_isolated_home(NAME, |home| {
                // SAFETY: single test thread in an isolated child process.
                unsafe { std::env::set_var("IROH_TUNNEL_MACHINE_LOCK_DIR", home.join("machine")) };
                (InstallSeams::production().ensure_presence)(Role::Serve).unwrap();
                assert!(home.join("machine/serve.user.lock").is_file());
            });
        }
    }

    #[test]
    fn install_returns_report_not_need() {
        let (_tmp, config, state) = system_files();
        let (_bin, mut req) = request_with_binary();
        req.config = Some(config);
        req.state_dir_override = Some(state);
        let record = |_: Role| Ok(());
        let (result, log) = run_install_with(
            &req,
            ELEVATED,
            Vec::new(),
            &seams(&owner_env, &|_| Ok(()), &record),
        );
        // Either a report (the backend wrote its registration through the
        // fake runner) or a failure; never a request for more input.
        match result {
            Ok(report) => assert!(!report.applied.is_empty() || !commands(&log).is_empty()),
            Err(failure) => {
                let _: &anyhow::Error = &failure.error;
            }
        }
    }

    #[test]
    fn failed_trust_check_writes_no_managed_copy() {
        let (_tmp, config, state) = system_files();
        if me() == 0 {
            std::os::unix::fs::chown(&config, Some(4242), None).unwrap();
        }
        let (_bin, mut req) = request_with_binary();
        req.config = Some(config);
        req.state_dir_override = Some(state);
        let empty = |_: &str| None;
        let record = |_: Role| Ok(());
        let verify = |_: &Path| refuse(Path::new("/x"));
        let (result, log) =
            run_install_with(&req, ELEVATED, Vec::new(), &seams(&empty, &verify, &record));
        let failure = result.expect_err("an untrusted config must fail the install");
        assert!(
            matches!(kind(&failure.error), Some(CliError::Permission(_))),
            "{:?}",
            failure.error
        );
        assert!(failure.applied.is_empty(), "{:?}", failure.applied);
        assert!(commands(&log).is_empty(), "{log:?}");
        assert!(!managed_copy_path("serve").exists() || cfg!(not(feature = "test-utils")));
    }

    #[test]
    fn failed_binary_decision_creates_no_state_or_presence() {
        let (_tmp, config, state) = system_files();
        let (_bin, mut req) = request_with_binary();
        // An absolute binary that does not exist cannot be canonicalized.
        req.tunnel_executable = state.with_file_name("missing-binary");
        req.config = Some(config);
        req.state_dir_override = Some(state.clone());
        let presence = RefCell::new(Vec::new());
        let record = |role: Role| {
            presence.borrow_mut().push(role);
            Ok(())
        };
        let (result, log) = run_install_with(
            &req,
            ELEVATED,
            Vec::new(),
            &seams(&owner_env, &|_| Ok(()), &record),
        );
        let failure = result.expect_err("an unresolvable binary must fail the install");
        assert!(
            matches!(kind(&failure.error), Some(CliError::Config(_))),
            "{:?}",
            failure.error
        );
        assert!(!state.exists(), "state dir created before the binary check");
        assert!(presence.borrow().is_empty());
        assert!(failure.applied.is_empty(), "{:?}", failure.applied);
        assert!(commands(&log).is_empty(), "{log:?}");
    }

    #[test]
    fn install_proceeds_when_other_tier_runs_with_note() {
        const NAME: &str =
            "service::install::unix_tests::install_proceeds_when_other_tier_runs_with_note";
        in_isolated_home(NAME, |home| {
            #[cfg(target_os = "linux")]
            if !platform::linux_is_systemd() {
                return; // BusyBox: a non-root install is refused for privilege
            }
            let machine = home.join("machine");
            std::fs::create_dir_all(&machine).unwrap();
            // SAFETY: single test thread in an isolated child process.
            unsafe { std::env::set_var("IROH_TUNNEL_MACHINE_LOCK_DIR", &machine) };
            // A system-tier serve holds its detection lock shared.
            let _system = crate::ownership::hold_for_test(
                Role::Serve,
                &machine,
                crate::ownership::LockKind::Shared,
            )
            .unwrap();
            let mut req = request();
            req.config = Some(write_valid_config(home));
            req.state_dir_override = Some(home.join("state"));
            let (result, log) = run_install(&req, BATCH, Vec::new());
            let report = result.unwrap_or_else(|f| panic!("install refused: {:#}", f.error));
            assert!(
                matches!(
                    report.applied.first(),
                    Some(AppliedEffect::RegistrationWritten(_))
                ),
                "{:?}",
                report.applied
            );
            let note = Event::Note(ServiceNote::OtherTierRunning {
                role: "serve".into(),
                other: Tier::System,
            });
            let at = log
                .iter()
                .position(|e| *e == note)
                .expect("other-tier note");
            let first_call = log.iter().position(|e| matches!(e, Event::Call { .. }));
            assert!(
                first_call.is_none_or(|c| at < c),
                "note precedes the manager: {log:?}"
            );
        });
    }

    #[cfg(feature = "test-utils")]
    #[test]
    fn user_install_default_config_ignores_the_ambient_tier() {
        const NAME: &str =
            "service::install::unix_tests::user_install_default_config_ignores_the_ambient_tier";
        in_isolated_home(NAME, |home| {
            // SAFETY: single test thread in an isolated child process.
            unsafe { std::env::set_var("IROH_TUNNEL_TEST_TIER", "system") };
            let user_config =
                crate::config::resolve_config_path_for(Role::Serve, Tier::User).unwrap();
            assert!(user_config.starts_with(home), "{user_config:?}");
            std::fs::create_dir_all(user_config.parent().unwrap()).unwrap();
            std::fs::copy(write_valid_config(home), &user_config).unwrap();
            let mut req = request();
            req.state_dir_override = Some(home.join("state"));
            let (config, _) = plan_user_paths(&req).unwrap();
            assert_eq!(config, user_config);
        });
    }

    #[test]
    fn a_hold_at_the_registering_tier_is_permitted() {
        const NAME: &str =
            "service::install::unix_tests::a_hold_at_the_registering_tier_is_permitted";
        in_isolated_home(NAME, |home| {
            #[cfg(target_os = "linux")]
            if !platform::linux_is_systemd() {
                return;
            }
            // SAFETY: single test thread in an isolated child process.
            unsafe { std::env::set_var("IROH_TUNNEL_MACHINE_LOCK_DIR", home.join("machine")) };
            let _own = crate::ownership::claim(Role::Serve).unwrap();
            let mut req = request();
            req.config = Some(write_valid_config(home));
            req.state_dir_override = Some(home.join("state"));
            let (result, _) = run_install(&req, BATCH, Vec::new());
            result.unwrap_or_else(|f| panic!("re-install refused: {:#}", f.error));
        });
    }

    #[test]
    fn install_renders_supplied_tunnel_executable() {
        const NAME: &str =
            "service::install::unix_tests::install_renders_supplied_tunnel_executable";
        in_isolated_home(NAME, |home| {
            #[cfg(target_os = "linux")]
            if !platform::linux_is_systemd() {
                return; // the BusyBox backend needs real root
            }
            let mut req = request();
            req.config = Some(write_valid_config(home));
            req.state_dir_override = Some(home.join("state"));
            let (result, _log) = run_install(&req, BATCH, vec![]);
            let report = result.unwrap_or_else(|f| panic!("install failed: {:#}", f.error));
            let Some(AppliedEffect::RegistrationWritten(path)) = report.applied.first() else {
                panic!("no registration written: {:?}", report.applied);
            };
            let body = std::fs::read_to_string(path).unwrap();
            assert!(body.contains("/opt/x/iroh-tunnel"), "{body}");
        });
    }

    #[test]
    fn partial_install_reports_written_registration() {
        const NAME: &str =
            "service::install::unix_tests::partial_install_reports_written_registration";
        in_isolated_home(NAME, |home| {
            #[cfg(target_os = "linux")]
            if !platform::linux_is_systemd() {
                return;
            }
            let mut req = request();
            req.config = Some(write_valid_config(home));
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

    #[test]
    fn plan_binary_system_tier_managed_when_refused() {
        let (_dir, req) = request_with_binary();
        let planned = plan_binary(&req, Tier::System, false, &refuse).unwrap();
        assert_eq!(
            planned,
            ServiceBinary::Managed {
                source: req.running_executable.clone(),
                copy: managed_copy_path("serve"),
            }
        );

        let canonical = req.tunnel_executable.canonicalize().unwrap();
        let planned = plan_binary(&req, Tier::System, false, &|path| {
            assert_eq!(path, canonical, "the canonical path is checked");
            Ok(())
        })
        .unwrap();
        assert_eq!(planned, ServiceBinary::Direct(canonical));
    }

    #[test]
    fn unprivileged_root_run_install_is_refused_before_any_decision() {
        if !platform::system_wide_only() {
            return; // only BusyBox makes a user-tier registration root-run
        }
        let (_dir, req) = request_with_binary();
        let empty = |_: &str| None;
        let record = |_: Role| panic!("no presence before the privilege refusal");
        let Err(err) = plan_install(
            &req,
            BATCH,
            &seams(&empty, &|_| panic!("no binary decision"), &record),
        ) else {
            panic!("must be refused");
        };
        let Some(CliError::Permission(msg)) = kind(&err) else {
            panic!("expected Permission, got {err:?}");
        };
        assert!(msg.contains("Re-run with sudo"), "{msg}");
        assert!(msg.contains("Nothing was changed"), "{msg}");
    }

    #[test]
    fn plan_binary_busybox_user_tier_is_root_run() {
        let (_dir, req) = request_with_binary();
        let planned = plan_binary(&req, Tier::User, true, &refuse).unwrap();
        assert_eq!(
            planned,
            ServiceBinary::Managed {
                source: req.running_executable.clone(),
                copy: managed_copy_path("serve"),
            }
        );
        let planned = plan_binary(&req, Tier::User, true, &|_| Ok(())).unwrap();
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
        const NAME: &str = "service::install::unix_tests::managed_standalone_selection_materializes_and_registers_both_roles";
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
                let mut req = request();
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
                assert!(verify_service_binary(&source).is_err());
                let chosen =
                    plan_binary(&req, Tier::System, false, &verify_service_binary).unwrap();
                assert!(matches!(chosen, ServiceBinary::Managed { .. }));
                let events = Events::default();
                let mut fake = FakeRunner::new(&events, vec![]);
                let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
                let mut native = Native::new(OutputMode::Capture, &mut runner);
                let config = home.join("config");
                let plan = InstallPlan {
                    tier: Tier::System,
                    config: config.clone(),
                    state: home.join("state"),
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
                            TrustedOwners::TestOwner(unsafe { libc::geteuid() }),
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
}

/// Windows plan against the real token; gated like the other service tests.
#[cfg(test)]
#[cfg(windows)]
mod windows_plan_tests {
    use super::super::fixtures::request;
    use super::super::trust_windows::token_user;
    use super::*;

    /// The tier follows privilege: a non-elevated caller plans a user task,
    /// never a Windows Service, and no machine-wide file is consulted.
    #[test]
    fn non_elevated_install_plans_the_user_tier() {
        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path().join("serve.toml");
        std::fs::write(
            &config,
            "[node]\nrelay_urls = []\n\n[[services]]\nname = \"echo\"\nprotocol = \"tcp\"\nport = 9\n",
        )
        .unwrap();
        let mut req = request();
        req.config = Some(config);
        let empty = |_: &str| None;
        let verify = |_: &Path| panic!("a user task's binary is not machine-checked");
        let presence = |_: Role| panic!("a user install creates no presence file");
        let plan = plan_install(
            &req,
            super::super::fixtures::BATCH,
            &super::super::fixtures::seams(&empty, &verify, &presence),
        )
        .unwrap();
        assert_eq!(plan.tier, Tier::User);
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
        let err = super::super::trust_windows::verify_config_confidential(
            config,
            super::super::trust_windows::MissingObject::Refuse,
            &check,
        )
        .unwrap_err();
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
    fn plan_for(root: &Path, config: PathBuf) -> Result<InstallPlan> {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let owner = std::ffi::OsString::from(token_user().unwrap().0);
        let env = |name: &str| (name == OWNER_VAR).then(|| owner.clone());
        let mut req = request();
        req.config = Some(config);
        req.state_dir_override = Some(root.join("state"));
        let system_root = std::env::var_os("SystemRoot").expect("SystemRoot is set");
        req.tunnel_executable = PathBuf::from(system_root).join("System32\\cmd.exe");
        let presence = |_: Role| Ok(());
        plan_install(
            &req,
            super::super::fixtures::ELEVATED,
            &super::super::fixtures::seams(&env, &verify_service_binary, &presence),
        )
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
        let Err(err) = planned else {
            panic!("a readable config must be refused");
        };
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
