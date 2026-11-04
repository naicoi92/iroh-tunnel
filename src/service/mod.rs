//! System-service management: install/uninstall/start/stop/restart/status.
//!
//! Implements T-12. The macOS backend is launchd. On Linux the backend is
//! chosen at RUNTIME (not compile time): systemd when the host boots with it
//! (detected via `/run/systemd/system`), otherwise a BusyBox/SysV init backend
//! that installs an `/etc/init.d/S??iroh-tunnel-<role>` script. The latter
//! covers embedded buildroot devices such as the Sipeed NanoKVM, whose busybox
//! init runs every executable `/etc/init.d/S*` script on boot. On Windows the
//! backend is a per-user Task Scheduler task that runs in the logged-in
//! session (user scope only; SM-04, issue #92).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::error::CliError;
#[allow(unused_imports)] // unused on targets without a service backend
use native::{system_runner, Native};

pub mod account;
#[cfg(target_os = "linux")]
mod busybox;
#[cfg(target_os = "macos")]
mod launchd;
mod native;
pub use native::{InstalledAt, NativeOutput, OutputMode, ServiceCall, ServiceNote};
pub mod pin;
pub mod scope;
#[cfg(unix)]
pub mod system;
#[cfg(target_os = "linux")]
mod systemd;
mod task_xml;
#[cfg(unix)]
pub mod trust;
#[cfg(windows)]
mod windows;

/// Load a role config for `run`, enforcing the privileged trust policy when a
/// system service asks for it (SM-04). Plain read elsewhere.
///
/// # Errors
///
/// [`CliError::Permission`] for an unsafe path under a system service, or the
/// ordinary config errors.
pub(crate) fn load_config<C: crate::config::RoleDoc>(path: &Path) -> Result<C> {
    #[cfg(unix)]
    {
        trust::load_config(path)
    }
    #[cfg(not(unix))]
    {
        C::load(path)
    }
}

/// Restart policy shared by every platform's service template.
///
/// Both `systemd::format_unit` and `launchd::format_plist` read from this so
/// the policy has a single source of truth — the previous design hard-coded it
/// separately in each template (`Restart=on-failure`/`RestartSec=5` in the
/// unit, `KeepAlive SuccessfulExit=false` in the plist), which silently drifted
/// whenever one side was edited.
///
/// `delay_secs` is honored by systemd only; launchd manages its own throttle
/// interval internally and has no user-settable knob for it. The field stays
/// shared so the policy reads as one coherent thing, even if one platform
/// ignores the backoff.
#[allow(dead_code)] // unused on targets without a service backend
pub(crate) struct RestartPolicy {
    /// Restart the service when it exits with a non-zero status.
    pub on_failure: bool,
    /// Delay between restart attempts, in seconds. Read by systemd; ignored
    /// by launchd (which uses an internal ThrottleInterval).
    #[allow(dead_code)] // unread on macOS builds (launchd has no equivalent knob)
    pub delay_secs: u64,
}

impl RestartPolicy {
    /// The policy iroh-tunnel ships with: restart on crash, 5 s backoff.
    /// Matches the previous hard-coded values in both templates.
    #[allow(dead_code)] // unused on targets without a service backend
    pub const DEFAULT: RestartPolicy = RestartPolicy {
        on_failure: true,
        delay_secs: 5,
    };
}

/// Whether the service is installed system-wide or per-user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceScope {
    System,
    User,
}

// ---------------------------------------------------------------------------
// Public dispatchers — one per ServiceAction. They forward to the platform
// module selected at compile time. On Linux the choice between systemd and the
// BusyBox/SysV backend is made at runtime (see [`linux_is_systemd`]); on macOS
// it is always launchd. Each has the same cfg(linux)/cfg(macos)/
// cfg(not(...)) skeleton; they're kept explicit rather than factored into a
// macro because (a) `install` takes an extra `config` arg the others don't,
// (b) the unsupported fallback needs to consume the args to stay warning-free,
// and (c) six functions × three cfg arms reads more clearly than a macro that
// has to solve all of that generically.
// ---------------------------------------------------------------------------

/// Run `f` with a [`Native`] in the call's output mode and the call's note
/// observer.
#[allow(dead_code)] // unused on targets without a service backend
fn with_native<T>(
    call: &mut ServiceCall,
    f: impl FnOnce(&mut Native, &mut dyn FnMut(&ServiceNote)) -> T,
) -> T {
    let mut runner = system_runner;
    let mut native = Native::new(call.output, &mut runner);
    f(&mut native, &mut *call.notes)
}

