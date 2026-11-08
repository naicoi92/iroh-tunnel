//! Reading the persistent secret key a quick-mode run is given.

use std::path::Path;

use anyhow::Result;

use super::resolve_secret_key::resolve_secret_key;
use crate::error::CliError;

/// Read the persistent secret key supplied with `-k/--key-file`.
///
/// Returns an empty string (ephemeral identity) when no file is given. The
/// file holds the base64 key written by `config keygen`. Diagnostics name the
/// path but never echo file contents (RS-10); nothing is written back (RS-05).
///
/// # Errors
///
/// [`CliError::Config`] when the file cannot be read, is empty, or does not
/// hold a valid base64 32-byte secret key (RS-06).
pub(super) fn read_key_file(path: Option<&Path>) -> Result<String> {
    let Some(path) = path else {
        return Ok(String::new());
    };
    let text = std::fs::read_to_string(path).map_err(|e| {
        CliError::Config(format!("--key-file {}: cannot read: {e}", path.display()))
    })?;
    let key = text.trim();
    if key.is_empty() {
        return Err(
            CliError::Config(format!("--key-file {}: file is empty", path.display())).into(),
        );
    }
    resolve_secret_key(key).map_err(|_| {
        CliError::Config(format!(
            "--key-file {}: not a valid base64 32-byte secret key",
            path.display()
        ))
    })?;
    Ok(key.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::encode_secret_key;

    /// The config-error message of a failed read, panicking on success or on
    /// any other error kind.
    fn config_error(path: &Path) -> String {
        let err = read_key_file(Some(path)).unwrap_err();
        match err.downcast_ref::<CliError>() {
            Some(CliError::Config(m)) => m.clone(),
            other => panic!("expected a config error, got {other:?}: {err:#}"),
        }
    }

    #[test]
    fn no_file_means_an_ephemeral_identity() {
        assert_eq!(read_key_file(None).unwrap(), "");
    }

    #[test]
    fn a_valid_key_is_returned_trimmed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("k");
        let key = encode_secret_key(&iroh::SecretKey::from_bytes(&[7; 32]));
        std::fs::write(&path, format!("  {key}\n")).unwrap();
        assert_eq!(read_key_file(Some(&path)).unwrap(), key);
    }

    #[test]
    fn a_missing_file_is_a_config_error_naming_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.key");
        let msg = config_error(&path);
        assert!(
            msg.starts_with(&format!("--key-file {}: cannot read:", path.display())),
            "{msg}"
        );
    }

    #[test]
    fn a_blank_file_is_a_config_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blank.key");
        std::fs::write(&path, " \n").unwrap();
        assert_eq!(
            config_error(&path),
            format!("--key-file {}: file is empty", path.display())
        );
    }

    #[test]
    fn an_invalid_key_is_a_config_error_that_hides_the_contents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invalid.key");
        let secret_looking = "c2VjcmV0LWJ1dC10b28tc2hvcnQ=";
        std::fs::write(&path, secret_looking).unwrap();
        let msg = config_error(&path);
        assert_eq!(
            msg,
            format!(
                "--key-file {}: not a valid base64 32-byte secret key",
                path.display()
            )
        );
        assert!(!msg.contains(secret_looking));
    }
}
