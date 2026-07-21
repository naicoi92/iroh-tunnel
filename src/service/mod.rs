//! System-service management: install/uninstall/start/stop/restart/status for
//! systemd (Linux) and launchd (macOS).
//!
//! Implements T-12. Each platform is a cfg-gated submodule exposing the same
//! six actions; the top-level functions dispatch to whichever platform the
//! binary was built for. Based on Page 06 v5 §1.3 (service subcommand) and
//! §1.3.2/§1.3.3 (unit/plist templates).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

#[cfg(target_os = "macos")]
mod launchd;
#[cfg(target_os = "linux")]
mod systemd;

/// Whether the service is installed system-wide or per-user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceScope {
    System,
    User,
}

// ---------------------------------------------------------------------------
// Public dispatchers — one per ServiceAction. They forward to the platform
// module selected at compile time.
// ---------------------------------------------------------------------------

/// `service install`: write the unit/plist, enable, and start.
pub fn install(role: &str, scope: ServiceScope, config: &Path) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        systemd::install(role, scope, config)
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

/// `service uninstall`: stop, disable, and remove the unit/plist.
pub fn uninstall(role: &str, scope: ServiceScope) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        systemd::uninstall(role, scope)
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
        systemd::start(role, scope)
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
        systemd::stop(role, scope)
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
        systemd::restart(role, scope)
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
        systemd::status(role, scope)
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

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn unsupported() -> Result<()> {
    anyhow::bail!("service management is only supported on Linux (systemd) and macOS (launchd)")
}

// ---------------------------------------------------------------------------
// Resolve the iroh-tunnel binary path (shared across platforms).
// ---------------------------------------------------------------------------

/// Locate the `iroh-tunnel` executable to put in the unit/plist `ExecStart`.
///
/// Prefer `which` (so a system install is used); fall back to the current
/// executable so `cargo run -- service install` works during development.
fn resolve_binary() -> Result<PathBuf> {
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
}
