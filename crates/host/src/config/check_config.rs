//! `config check`: validate a config file without taking the lock or writing.

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::mutate::validate_role_doc;
use super::require_existing::require_existing;
use super::resolve_config_path::resolve_config_path;
use super::store::resolve_target;
use crate::role::Role;

/// Validate the role's config with the same schema rules as the mutations.
///
/// # Errors
///
/// [`crate::error::CliError::Config`] when the file is missing or invalid.
pub fn check_config(role: Role, config: Option<&Path>) -> Result<PathBuf> {
    let path = resolve_target(&resolve_config_path(role, config)?);
    let text = require_existing(role, &path)?;
    validate_role_doc(role, &text, &path)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::fixtures::VALID_SERVE;
    use crate::config::store::ConfigLock;
    use crate::error::CliError;
    use std::time::{Duration, Instant};

    fn config_msg(err: &anyhow::Error) -> String {
        match err.downcast_ref::<CliError>() {
            Some(CliError::Config(m)) => m.clone(),
            _ => panic!("expected CliError::Config, got {err:?}"),
        }
    }

    #[test]
    fn check_valid_returns_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.toml");
        std::fs::write(&path, VALID_SERVE).unwrap();
        assert_eq!(check_config(Role::Serve, Some(&path)).unwrap(), path);
    }

    #[test]
    fn check_invalid_is_config_error_and_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.toml");
        let text = "[[services]]\nname = \"Bad Name!\"\nprotocol = \"tcp\"\nhost = \"127.0.0.1\"\nport = 8080\n";
        std::fs::write(&path, text).unwrap();
        let err = check_config(Role::Serve, Some(&path)).unwrap_err();
        config_msg(&err);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }

    #[test]
    fn check_missing_uses_require_existing_guidance() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.toml");
        let err = check_config(Role::Serve, Some(&path)).unwrap_err();
        assert!(config_msg(&err).contains("config keygen"));
        assert!(!path.exists());
    }

    #[test]
    fn check_invalid_diagnostic_has_no_secret() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.toml");
        std::fs::write(&path, "[node]\nrelay_token = \"leak-me\" oops\n").unwrap();
        let msg = config_msg(&check_config(Role::Serve, Some(&path)).unwrap_err());
        assert!(msg.contains("line 2"), "{msg}");
        assert!(!msg.contains("leak-me"), "{msg}");
    }

    #[test]
    fn check_ignores_held_lock_and_never_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.toml");
        let text = format!("{VALID_SERVE}multiplex = true\n");
        std::fs::write(&path, &text).unwrap();
        let _lock = ConfigLock::acquire(&path).unwrap();
        let start = Instant::now();
        let got = check_config(Role::Serve, Some(&path)).unwrap();
        assert!(start.elapsed() < Duration::from_secs(1));
        assert_eq!(got, path);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }
}
