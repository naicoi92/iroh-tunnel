//! BusyBox / SysV init (non-systemd Linux) service backend.
//!
//! Active when the host does *not* run systemd — e.g. the Sipeed NanoKVM and
//! other buildroot/BusyBox embedded devices. Their init (`/sbin/init` →
//! busybox) runs every executable `/etc/init.d/S??*` script in numeric order
//! with a `start` argument during boot (driven by `/etc/init.d/rcS`), so
//! installing an `S96iroh-tunnel-<role>` script here is all that is needed for
//! boot-time autostart.
//!
//! The script manages the daemon with a plain background launch + PID file — no
//! `start-stop-daemon` dependency, so it works on the leanest BusyBox config.
//! iroh-tunnel traps SIGTERM for a graceful drain, which `stop` honours before
//! falling back to SIGKILL after a 5 s grace window.
//!
//! Boot order `S96` deliberately runs *after* networking and the host stack:
//! on the NanoKVM, `S40network` and `S95nanokvm` precede it, so the tunnelled
//! targets exist before iroh-tunnel binds/dials.
//!
//! Scope: BusyBox init is system-wide only (no per-user domain exists), so
//! both scopes manage the one `/etc/init.d` script — User scope prints a
//! note rather than failing. Requires root.

use super::super::effective_binary::{parse_busybox_script, EffectiveBinary};
use super::super::native::{InstalledAt, Native, NativeOutput, ServiceNote};
use super::super::pin::Pin;
use super::super::registration::write_registration;
use super::super::types::AppliedEffect;
use super::super::types::ServiceInvocation;
use super::super::types::ServiceScope;
use super::busybox_script::{InitScript, ScriptLayout};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// Boot-order prefix. BusyBox init runs `/etc/init.d/S*` sorted numerically, so
/// `S96` starts iroh-tunnel after the host stack is up (see module docs).
const START_ORDER: u8 = 96;

/// `/etc/init.d/S96iroh-tunnel-<role>` — the name busybox init auto-starts.
fn script_name(role: &str) -> String {
    format!("S{START_ORDER}iroh-tunnel-{role}")
}

/// Where the init script lives. BusyBox init scans `/etc/init.d`, so this is
/// fixed — there is no scope/per-user variant on a non-systemd host.
fn script_path(role: &str) -> PathBuf {
    PathBuf::from("/etc/init.d").join(script_name(role))
}

/// Mark `path` executable (0755). BusyBox init's rcS only runs scripts that
/// pass its `-x` test, so the bit is mandatory for boot autostart.
#[cfg(unix)]
fn set_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)
        .with_context(|| format!("failed to stat {}", path.display()))?
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms)
        .with_context(|| format!("failed to chmod {}", path.display()))?;
    Ok(())
}

/// Refuse unless running as root. The init script lives under `/etc/init.d`
/// and writes a PID file under `/var/run`, so root is required. BusyBox has no
/// per-user services, so a user-scope request is refused too: it is never
/// silently turned into a system install.
///
/// # Errors
///
/// [`crate::error::CliError::Permission`] with rerun guidance.
fn require_root() -> Result<()> {
    super::super::policy::ensure_root_run_privilege(
        true,
        super::super::is_elevated::is_elevated(),
    )?;
    Ok(())
}

/// The note a user-scope request earns on BusyBox: there is only the
/// system-wide script.
fn busybox_scope_note(role: &str, scope: ServiceScope) -> Option<ServiceNote> {
    (scope == ServiceScope::User).then(|| ServiceNote::BusyboxSystemWide {
        script: script_path(role),
    })
}

/// Run the installed init script with `verb` (start/stop/restart), bailing on a
/// non-zero exit. The script's shebang + executable bit let us invoke it
/// directly (same path busybox init uses on boot).
fn run_script(native: &mut Native, role: &str, verb: &str) -> Result<()> {
    let path = script_path(role);
    let output = native
        .run(std::process::Command::new(&path).arg(verb))
        .with_context(|| format!("failed to run {} {}", path.display(), verb))?;
    if !output.success() {
        bail!(
            "{} {} failed with {}",
            path.display(),
            verb,
            output.status_text()
        );
    }
    Ok(())
}

/// Where the init script is (or would be).
pub(crate) fn registration_path(role: &str) -> PathBuf {
    script_path(role)
}

/// What the init script will run, read from the installed script.
pub(crate) fn effective_binary(role: &str) -> EffectiveBinary {
    parse_busybox_script(existing_registration(role).as_deref())
}

