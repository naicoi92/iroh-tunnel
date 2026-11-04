//! Windows user-background backend (SM-04, issue #92).
//!
//! A Task Scheduler task for the current user: logon trigger, interactive
//! token, no elevation. It is driven through `schtasks` (`/Create /XML`,
//! `/Run`, `/End`, `/Query`, `/Delete`), so install, start, status and stop
//! need no administrator rights. There is no Windows system service: system
//! scope is refused by `dispatch_service` (via `scope::ensure_supported`)
//! before reaching this module.
//!
//! Per RO-03 stop is `schtasks /End`; no control channel or stop command is
//! added. Whether `/End` lets the tunnel drain is measured on real Windows in
//! `tests/service_user_windows.rs`.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

use super::super::native::{InstalledAt, Native, NativeOutput, ServiceNote};
use super::super::registration::instance_suffix;
use super::super::task_xml::{encode_utf16_le, format_task_xml, task_action};
use super::super::types::AppliedEffect;
use super::super::types::{RestartPolicy, ServiceScope};
use crate::error::CliError;

/// Task name, one per role (plus the test-only instance suffix).
fn task_name(role: &str) -> String {
    format!("iroh-tunnel-{role}{}", instance_suffix())
}

/// `DOMAIN\name` of the account the process runs as, asked of the OS.
///
/// `USERNAME`/`USERDOMAIN` are inherited environment and can name another
/// account than the token (for example after a credentialed launch), which
/// would bind the logon trigger to the wrong user.
fn current_user(native: &mut Native) -> Result<String> {
    let output = native
        .output(&mut Command::new("whoami"))
        .context("failed to run whoami")?;
    let name = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !output.success() || name.is_empty() {
        return Err(CliError::Service("whoami did not report the current user".into()).into());
    }
    Ok(name)
}

/// Run `schtasks` with `args`, returning its output whatever the status.
/// Always captured: its text is parsed or folded into errors.
fn schtasks(native: &mut Native, args: &[&str]) -> Result<NativeOutput> {
    native
        .output(Command::new("schtasks").args(args))
        .with_context(|| format!("failed to run schtasks {}", args.join(" ")))
}

/// Run `schtasks`, turning a failure into a service error carrying its text.
fn schtasks_ok(native: &mut Native, args: &[&str]) -> Result<String> {
    let output = schtasks(native, args)?;
    ensure_schtasks_ok(&output, args)
}

/// The stdout of a successful `schtasks` run, or a service error carrying its text.
fn ensure_schtasks_ok(output: &NativeOutput, args: &[&str]) -> Result<String> {
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if output.success() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(CliError::Service(format!(
        "schtasks {} failed with {}: {}{}",
        args.join(" "),
        output.status_text(),
        stderr.trim(),
        stdout.trim()
    ))
    .into())
}

fn not_installed_hint(role: &str) -> String {
    format!(
        "no task '{}' is registered for this user. Install it first with \
         `iroh-tunnel {role} service install`",
        task_name(role)
    )
}

/// Whether the role's task is registered for this user.
///
/// Lists every task and looks for the name: a failed listing (access denied,
/// Task Scheduler down) is an error, never "not installed", and the answer
/// does not depend on the localised "not found" text of `/Query /TN`.
fn is_installed(native: &mut Native, role: &str) -> Result<bool> {
    let listing = schtasks_ok(native, &["/Query", "/FO", "CSV", "/NH"])?;
    let wanted = format!("\\{}\"", task_name(role));
    Ok(listing.lines().any(|line| line.contains(&wanted)))
}

fn require_installed(native: &mut Native, role: &str) -> Result<()> {
    if is_installed(native, role)? {
        Ok(())
    } else {
        Err(CliError::Service(not_installed_hint(role)).into())
    }
}

/// Refuse a state directory other than the user's default.
///
/// A scheduled task cannot carry an environment variable, and wrapping the
/// binary in a shell to set one would make `schtasks /End` kill only the
/// wrapper. Refusing keeps stop reliable and the pinned paths honest.
fn ensure_default_state(state: &Path) -> Result<()> {
    let default = crate::paths::default_state_dir()?;
    // Windows paths are case-insensitive and accept either separator.
    let normal = |p: &Path| p.to_string_lossy().replace('/', "\\").to_lowercase();
    if normal(state) == normal(&default) {
        return Ok(());
    }
    Err(CliError::Config(format!(
        "the user-background task always uses the default state directory {}, but \
         IROH_TUNNEL_STATE_DIR selects {}; unset IROH_TUNNEL_STATE_DIR and re-run",
        default.display(),
        state.display()
    ))
    .into())
}

pub(crate) fn install(
    native: &mut Native,
    notes: &mut dyn FnMut(&ServiceNote),
    role: &str,
    scope: ServiceScope,
    binary: &Path,
    config: &Path,
    state: &Path,
) -> Result<()> {
    debug_assert_eq!(scope, ServiceScope::User, "system scope is refused earlier");
    ensure_default_state(state)?;
    let action = task_action(role, binary, config)?;
    let xml = format_task_xml(
        role,
        &current_user(native)?,
        &action,
        &RestartPolicy::DEFAULT,
    );

    let definition = TempDefinition::write(&encode_utf16_le(&xml))?;

    let name = task_name(role);
    let path = definition.0.to_string_lossy().into_owned();
    // /F replaces an existing task, so re-install is idempotent.
    schtasks_ok(native, &["/Create", "/TN", &name, "/XML", &path, "/F"])
        .map_err(|e| denied_or(e, "registering the scheduled task"))?;
    native.applied(AppliedEffect::Enabled);
    schtasks_ok(native, &["/Run", "/TN", &name])
        .context("task registered but could not be started")?;
    native.applied(AppliedEffect::Started);
    notes(&ServiceNote::Installed {
        name,
        at: InstalledAt::WindowsTask,
    });
    Ok(())
}

