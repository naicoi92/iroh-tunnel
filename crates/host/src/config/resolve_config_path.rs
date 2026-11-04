//! Resolution of a role's configuration path.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::role::Role;

/// Resolve a role configuration path. An explicit path takes precedence over
/// the platform default. With `test-utils`, an absolute test config root may
/// replace the platform root; a relative override is rejected.
///
/// Returns an error when the platform root is unavailable or the test root is
/// not absolute. Production builds resolve defaults with [`crate::paths`].
pub fn resolve_config_path(role: Role, override_path: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = override_path {
        return Ok(path.to_path_buf());
    }
    #[cfg(feature = "test-utils")]
    if let Some(dir) = std::env::var_os("IROH_TUNNEL_TEST_CONFIG_DIR") {
        let dir = std::path::PathBuf::from(dir);
        anyhow::ensure!(dir.is_absolute(), "test config directory must be absolute");
        return Ok(dir
            .join("iroh-tunnel")
            .join(format!("{}.toml", role.name())));
    }
    crate::paths::default_config_path(role.name())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_path_wins_over_the_platform_default() {
        let explicit = Path::new("/somewhere/serve.toml");
        assert_eq!(
            resolve_config_path(Role::Serve, Some(explicit)).unwrap(),
            explicit
        );
    }
}
