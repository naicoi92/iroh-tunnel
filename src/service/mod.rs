//! System-service management: install/uninstall/start/stop/restart/status.
//!
//! Implements T-12. The macOS backend is launchd. On Linux the backend is
//! chosen at RUNTIME (not compile time): systemd when the host boots with it
//! (detected via `/run/systemd/system`), otherwise a BusyBox/SysV init backend
//! that installs an `/etc/init.d/S??iroh-tunnel-<role>` script. The latter
//! covers embedded buildroot devices such as the Sipeed NanoKVM, whose busybox
//! init runs every executable `/etc/init.d/S*` script on boot.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

#[cfg(target_os = "linux")]
mod busybox;
#[cfg(target_os = "macos")]
mod launchd;
#[cfg(target_os = "linux")]
mod systemd;

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

/// `service install`: write the unit/plist/init-script, enable, and start.
pub fn install(role: &str, scope: ServiceScope, config: &Path) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        if linux_is_systemd() {
            systemd::install(role, scope, config)
        } else {
            busybox::install(role, scope, config)
        }
    }
    #[cfg(target_os = "macos")]
    {
        launchd::install(role, scope, config)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (role, scope, config);
        unsupported()
    }
}

/// `service uninstall`: stop, disable, and remove the unit/plist/init-script.
pub fn uninstall(role: &str, scope: ServiceScope) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        if linux_is_systemd() {
            systemd::uninstall(role, scope)
        } else {
            busybox::uninstall(role, scope)
        }
    }
    #[cfg(target_os = "macos")]
    {
        launchd::uninstall(role, scope)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (role, scope);
        unsupported()
    }
}

/// `service start`.
pub fn start(role: &str, scope: ServiceScope) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        if linux_is_systemd() {
            systemd::start(role, scope)
        } else {
            busybox::start(role, scope)
        }
    }
    #[cfg(target_os = "macos")]
    {
        launchd::start(role, scope)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (role, scope);
        unsupported()
    }
}

/// `service stop`.
pub fn stop(role: &str, scope: ServiceScope) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        if linux_is_systemd() {
            systemd::stop(role, scope)
        } else {
            busybox::stop(role, scope)
        }
    }
    #[cfg(target_os = "macos")]
    {
        launchd::stop(role, scope)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (role, scope);
        unsupported()
    }
}

/// `service restart`.
pub fn restart(role: &str, scope: ServiceScope) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        if linux_is_systemd() {
            systemd::restart(role, scope)
        } else {
            busybox::restart(role, scope)
        }
    }
    #[cfg(target_os = "macos")]
    {
        launchd::restart(role, scope)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (role, scope);
        unsupported()
    }
}

/// `service status`: print running/stopped (+ pid when available).
pub fn status(role: &str, scope: ServiceScope) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        if linux_is_systemd() {
            systemd::status(role, scope)
        } else {
            busybox::status(role, scope)
        }
    }
    #[cfg(target_os = "macos")]
    {
        launchd::status(role, scope)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (role, scope);
        unsupported()
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

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn unsupported() -> Result<()> {
    anyhow::bail!(
        "service management is only supported on Linux (systemd/BusyBox init) and macOS (launchd)"
    )
}

// ---------------------------------------------------------------------------
// Resolve the iroh-tunnel binary path (shared across platforms).
// ---------------------------------------------------------------------------

/// Locate the `iroh-tunnel` executable to put in the unit/plist `ExecStart`.
///
/// Prefer `which` (so a system install is used); fall back to the current
/// executable so `cargo run -- service install` works during development.
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