/// A task definition on disk, deleted when dropped. Created with `create_new`
/// so a pre-planted file or link at the path is never followed (RS-11).
struct TempDefinition(std::path::PathBuf);

impl TempDefinition {
    fn write(bytes: &[u8]) -> Result<Self> {
        use std::io::Write;
        let path = std::env::temp_dir().join(format!(
            "iroh-tunnel-task-{}-{}.xml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .with_context(|| format!("failed to create {}", path.display()))?;
        let definition = Self(path);
        file.write_all(bytes)
            .context("failed to write the task definition")?;
        Ok(definition)
    }
}

impl Drop for TempDefinition {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Add scope guidance to an access-denied failure (the account is never
/// changed to work around it). Detection reads the English message: on a
/// localised Windows the failure stays a plain service error (exit 5).
fn denied_or(error: anyhow::Error, what: &str) -> anyhow::Error {
    if error
        .to_string()
        .to_lowercase()
        .contains("access is denied")
    {
        CliError::Permission(format!(
            "access denied {what}. The service scope and account were not changed and \
             nothing was registered; fix the permissions or policy of your account and re-run"
        ))
        .into()
    } else {
        error
    }
}

pub(crate) fn uninstall(
    native: &mut Native,
    notes: &mut dyn FnMut(&ServiceNote),
    role: &str,
    _scope: ServiceScope,
) -> Result<()> {
    let name = task_name(role);
    if is_installed(native, role)? {
        // A running instance would keep going after its task is deleted. A
        // failed /End only means nothing was running; /Delete reports the rest.
        if schtasks(native, &["/End", "/TN", &name]).is_ok_and(|o| o.success()) {
            native.applied(AppliedEffect::Stopped);
        }
        schtasks_ok(native, &["/Delete", "/TN", &name, "/F"])?;
        native.applied(AppliedEffect::Disabled);
    }
    notes(&ServiceNote::Uninstalled { name });
    Ok(())
}

pub(crate) fn start(native: &mut Native, role: &str, _scope: ServiceScope) -> Result<()> {
    require_installed(native, role)?;
    schtasks_ok(native, &["/Run", "/TN", &task_name(role)])?;
    native.applied(AppliedEffect::Started);
    Ok(())
}

pub(crate) fn stop(
    native: &mut Native,
    notes: &mut dyn FnMut(&ServiceNote),
    role: &str,
    _scope: ServiceScope,
) -> Result<()> {
    require_installed(native, role)?;
    let name = task_name(role);
    let output = schtasks(native, &["/End", "/TN", &name])?;
    if output.success() {
        native.applied(AppliedEffect::Stopped);
        return Ok(());
    }
    // `/End` on a task with no running instance fails; like the other
    // backends, stopping something already stopped is not an error.
    if task_is_running(role)? {
        return Err(CliError::Service(format!(
            "schtasks /End /TN {name} failed with {}: {}",
            output.status_text(),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
        .into());
    }
    notes(&ServiceNote::AlreadyStopped { name });
    Ok(())
}

/// How long `restart` waits for the old process to release the role.
const RESTART_WAIT: std::time::Duration = std::time::Duration::from_secs(15);

pub(crate) fn restart(
    native: &mut Native,
    notes: &mut dyn FnMut(&ServiceNote),
    role: &str,
    scope: ServiceScope,
) -> Result<()> {
    stop(native, notes, role, scope)?;
    // `/End` returns before the process is gone; a start that races the old
    // process would be dropped (IgnoreNew) or refused by role ownership.
    let deadline = std::time::Instant::now() + RESTART_WAIT;
    while task_is_running(role)? {
        if std::time::Instant::now() >= deadline {
            return Err(CliError::Service(format!(
                "{} did not stop within {}s after schtasks /End; not starting a second instance",
                task_name(role),
                RESTART_WAIT.as_secs()
            ))
            .into());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    start(native, role, scope)
}

pub(crate) fn status(
    native: &mut Native,
    notes: &mut dyn FnMut(&ServiceNote),
    role: &str,
    _scope: ServiceScope,
) -> Result<NativeOutput> {
    require_installed(native, role)?;
    // The role lock, not the localised task text, says whether it runs.
    match crate::ownership::probe(status_role(role)?)? {
        crate::ownership::Liveness::Running { owner, .. } => notes(&ServiceNote::RoleRunning {
            role: role.to_owned(),
            owner: crate::ownership::describe_owner(owner.as_ref()),
        }),
        crate::ownership::Liveness::NotRunning => notes(&ServiceNote::RoleNotRunning {
            role: role.to_owned(),
        }),
    }
    let name = task_name(role);
    let query = ["/Query", "/TN", &name, "/FO", "LIST", "/V"];
    let output = schtasks(native, &query)?;
    let text = ensure_schtasks_ok(&output, &query)?;
    notes(&ServiceNote::TaskListing { text });
    Ok(output)
}

fn status_role(role: &str) -> Result<crate::role::Role> {
    match role {
        "serve" => Ok(crate::role::Role::Serve),
        "access" => Ok(crate::role::Role::Access),
        other => Err(CliError::Config(format!("unknown role {other:?}")).into()),
    }
}

/// Whether the role is running, by its ownership lock. The task `Status`
/// text is localised, so the lock is the locale-independent source of truth.
fn task_is_running(role: &str) -> Result<bool> {
    Ok(matches!(
        crate::ownership::probe(status_role(role)?)?,
        crate::ownership::Liveness::Running { .. }
    ))
}
