//! Platform selection helpers of the service backends (private).

use std::path::Path;

use anyhow::Result;

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
/// previous install's pin is read from to detect a retarget (SM-02).
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

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub(crate) fn unsupported<T>() -> Result<T> {
    anyhow::bail!(
        "service management is only supported on Linux (systemd/BusyBox init), macOS (launchd) \
         and Windows (user background task)"
    )
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    use super::*;

    // Compile-only guard: on Linux the runtime probe must exist and be bool.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_is_systemd_is_bool() {
        let _ = linux_is_systemd();
    }
}
