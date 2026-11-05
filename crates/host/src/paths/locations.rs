//! Platform base directories shared by the path operations.

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

#[cfg(windows)]
pub(super) fn local_app_data() -> Result<PathBuf> {
    match dirs::data_local_dir() {
        Some(dir) if dir.is_absolute() => Ok(dir.join(APP_DIR)),
        _ => Err(config_err(
            "could not resolve the LocalAppData folder; \
             pass --config / IROH_TUNNEL_STATE_DIR explicitly"
                .into(),
        )),
    }
}

/// Default directory holding `<role>.toml`.
///
/// # Errors
///
/// [`CliError::Config`] when the home directory (or LocalAppData on Windows)
/// is missing or not absolute.
pub(super) fn default_config_dir() -> Result<PathBuf> {
    #[cfg(windows)]
    return Ok(local_app_data()?.join("config"));
    #[cfg(not(windows))]
    Ok(home_dir()?.join(".config").join(APP_DIR))
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
