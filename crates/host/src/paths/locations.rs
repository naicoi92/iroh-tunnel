//! Platform base directories shared by the path operations.

#[cfg(windows)]
use std::path::Path;
use std::path::PathBuf;

use anyhow::Result;

use crate::error::CliError;

pub(super) const APP_DIR: &str = "iroh-tunnel";

pub(super) fn config_err(msg: String) -> anyhow::Error {
    CliError::Config(msg).into()
}

#[cfg(not(windows))]
pub(super) fn home_dir() -> Result<PathBuf> {
    match dirs::home_dir() {
        Some(home) if home.is_absolute() => Ok(home),
        _ => Err(config_err(
            "could not determine an absolute home directory (is HOME set?); \
             set HOME, or pass --config / IROH_TUNNEL_STATE_DIR explicitly"
                .into(),
        )),
    }
}

/// The LocalAppData root of the current user (without the app directory).
#[cfg(windows)]
pub(crate) fn local_app_data() -> Result<PathBuf> {
    match dirs::data_local_dir() {
        Some(dir) if dir.is_absolute() => Ok(dir),
        _ => Err(config_err(
            "could not resolve the LocalAppData folder; \
             pass --config / IROH_TUNNEL_STATE_DIR explicitly"
                .into(),
        )),
    }
}

/// Config directory under a Windows LocalAppData `root`.
#[cfg(windows)]
pub(super) fn windows_config_dir(root: &Path) -> PathBuf {
    root.join(APP_DIR).join("config")
}

/// State directory under a Windows LocalAppData `root`.
#[cfg(windows)]
pub(super) fn windows_state_dir(root: &Path) -> PathBuf {
    root.join(APP_DIR).join("state")
}

/// Default directory holding `<role>.toml`.
///
/// # Errors
///
/// [`CliError::Config`] when the home directory (or LocalAppData on Windows)
/// is missing or not absolute.
pub(super) fn default_config_dir() -> Result<PathBuf> {
    #[cfg(windows)]
    return Ok(windows_config_dir(&local_app_data()?));
    #[cfg(not(windows))]
    Ok(home_dir()?.join(".config").join(APP_DIR))
}

/// Machine-wide config directory of the system tier.
///
/// # Errors
///
/// [`CliError::Config`] when the Windows `ProgramData` folder cannot be
/// resolved.
pub(super) fn system_config_dir() -> Result<PathBuf> {
    #[cfg(target_os = "linux")]
    return Ok(PathBuf::from("/etc").join(APP_DIR));
    #[cfg(all(unix, not(target_os = "linux")))]
    return Ok(PathBuf::from("/Library/Application Support")
        .join(APP_DIR)
        .join("config"));
    #[cfg(windows)]
    return Ok(program_data()?.join(APP_DIR).join("config"));
}

/// Machine-wide runtime state directory of the system tier.
///
/// # Errors
///
/// [`CliError::Config`] when the Windows `ProgramData` folder cannot be
/// resolved.
pub(super) fn system_state_dir() -> Result<PathBuf> {
    #[cfg(target_os = "linux")]
    return Ok(PathBuf::from("/run").join(APP_DIR));
    #[cfg(all(unix, not(target_os = "linux")))]
    return Ok(PathBuf::from("/var/run").join(APP_DIR));
    #[cfg(windows)]
    return Ok(program_data()?.join(APP_DIR).join("state"));
}

#[cfg(windows)]
fn program_data() -> Result<PathBuf> {
    crate::service::program_data_dir().map_err(|e| config_err(format!("{e:#}")))
}

#[cfg(test)]
mod tests {
    #[cfg(not(windows))]
    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn config_dir_is_the_app_dir_under_dot_config() {
        if let Ok(dir) = default_config_dir() {
            assert!(dir.is_absolute());
            assert!(dir.ends_with(".config/iroh-tunnel"));
        }
    }
}
