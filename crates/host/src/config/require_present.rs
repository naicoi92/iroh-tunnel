//! The `run` existence check on a config path, without opening it.

use std::path::Path;

use anyhow::Result;

use super::require_existing::require_existing;
use crate::role::Role;

/// Check that `path` exists without opening it: `run` under a system service
/// reads the config only through the trust check, so this existence check
/// must not open a user-controlled path with the service's rights.
///
/// # Errors
///
/// The same [`CliError::Config`] as [`require_existing`].
pub fn require_present(role: Role, path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            require_existing(role, path).map(drop)
        }
        Err(e) => {
            Err(anyhow::Error::new(e).context(format!("failed to read config: {}", path.display())))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CliError;

    #[test]
    fn require_present_refuses_a_missing_config_like_require_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.toml");
        let err = require_present(Role::Serve, &path).unwrap_err();
        assert!(matches!(
            err.downcast_ref::<CliError>(),
            Some(CliError::Config(m)) if m.contains("config set")
        ));
        std::fs::write(&path, "not read").unwrap();
        require_present(Role::Serve, &path).unwrap();
    }
}
