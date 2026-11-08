//! Default directory of the status files.

use std::path::PathBuf;

use anyhow::Result;

#[cfg(not(windows))]
use super::locations::{home_dir, APP_DIR};
#[cfg(windows)]
use super::locations::{local_app_data, windows_state_dir};

/// Default directory holding the `<role>-status.json` files.
///
/// # Errors
///
/// [`CliError::Config`] when the home directory (or LocalAppData on Windows)
/// is missing or not absolute.
pub fn default_state_dir() -> Result<PathBuf> {
    #[cfg(windows)]
    return Ok(windows_state_dir(&local_app_data()?));
    #[cfg(not(windows))]
    Ok(home_dir()?.join(".local").join("state").join(APP_DIR))
}

#[cfg(test)]
mod tests {
    #[cfg(not(windows))]
    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn state_dir_is_under_dot_local_state() {
        if let Ok(dir) = default_state_dir() {
            assert!(dir.is_absolute());
            assert!(dir.ends_with(".local/state/iroh-tunnel"));
        }
    }
}