/// The installed script's text, if any (to read its pin).
pub(crate) fn existing_registration(role: &str) -> Option<String> {
    super::super::registration::read_registration(&script_path(role))
}

#[allow(clippy::too_many_arguments)] // one registration's inputs
pub(crate) fn install(
    native: &mut Native,
    notes: &mut dyn FnMut(&ServiceNote),
    role: &str,
    invocation: ServiceInvocation,
    scope: ServiceScope,
    binary: &Path,
    config: &Path,
    state: &Path,
    pin: Option<&Pin>,
) -> Result<()> {
    super::super::pin::require_unix_pin(pin)?;
    require_root()?;
    if let Some(note) = busybox_scope_note(role, scope) {
        notes(&note);
    }
    let script = InitScript {
        role,
        invocation,
        binary,
        config,
        state,
        pin,
    }
    .render(&ScriptLayout::for_role(role))?;
    let path = script_path(role);
    write_registration(&path, &script)?;
    native.applied(AppliedEffect::RegistrationWritten(path.clone()));
    set_executable(&path)?;
    native.applied(AppliedEffect::Enabled);
    // Start now. A start failure is surfaced, but the script is installed, so
    // the service will come up on the next boot regardless.
    // `restart`, not `start`: a re-install over a running service must pick up
    // the new registration instead of leaving the old process on the old pin.
    run_script(native, role, "restart")?;
    native.applied(AppliedEffect::Started);
    notes(&ServiceNote::Installed {
        name: script_name(role),
        at: InstalledAt::Path(path),
    });
    Ok(())
}

pub(crate) fn uninstall(
    native: &mut Native,
    notes: &mut dyn FnMut(&ServiceNote),
    role: &str,
    _scope: ServiceScope,
) -> Result<()> {
    require_root()?;
    // best-effort: the service may not be running
    if run_script(native, role, "stop").is_ok() {
        native.applied(AppliedEffect::Stopped);
    }
    let path = script_path(role);
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("failed to remove {}", path.display()))?;
        native.applied(AppliedEffect::RegistrationRemoved(path));
    }
    notes(&ServiceNote::Uninstalled {
        name: script_name(role),
    });
    Ok(())
}

pub(crate) fn start(native: &mut Native, role: &str, _scope: ServiceScope) -> Result<()> {
    require_root()?;
    run_script(native, role, "start")?;
    native.applied(AppliedEffect::Started);
    Ok(())
}

pub(crate) fn stop(
    native: &mut Native,
    _notes: &mut dyn FnMut(&ServiceNote),
    role: &str,
    _scope: ServiceScope,
) -> Result<()> {
    require_root()?;
    run_script(native, role, "stop")?;
    native.applied(AppliedEffect::Stopped);
    Ok(())
}

pub(crate) fn restart(
    native: &mut Native,
    _notes: &mut dyn FnMut(&ServiceNote),
    role: &str,
    _scope: ServiceScope,
) -> Result<()> {
    require_root()?;
    run_script(native, role, "restart")?;
    native.applied(AppliedEffect::Started);
    Ok(())
}

pub(crate) fn status(
    native: &mut Native,
    _notes: &mut dyn FnMut(&ServiceNote),
    role: &str,
    _scope: ServiceScope,
) -> Result<NativeOutput> {
    let path = script_path(role);
    // The script returns 3 when not running — informative, not a hard error
    // (mirrors how the systemd backend treats `systemctl status` of a stopped
    // unit), so the status is not checked.
    native
        .run(std::process::Command::new(&path).arg("status"))
        .context("failed to run init script status")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busybox_user_scope_request_emits_system_wide_note() {
        assert_eq!(
            busybox_scope_note("serve", ServiceScope::User),
            Some(ServiceNote::BusyboxSystemWide {
                script: script_path("serve")
            })
        );
        assert_eq!(busybox_scope_note("serve", ServiceScope::System), None);
    }

    #[test]
    fn script_name_has_boot_order_prefix() {
        assert_eq!(script_name("serve"), "S96iroh-tunnel-serve");
        assert_eq!(script_name("access"), "S96iroh-tunnel-access");
    }

    #[test]
    fn script_path_lives_under_init_d() {
        assert_eq!(
            script_path("serve"),
            PathBuf::from("/etc/init.d/S96iroh-tunnel-serve")
        );
    }
}
