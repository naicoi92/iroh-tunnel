//! `config keygen`: write a fresh node identity into a role config.

use std::path::Path;

use anyhow::Result;

use super::mutate::{mutate_deciding, Edit};
use super::resolve_config_path::resolve_config_path;
use super::store::{ensure_node_table, existing_secret_key, resolve_target};
use super::types::{KeygenOutcome, RotationConsent};
use crate::role::Role;

/// Generate a new SecretKey and write it into `[node].secret_key`.
///
/// Works for both roles: serve pins the node identity that access peers dial,
/// and access pins its own identity so the serve side sees a stable peer.
/// Replacing an existing key changes the NodeId, so without
/// [`RotationConsent::Given`] the decision (made under the writer lock) is
/// [`KeygenOutcome::ConsentNeeded`] and the file is left byte-identical.
pub fn keygen(
    role: Role,
    config: Option<&Path>,
    consent: RotationConsent,
) -> Result<KeygenOutcome> {
    let path = resolve_target(&resolve_config_path(role, config)?);
    let key = iroh::SecretKey::generate();
    let key_str = crate::config::encode_secret_key(&key);
    let outcome = mutate_deciding(role, &path, |doc| {
        let existing = existing_secret_key(doc);
        if existing.is_some() && consent == RotationConsent::NotGiven {
            let current_node_id = existing
                .and_then(|s| crate::config::resolve_secret_key(s).ok())
                .map(|(k, _)| k.public().to_string());
            return Ok(Edit::Keep(KeygenOutcome::ConsentNeeded {
                path: path.clone(),
                current_node_id,
            }));
        }
        let rotated = existing.is_some();
        ensure_node_table(doc, &path)?;
        doc["node"]["secret_key"] = toml_edit::value(key_str.as_str());
        Ok(Edit::Apply(KeygenOutcome::Generated {
            path: path.clone(),
            node_id: key.public().to_string(),
            rotated,
        }))
    })?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::{write_keyed, KEYED};
    use super::super::identity_state::identity_state;
    use super::super::types::IdentityState;
    use super::*;

    #[test]
    fn keygen_without_consent_returns_consent_needed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.toml");
        let old_id = write_keyed(&path);
        let before = std::fs::read(&path).unwrap();
        let outcome = keygen(Role::Serve, Some(&path), RotationConsent::NotGiven).unwrap();
        assert_eq!(
            outcome,
            KeygenOutcome::ConsentNeeded {
                path: path.clone(),
                current_node_id: Some(old_id)
            }
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn keygen_with_consent_rotates_and_returns_new_node_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.toml");
        let old_id = write_keyed(&path);
        let KeygenOutcome::Generated {
            node_id, rotated, ..
        } = keygen(Role::Serve, Some(&path), RotationConsent::Given).unwrap()
        else {
            panic!("expected Generated");
        };
        assert!(rotated);
        assert_ne!(node_id, old_id);
        assert_eq!(
            identity_state(Role::Serve, Some(&path)).unwrap(),
            IdentityState::HasKey
        );
    }

    #[test]
    fn first_keygen_needs_no_consent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.toml");
        std::fs::write(&path, KEYED).unwrap();
        assert_eq!(
            identity_state(Role::Serve, Some(&path)).unwrap(),
            IdentityState::NoKey
        );
        let outcome = keygen(Role::Serve, Some(&path), RotationConsent::NotGiven).unwrap();
        assert!(matches!(
            outcome,
            KeygenOutcome::Generated { rotated: false, .. }
        ));
    }
}