/// `service install`: write the unit/plist/init-script, enable, and start.
///
/// `config` and `state` must be absolute: they are pinned into the
/// registration so the service never depends on its own working directory or
/// home (SM-02).
///
/// `pin` is recorded in a system registration so the root service trusts only
/// that user's files (SM-04); it is `None` for a user service.
pub fn install(
    role: &str,
    scope: ServiceScope,
    config: &Path,
    state: &Path,
    pin: Option<&pin::Pin>,
    call: &mut ServiceCall,
) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        if linux_is_systemd() {
            with_native(call, |n, notes| {
                systemd::install(n, notes, role, scope, config, state, pin)
            })
        } else {
            with_native(call, |n, notes| {
                busybox::install(n, notes, role, scope, config, state, pin)
            })
        }
    }
    #[cfg(target_os = "macos")]
    {
        with_native(call, |n, notes| {
            launchd::install(n, notes, role, scope, config, state, pin)
        })
    }
    #[cfg(windows)]
    {
        let _ = pin;
        with_native(call, |n, notes| {
            windows::install(n, notes, role, scope, config, state)
        })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = (role, scope, config, state, pin, call);
        unsupported()
    }
}

/// The text of the currently installed system registration, if any: where a
/// previous install's pin is read from to detect a retarget (SM-02).
pub fn existing_registration(role: &str, scope: ServiceScope) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        if linux_is_systemd() {
            systemd::existing_registration(role, scope)
        } else {
            let _ = scope;
            busybox::existing_registration(role)
        }
    }
    #[cfg(target_os = "macos")]
    {
        launchd::existing_registration(role, scope)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (role, scope);
        None
    }
}

/// `service uninstall`: stop, disable, and remove the unit/plist/init-script.
pub fn uninstall(role: &str, scope: ServiceScope, call: &mut ServiceCall) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        if linux_is_systemd() {
            with_native(call, |n, notes| systemd::uninstall(n, notes, role, scope))
        } else {
            with_native(call, |n, notes| busybox::uninstall(n, notes, role, scope))
        }
    }
    #[cfg(target_os = "macos")]
    {
        with_native(call, |n, notes| launchd::uninstall(n, notes, role, scope))
    }
    #[cfg(windows)]
    {
        with_native(call, |n, notes| windows::uninstall(n, notes, role, scope))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = (role, scope, call);
        unsupported()
    }
}

/// `service start`.
pub fn start(role: &str, scope: ServiceScope, call: &mut ServiceCall) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        if linux_is_systemd() {
            with_native(call, |n, _| systemd::start(n, role, scope))
        } else {
            with_native(call, |n, _| busybox::start(n, role, scope))
        }
    }
    #[cfg(target_os = "macos")]
    {
        with_native(call, |n, _| launchd::start(n, role, scope))
    }
    #[cfg(windows)]
    {
        with_native(call, |n, _| windows::start(n, role, scope))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = (role, scope, call);
        unsupported()
    }
}

/// `service stop`.
pub fn stop(role: &str, scope: ServiceScope, call: &mut ServiceCall) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        if linux_is_systemd() {
            with_native(call, |n, notes| systemd::stop(n, notes, role, scope))
        } else {
            with_native(call, |n, notes| busybox::stop(n, notes, role, scope))
        }
    }
    #[cfg(target_os = "macos")]
    {
        with_native(call, |n, notes| launchd::stop(n, notes, role, scope))
    }
    #[cfg(windows)]
    {
        with_native(call, |n, notes| windows::stop(n, notes, role, scope))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = (role, scope, call);
        unsupported()
    }
}

/// `service restart`.
pub fn restart(role: &str, scope: ServiceScope, call: &mut ServiceCall) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        if linux_is_systemd() {
            with_native(call, |n, notes| systemd::restart(n, notes, role, scope))
        } else {
            with_native(call, |n, notes| busybox::restart(n, notes, role, scope))
        }
    }
    #[cfg(target_os = "macos")]
    {
        with_native(call, |n, notes| launchd::restart(n, notes, role, scope))
    }
    #[cfg(windows)]
    {
        with_native(call, |n, notes| windows::restart(n, notes, role, scope))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = (role, scope, call);
        unsupported()
    }
}

/// `service status`: running/stopped (+ pid when available), shown by the
/// native tool (inherited) or returned in the [`NativeOutput`] (captured).
pub fn status(role: &str, scope: ServiceScope, call: &mut ServiceCall) -> Result<NativeOutput> {
    #[cfg(target_os = "linux")]
    {
        if linux_is_systemd() {
            with_native(call, |n, notes| systemd::status(n, notes, role, scope))
        } else {
            with_native(call, |n, notes| busybox::status(n, notes, role, scope))
        }
    }
    #[cfg(target_os = "macos")]
    {
        with_native(call, |n, notes| launchd::status(n, notes, role, scope))
    }
    #[cfg(windows)]
    {
        with_native(call, |n, notes| windows::status(n, notes, role, scope))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = (role, scope, call);
        unsupported()
    }
}

// ---------------------------------------------------------------------------
// Registration helpers shared by the backends.
// ---------------------------------------------------------------------------

