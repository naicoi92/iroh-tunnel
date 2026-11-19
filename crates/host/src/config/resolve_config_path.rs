//! Resolution of a role's configuration path.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::role::Role;

/// Resolve a role configuration path. An explicit path takes precedence over
/// the platform default. With `test-utils`, an absolute test config root may
/// replace the platform root; a relative override is rejected.
///
/// Production builds resolve defaults with [`crate::paths`].
///
/// # Errors
///
/// Fails when the platform root is unavailable or the test root is not absolute.
pub fn resolve_config_path(role: Role, override_path: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = override_path {
        return Ok(path.to_path_buf());
    }
    resolve_config_path_for(role, crate::Tier::current())
}

/// The default configuration path of `role` in `tier`, for callers that act
/// for a tier other than the process's own (the service layer derives it from
/// the injected privilege). The `test-utils` config root applies to both
/// tiers.
///
/// # Errors
///
/// As [`resolve_config_path`].
pub(crate) fn resolve_config_path_for(role: Role, tier: crate::Tier) -> Result<PathBuf> {
    #[cfg(feature = "test-utils")]
    if let Some(dir) = std::env::var_os("IROH_TUNNEL_TEST_CONFIG_DIR") {
        let dir = std::path::PathBuf::from(dir);
        anyhow::ensure!(dir.is_absolute(), "test config directory must be absolute");
        return Ok(dir
            .join("iroh-tunnel")
            .join(format!("{}.toml", role.name())));
    }
    crate::paths::default_config_path_for(role.name(), tier)
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
