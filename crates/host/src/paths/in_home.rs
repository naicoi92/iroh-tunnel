//! Locations under an explicit home directory (system-service install).

use std::path::{Path, PathBuf};

#[cfg(unix)]
use super::locations::APP_DIR;
use super::locations::{windows_config_dir, windows_state_dir};

/// Config file of `role` under an explicit `home`, without consulting `HOME`
/// or migrating anything. Used when root installs a system service for
/// another user's account (SM-02): the installer's own home must never be
/// substituted.
#[cfg(unix)]
pub(crate) fn config_path_in(home: &Path, role: &str) -> PathBuf {
    home.join(".config")
        .join(APP_DIR)
        .join(format!("{role}.toml"))
}

/// State directory under an explicit `home` (see [`config_path_in`]).
#[cfg(unix)]
pub(crate) fn state_dir_in(home: &Path) -> PathBuf {
    home.join(".local").join("state").join(APP_DIR)
}

/// Config file of `role` under a Windows LocalAppData `root` (system-service
/// install for the token user, whose own folder is not the caller's).
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn windows_config_path_in(root: &Path, role: &str) -> PathBuf {
    windows_config_dir(root).join(format!("{role}.toml"))
}

/// State directory under a Windows LocalAppData `root`.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn windows_state_path_in(root: &Path) -> PathBuf {
    windows_state_dir(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_locations_are_under_the_app_dir_of_the_root() {
        let root = Path::new("C:\\Users\\a\\AppData\\Local");
        assert_eq!(
            windows_config_path_in(root, "serve"),
            root.join("iroh-tunnel").join("config").join("serve.toml")
        );
        assert_eq!(
            windows_state_path_in(root),
            root.join("iroh-tunnel").join("state")
        );
    }

    #[cfg(unix)]
    #[test]
    fn locations_are_derived_from_the_given_home_only() {
        let home = Path::new("/home/someone");
        assert_eq!(
            config_path_in(home, "serve"),
            Path::new("/home/someone/.config/iroh-tunnel/serve.toml")
        );
        assert_eq!(
            state_dir_in(home),
            Path::new("/home/someone/.local/state/iroh-tunnel")
        );
    }
}
