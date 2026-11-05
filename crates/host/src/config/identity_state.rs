//! Unlocked pre-check for key rotation.

use std::path::Path;

use anyhow::Result;
use toml_edit::DocumentMut;

use super::resolve_config_path::resolve_config_path;
use super::store::{existing_secret_key, read_optional, resolve_target};
use super::types::IdentityState;
use crate::role::Role;

/// Unlocked pre-check used to decide whether to ask before taking the lock.
/// A missing or unparsable file reads as [`IdentityState::NoKey`]; a read
/// error propagates.
pub fn identity_state(role: Role, config: Option<&Path>) -> Result<IdentityState> {
    let path = resolve_target(&resolve_config_path(role, config)?);
    let has_key = read_optional(&path)?
        .and_then(|text| text.parse::<DocumentMut>().ok())
        .is_some_and(|doc| existing_secret_key(&doc).is_some());
    Ok(if has_key {
        IdentityState::HasKey
    } else {
        IdentityState::NoKey
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_and_unparsable_files_read_as_no_key() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("none.toml");
        assert_eq!(
            identity_state(Role::Serve, Some(&missing)).unwrap(),
            IdentityState::NoKey
        );
        let bad = dir.path().join("bad.toml");
        std::fs::write(&bad, "= not toml").unwrap();
        assert_eq!(
            identity_state(Role::Serve, Some(&bad)).unwrap(),
            IdentityState::NoKey
        );
    }

    #[test]
    fn a_stored_key_reads_as_has_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.toml");
        std::fs::write(&path, "[node]\nsecret_key = \"abc\"\n").unwrap();
        assert_eq!(
            identity_state(Role::Serve, Some(&path)).unwrap(),
            IdentityState::HasKey
        );
    }
}
