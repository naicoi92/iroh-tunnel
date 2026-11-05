//! Locations under an explicit home directory (system-service install).

use std::path::{Path, PathBuf};

use super::locations::APP_DIR;

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

#[cfg(test)]
mod tests {
    use super::*;

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
