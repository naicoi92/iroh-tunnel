//! Default directory of the status files in a tier.

use std::path::PathBuf;

use anyhow::Result;

use super::locations::system_state_dir;
#[cfg(not(windows))]
use super::locations::{home_dir, APP_DIR};
#[cfg(windows)]
use super::locations::{local_app_data, windows_state_dir};
use crate::Tier;

/// Default state directory in `tier`. The system tier is a fixed
/// machine-wide runtime path that never consults the home directory.
///
/// # Errors
///
/// [`crate::error::CliError::Config`] when the user home directory (or
/// LocalAppData on Windows) is missing or not absolute, or when the Windows
/// `ProgramData` folder cannot be resolved for the system tier.
pub fn default_state_dir_for(tier: Tier) -> Result<PathBuf> {
    if tier == Tier::System {
        return system_state_dir();
    }
    #[cfg(windows)]
    return Ok(windows_state_dir(&local_app_data()?));
    #[cfg(not(windows))]
    Ok(home_dir()?.join(".local").join("state").join(APP_DIR))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_state_dir_is_platform_runtime_path() {
        let dir = default_state_dir_for(Tier::System).unwrap();
        #[cfg(target_os = "linux")]
        assert_eq!(dir, std::path::Path::new("/run/iroh-tunnel"));
        #[cfg(target_os = "macos")]
        assert_eq!(dir, std::path::Path::new("/var/run/iroh-tunnel"));
        #[cfg(windows)]
        assert_eq!(
            dir,
            crate::service::program_data_dir()
                .unwrap()
                .join("iroh-tunnel")
                .join("state")
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn state_dir_is_under_dot_local_state() {
        if let Ok(dir) = default_state_dir_for(Tier::User) {
            assert!(dir.is_absolute());
            assert!(dir.ends_with(".local/state/iroh-tunnel"));
        }
    }
}