/// Name suffix that keeps a test registration apart from any real service.
/// Empty in production builds; `test-utils` reads it from the environment.
#[allow(dead_code)] // unused on targets without a service backend
fn instance_suffix() -> String {
    #[cfg(feature = "test-utils")]
    if let Some(suffix) = std::env::var_os("IROH_TUNNEL_TEST_SERVICE_SUFFIX") {
        return suffix.to_string_lossy().into_owned();
    }
    String::new()
}

/// Write a unit/plist/script, creating its directory.
///
/// # Errors
///
/// [`CliError::Permission`] when the OS denies access. The scope and account
/// are left as requested and the message says how to proceed, because
/// retrying under another scope or account would silently broaden access.
#[allow(dead_code)] // unused on targets without a service backend
pub(super) fn write_registration(path: &Path, contents: &str) -> Result<()> {
    let denied = |e: std::io::Error, what: &str, target: &Path| -> anyhow::Error {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            CliError::Permission(format!(
                "permission denied {what} {}: {e}. The service scope and account were not \
                 changed and nothing was registered. Fix the permissions of that location and \
                 re-run; use --system (as root) only if you intend a system service",
                target.display()
            ))
            .into()
        } else {
            anyhow::Error::new(e).context(format!("failed {what} {}", target.display()))
        }
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| denied(e, "creating", parent))?;
    }
    // Atomic and never group/world-writable whatever the umask: a root-owned
    // unit that a group could edit would be a path to root (SM-04), and a
    // crash mid-write must not leave a truncated unit.
    let temp = path.with_extension(format!("tmp{}", std::process::id()));
    let written = (|| {
        std::fs::write(&temp, contents)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = if path.extension().is_none() {
                0o755
            } else {
                0o644
            };
            std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(mode))?;
        }
        std::fs::rename(&temp, path)
    })();
    written.map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        denied(e, "writing", path)
    })
}

/// The text of an installed registration. Absent is `None`; a registration
/// that exists but cannot be read counts as one with an unknown owner (empty
/// text), never as "nothing installed", so the retarget prompt is not skipped.
#[allow(dead_code)] // unused on targets without a service backend
pub(super) fn read_registration(path: &Path) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => Some(String::new()),
    }
}

// ---------------------------------------------------------------------------
// Linux init-system detection.
// ---------------------------------------------------------------------------

/// True when systemd is the running init (PID 1). Used to pick the systemd vs
/// BusyBox/SysV backend at runtime.
///
/// Primary signal: systemd creates `/run/systemd/system` when it manages the
/// boot. Fallback: read the target of `/proc/1/exe` (the kernel exposes PID 1's
/// executable there) and check it ends in `systemd`. Both are no-ops (return
/// false) on a non-systemd host and on non-Linux builds.
#[cfg(target_os = "linux")]
fn linux_is_systemd() -> bool {
    if std::path::Path::new("/run/systemd/system").exists() {
        return true;
    }
    std::fs::read_link("/proc/1/exe")
        .map(|p| p.ends_with("systemd"))
        .unwrap_or(false)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn unsupported<T>() -> Result<T> {
    anyhow::bail!(
        "service management is only supported on Linux (systemd/BusyBox init), macOS (launchd) \
         and Windows (user background task)"
    )
}

// ---------------------------------------------------------------------------
// Resolve the iroh-tunnel binary path (shared across platforms).
// ---------------------------------------------------------------------------

/// Locate the `iroh-tunnel` executable to put in the unit/plist `ExecStart`.
///
/// Prefer `which` (so a system install is used); fall back to the current
/// executable so `cargo run -- service install` works during development.
#[allow(dead_code)] // unused on targets without a service backend
pub(super) fn resolve_binary() -> Result<PathBuf> {
    if let Ok(p) = which::which("iroh-tunnel") {
        return Ok(p);
    }
    let current = std::env::current_exe().context("failed to resolve current executable")?;
    Ok(current)
}

// Shared unit test covering the cross-platform ServiceScope + dispatch API
// shape, so the module is exercised on every platform (not just the host's).
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_variants_exist() {
        // Compile + equality smoke test for the public enum.
        assert_ne!(ServiceScope::System, ServiceScope::User);
    }

    #[test]
    fn resolve_binary_returns_a_path() {
        // In tests the current exe is always resolvable, so the fallback path
        // is exercised at minimum.
        let p = resolve_binary().expect("resolve_binary should not fail in tests");
        assert!(p.is_absolute(), "binary path should be absolute");
    }

    #[test]
    fn restart_policy_default_matches_previous_hardcoded_values() {
        // Regression guard: the shared RestartPolicy must produce the same
        // values the per-platform templates used to hard-code (Restart=
        // on-failure, RestartSec=5 / KeepAlive SuccessfulExit=false).
        let p = RestartPolicy::DEFAULT;
        assert!(p.on_failure, "default policy must restart on failure");
        assert_eq!(p.delay_secs, 5, "default policy must keep the 5s backoff");
    }

    // Compile-only guard: on Linux the runtime probe must exist and be bool.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_is_systemd_is_bool() {
        let _ = linux_is_systemd();
    }
}
