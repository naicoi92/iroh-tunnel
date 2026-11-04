//! `config show`: the config text, redacted unless disclosure is requested.

use std::path::Path;

use anyhow::Result;

use super::mutate::redact_secrets;
use super::require_existing::require_existing;
use super::resolve_config_path::resolve_config_path;
use super::store::parse_doc;
use super::types::{Disclosure, ShownConfig};
use crate::role::Role;

/// Read the config file contents with `secret_key` and `relay_token` replaced
/// by a placeholder, unless `d` is [`Disclosure::Reveal`] (D2). Comments and
/// formatting are kept; the file itself is never changed.
pub fn show_config(role: Role, config: Option<&Path>, d: Disclosure) -> Result<ShownConfig> {
    let path = resolve_config_path(role, config)?;
    let text = require_existing(role, &path)?;
    if d == Disclosure::Reveal {
        return Ok(ShownConfig(text));
    }
    // A document that does not parse cannot be redacted, so it is never echoed.
    let mut doc = parse_doc(&text, &path)?;
    redact_secrets(&mut doc);
    Ok(ShownConfig(doc.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn show_redacts_secret_key_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.toml");
        std::fs::write(&path, "[node]\nsecret_key = \"TOPSECRETVALUE\"\n").unwrap();
        let redacted = show_config(Role::Serve, Some(&path), Disclosure::Redacted).unwrap();
        assert!(!redacted.text().contains("TOPSECRETVALUE"));
        assert!(!format!("{redacted:?}").contains("TOPSECRETVALUE"));
        let revealed = show_config(Role::Serve, Some(&path), Disclosure::Reveal).unwrap();
        assert!(revealed.text().contains("TOPSECRETVALUE"));
        assert!(!format!("{revealed:?}").contains("TOPSECRETVALUE"));
    }
}
