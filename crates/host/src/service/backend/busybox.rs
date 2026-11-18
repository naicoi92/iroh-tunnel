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
//! Tier: BusyBox init is system-wide only (no per-user domain exists), so
//! the only registration is the root-owned `/etc/init.d` script. Every
//! mutating operation requires root; a non-root caller is refused.

use super::super::effective_binary::{parse_busybox_script, EffectiveBinary};
use super::super::native::{InstalledAt, Native, NativeOutput, ServiceNote};
use super::super::registration::{write_registration, RegistrationInput};
use super::super::types::AppliedEffect;
use super::busybox_script::{render_script, ScriptLayout};
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
/// fixed — there is no tier/per-user variant on a non-systemd host.
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
/// per-user services, so a non-root caller is refused rather than silently
/// given a system install.
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

/// The installed script's text, if any.
pub(crate) fn existing_registration(role: &str) -> Option<String> {
    super::super::registration::read_registration(&script_path(role))
}

pub(crate) fn install(
    native: &mut Native,
    notes: &mut dyn FnMut(&ServiceNote),
    reg: &RegistrationInput<'_>,
) -> Result<()> {
    require_root()?;
    let role = reg.role;
    let script = render_script(reg, &ScriptLayout::for_role(role))?;
    let path = script_path(role);
    write_registration(&path, &script)?;
    native.applied(AppliedEffect::RegistrationWritten(path.clone()));
    set_executable(&path)?;
    native.applied(AppliedEffect::Enabled);
    // Start now. A start failure is surfaced, but the script is installed, so
    // the service will come up on the next boot regardless.
    // `restart`, not `start`: a re-install over a running service must pick up
    // the new registration instead of leaving the old process on the old one.
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
    _tier: crate::Tier,
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

pub(crate) fn start(native: &mut Native, role: &str, _tier: crate::Tier) -> Result<()> {
    require_root()?;
    run_script(native, role, "start")?;
    native.applied(AppliedEffect::Started);
    Ok(())
}

pub(crate) fn stop(
    native: &mut Native,
    _notes: &mut dyn FnMut(&ServiceNote),
    role: &str,
    _tier: crate::Tier,
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
    _tier: crate::Tier,
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
    _tier: crate::Tier,
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

    use crate::service::native::fake::{note_sink, Event, Events, FakeRunner};
    use crate::service::native::OutputMode;

    fn reg<'a>() -> RegistrationInput<'a> {
        RegistrationInput {
            tier: crate::Tier::System,
            ..crate::service::fixtures::registration(
                "serve",
                Path::new("/usr/bin/iroh-tunnel"),
                Path::new("/etc/iroh-tunnel/serve.toml"),
                Path::new("/run/iroh-tunnel"),
            )
        }
    }

    #[test]
    fn busybox_non_root_mutation_is_permission_error() {
        if crate::service::is_elevated() {
            return; // root passes the check and would touch /etc/init.d
        }
        let events = Events::default();
        let mut fake = FakeRunner::new(&events, vec![]);
        let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
        let mut native = Native::new(OutputMode::Capture, &mut runner);
        let mut notes = note_sink(&events);
        let tier = crate::Tier::User;
        let results = [
            install(&mut native, &mut notes, &reg()),
            uninstall(&mut native, &mut notes, "serve", tier),
            start(&mut native, "serve", tier),
            stop(&mut native, &mut notes, "serve", tier),
            restart(&mut native, &mut notes, "serve", tier),
        ];
        for result in results {
            let err = result.expect_err("a non-root BusyBox mutation is refused");
            let Some(crate::error::CliError::Permission(msg)) =
                err.downcast_ref::<crate::error::CliError>()
            else {
                panic!("expected Permission, got {err:?}");
            };
            assert!(msg.contains("no per-user services"), "{msg}");
            assert!(msg.contains("sudo"), "{msg}");
        }
        assert!(events.borrow().is_empty(), "nothing ran or was noted");
        let (applied, _) = native.finish();
        assert!(applied.is_empty(), "{applied:?}");
    }

    #[test]
    fn busybox_root_install_has_no_user_scope_note() {
        // Rendering a root install's script, as `install` writes it, raises
        // no note: every registration here is the system-wide script.
        let events = Events::default();
        let script = render_script(&reg(), &ScriptLayout::for_role("serve")).unwrap();
        assert!(script.contains("export IROH_TUNNEL_SERVICE=1"), "{script}");
        // Only a disposable privileged container may write /etc/init.d.
        if !crate::service::is_elevated()
            || std::env::var_os("IROH_TUNNEL_E2E_PRIVILEGED").is_none_or(|v| v != "1")
        {
            assert!(events.borrow().iter().all(|e| !matches!(e, Event::Note(_))));
            return;
        }
        // Real root: the install itself emits only `Installed`.
        let mut fake = FakeRunner::new(&events, vec![]);
        let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
        let mut native = Native::new(OutputMode::Capture, &mut runner);
        let mut notes = note_sink(&events);
        let _ = install(&mut native, &mut notes, &reg());
        let _ = std::fs::remove_file(script_path("serve"));
        let noted: Vec<_> = events
            .borrow()
            .iter()
            .filter_map(|e| match e {
                Event::Note(n) => Some(n.clone()),
                Event::Call { .. } => None,
            })
            .collect();
        assert!(
            noted
                .iter()
                .all(|n| matches!(n, ServiceNote::Installed { .. })),
            "{noted:?}"
        );
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
