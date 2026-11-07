//! Platform selection helpers of the service backends (private).

use std::path::Path;

use anyhow::Result;

#[cfg(unix)]
use super::types::ServiceScope;

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
        super::trust::load_config(path)
    }
    #[cfg(not(unix))]
    {
        C::load(path)
    }
}

/// The text of the currently installed system registration, if any: where a
/// previous install's pin is read from to detect a retarget (SM-02). Only
/// system installs read it, and those exist on Unix only.
#[cfg(unix)]
pub(crate) fn existing_registration(role: &str, scope: ServiceScope) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        if linux_is_systemd() {
            super::backend::systemd::existing_registration(role, scope)
        } else {
            let _ = scope;
            super::backend::busybox::existing_registration(role)
        }
    }
    #[cfg(target_os = "macos")]
    {
        super::backend::launchd::existing_registration(role, scope)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (role, scope);
        None
    }
}

/// True when systemd is the running init (PID 1). Used to pick the systemd vs
/// BusyBox/SysV backend at runtime.
///
/// Primary signal: systemd creates `/run/systemd/system` when it manages the
/// boot. Fallback: read the target of `/proc/1/exe` (the kernel exposes PID 1's
/// executable there) and check it ends in `systemd`. Both are no-ops (return
/// false) on a non-systemd host and on non-Linux builds.
#[cfg(target_os = "linux")]
pub(crate) fn linux_is_systemd() -> bool {
    if std::path::Path::new("/run/systemd/system").exists() {
        return true;
    }
    std::fs::read_link("/proc/1/exe")
        .map(|p| p.ends_with("systemd"))
        .unwrap_or(false)
}

/// Whether every registration on this host is system-wide: a Linux host whose
/// init is not systemd (BusyBox), which has no per-user services.
///
/// `backend_install` picks its backend on the same condition.
#[cfg(target_os = "linux")]
pub(crate) fn system_wide_only() -> bool {
    !linux_is_systemd()
}

/// Whether every registration on this host is system-wide; only Linux without
/// systemd is, so never here.
#[cfg(not(target_os = "linux"))]
pub(crate) fn system_wide_only() -> bool {
    false
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub(crate) fn unsupported<T>() -> Result<T> {
    anyhow::bail!(
        "service management is only supported on Linux (systemd/BusyBox init), macOS (launchd) \
         and Windows (user background task)"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // Compile-only guard: on Linux the runtime probe must exist and be bool.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_is_systemd_is_bool() {
        let _ = linux_is_systemd();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn system_wide_only_is_the_negation_of_systemd_on_linux() {
        assert_eq!(system_wide_only(), !linux_is_systemd());
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn system_wide_only_is_false_off_linux() {
        assert!(!system_wide_only());
    }
}
