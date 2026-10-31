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
//! `tests/service_user_windows.rs` and recorded in
//! `docs/design/issue-92-evidence.md`.

use std::path::Path;
use std::process::{Command, Output};

use anyhow::{Context, Result};

use super::task_xml::{encode_utf16_le, format_task_xml, task_action};
use super::{instance_suffix, resolve_binary, RestartPolicy, ServiceScope};
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
fn current_user() -> Result<String> {
    let output = Command::new("whoami")
        .output()
        .context("failed to run whoami")?;
    let name = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !output.status.success() || name.is_empty() {
        return Err(CliError::Service("whoami did not report the current user".into()).into());
    }
    Ok(name)
}

/// Run `schtasks` with `args`, returning its output whatever the status.
fn schtasks(args: &[&str]) -> Result<Output> {
    Command::new("schtasks")
        .args(args)
        .output()
        .with_context(|| format!("failed to run schtasks {}", args.join(" ")))
}

/// Run `schtasks`, turning a failure into a service error carrying its text.
fn schtasks_ok(args: &[&str]) -> Result<String> {
    let output = schtasks(args)?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if output.status.success() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(CliError::Service(format!(
        "schtasks {} failed with {}: {}{}",
        args.join(" "),
        output.status,
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
fn is_installed(role: &str) -> Result<bool> {
    let listing = schtasks_ok(&["/Query", "/FO", "CSV", "/NH"])?;
    let wanted = format!("\\{}\"", task_name(role));
    Ok(listing.lines().any(|line| line.contains(&wanted)))
}

fn require_installed(role: &str) -> Result<()> {
    if is_installed(role)? {
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

pub fn install(role: &str, scope: ServiceScope, config: &Path, state: &Path) -> Result<()> {
    debug_assert_eq!(scope, ServiceScope::User, "system scope is refused earlier");
    let binary = resolve_binary()?;
    ensure_default_state(state)?;
    let action = task_action(role, &binary, config)?;
    let xml = format_task_xml(role, &current_user()?, &action, &RestartPolicy::DEFAULT);

    let definition = TempDefinition::write(&encode_utf16_le(&xml))?;

    let name = task_name(role);
    let path = definition.0.to_string_lossy().into_owned();
    // /F replaces an existing task, so re-install is idempotent.
    schtasks_ok(&["/Create", "/TN", &name, "/XML", &path, "/F"])
        .map_err(|e| denied_or(e, "registering the scheduled task"))?;
    schtasks_ok(&["/Run", "/TN", &name]).context("task registered but could not be started")?;
    println!("Installed and started task {name} (runs in your logged-in session)");
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

pub fn uninstall(role: &str, _scope: ServiceScope) -> Result<()> {
    let name = task_name(role);
    if is_installed(role)? {
        // A running instance would keep going after its task is deleted. A
        // failed /End only means nothing was running; /Delete reports the rest.
        let _ = schtasks(&["/End", "/TN", &name]);
        schtasks_ok(&["/Delete", "/TN", &name, "/F"])?;
    }
    println!("Uninstalled {name}");
    Ok(())
}

pub fn start(role: &str, _scope: ServiceScope) -> Result<()> {
    require_installed(role)?;
    schtasks_ok(&["/Run", "/TN", &task_name(role)])?;
    Ok(())
}

pub fn stop(role: &str, _scope: ServiceScope) -> Result<()> {
    require_installed(role)?;
    let name = task_name(role);
    let output = schtasks(&["/End", "/TN", &name])?;
    if output.status.success() {
        return Ok(());
    }
    // `/End` on a task with no running instance fails; like the other
    // backends, stopping something already stopped is not an error.
    if task_is_running(role)? {
        return Err(CliError::Service(format!(
            "schtasks /End /TN {name} failed with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
        .into());
    }
    println!("{name} is already stopped.");
    Ok(())
}

/// How long `restart` waits for the old process to release the role.
const RESTART_WAIT: std::time::Duration = std::time::Duration::from_secs(15);

pub fn restart(role: &str, scope: ServiceScope) -> Result<()> {
    stop(role, scope)?;
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
    start(role, scope)
}

pub fn status(role: &str, _scope: ServiceScope) -> Result<()> {
    require_installed(role)?;
    // The role lock, not the localised task text, says whether it runs.
    match crate::ownership::probe(status_role(role)?)? {
        crate::ownership::Liveness::Running { owner, .. } => println!(
            "{role} is running ({})",
            crate::ownership::describe_owner(owner.as_ref())
        ),
        crate::ownership::Liveness::NotRunning => println!("{role} is not running"),
    }
    let out = schtasks_ok(&["/Query", "/TN", &task_name(role), "/FO", "LIST", "/V"])?;
    print!("{out}");
    Ok(())
}

fn status_role(role: &str) -> Result<crate::status::StatusRole> {
    match role {
        "serve" => Ok(crate::status::StatusRole::Serve),
        "access" => Ok(crate::status::StatusRole::Access),
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
