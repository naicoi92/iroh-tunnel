//! Shared mechanics of the config-editing operations: one validated, atomic,
//! serialized mutation of a role document.

use std::path::Path;

use anyhow::Result;
use toml_edit::DocumentMut;

use super::store::{parse_doc, read_optional, strip_retired_fields, write_atomic, ConfigLock};
use crate::error::CliError;
use crate::role::Role;

/// Apply `change` to the config at `path` as one validated, atomic, serialized
/// mutation (CFG-04).
///
/// Takes the writer lock, loads the document (empty when the file does not
/// exist yet), applies `change`, drops retired fields, validates the result
/// through the role schema and replaces the file atomically and owner-only.
/// If `change` or validation fails the file is left byte-for-byte unchanged.
pub(super) fn mutate(
    role: Role,
    path: &Path,
    change: impl FnOnce(&mut DocumentMut) -> Result<()>,
) -> Result<()> {
    mutate_deciding(role, path, |doc| change(doc).map(|()| Edit::Apply(())))
}

/// What a [`mutate_deciding`] closure decided about the document.
pub(super) enum Edit<T> {
    /// Validate and write the edited document.
    Apply(T),
    /// Leave the file untouched.
    Keep(T),
}

/// [`mutate`] where the closure may decide, under the lock, to write nothing.
pub(super) fn mutate_deciding<T>(
    role: Role,
    path: &Path,
    change: impl FnOnce(&mut DocumentMut) -> Result<Edit<T>>,
) -> Result<T> {
    let _lock = ConfigLock::acquire(path)?;
    let text = read_optional(path)?.unwrap_or_default();
    let mut doc = parse_doc(&text, path)?;
    let value = match change(&mut doc)? {
        Edit::Keep(value) => return Ok(value),
        Edit::Apply(value) => value,
    };
    strip_retired_fields(&mut doc);
    let rendered = doc.to_string();
    validate_role_doc(role, &rendered, path)?;
    write_atomic(path, rendered.as_bytes())?;
    Ok(value)
}

/// Index of the `[[services]]` entry named `name`.
///
/// `services` absent is `Ok(None)`; present but not an array of tables is a
/// config error naming `path`.
pub(super) fn find_service_index(
    doc: &DocumentMut,
    name: &str,
    path: &Path,
) -> Result<Option<usize>> {
    let Some(item) = doc.get("services") else {
        return Ok(None);
    };
    let services = item.as_array_of_tables().ok_or_else(|| {
        CliError::Config(format!(
            "invalid config at {}: [services] must be an array of tables",
            path.display()
        ))
    })?;
    Ok(services
        .iter()
        .position(|t| t.get("name").and_then(|v| v.as_str()) == Some(name)))
}

/// Re-parse `content` through the role's schema to enforce all validation rules
/// (name regex/length, port range, node_id length, duplicate names, https
/// relay_urls). A failure is a `CliError::Config` → exit 2.
pub(super) fn validate_role_doc(role: Role, content: &str, path: &Path) -> Result<()> {
    let invalid = |detail: String| -> anyhow::Error {
        CliError::Config(format!("invalid config at {}{detail}", path.display())).into()
    };
    // `toml` errors quote the offending source line, which can hold a secret;
    // report the message and line only (RS-10).
    let parse_failure = |e: toml::de::Error| {
        invalid(format!(
            "{}: {}",
            super::store::describe_location(content, e.span()),
            e.message()
        ))
    };
    match role {
        Role::Access => toml::from_str::<crate::config::AccessConfig>(content)
            .map_err(parse_failure)?
            .validate(),
        Role::Serve => toml::from_str::<crate::config::ServeConfig>(content)
            .map_err(parse_failure)?
            .validate(),
    }
    .map_err(|e| invalid(format!(": {e}")))
}

/// Placeholder printed by `config show` instead of a secret.
pub(super) const REDACTED: &str = "<redacted>";

/// `[node]` keys `config show` hides unless `--reveal` is given.
const SECRET_FIELDS: [&str; 2] = ["secret_key", "relay_token"];

/// Replace the value of every [`SECRET_FIELDS`] key in `[node]`, keeping the
/// surrounding whitespace and comments.
pub(super) fn redact_secrets(doc: &mut DocumentMut) {
    let Some(node) = doc.get_mut("node").and_then(|n| n.as_table_like_mut()) else {
        return;
    };
    for field in SECRET_FIELDS {
        let Some(value) = node.get_mut(field).and_then(|i| i.as_value_mut()) else {
            continue;
        };
        let decor = value.decor().clone();
        *value = toml_edit::Value::from(REDACTED);
        *value.decor_mut() = decor;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_replaces_secret_values_and_keeps_comments() {
        let mut doc: DocumentMut = "[node]\nsecret_key = \"S3CRET\" # keep\nrelay_urls = []\n"
            .parse()
            .unwrap();
        redact_secrets(&mut doc);
        let text = doc.to_string();
        assert!(!text.contains("S3CRET"));
        assert!(text.contains("# keep"));
        assert!(text.contains(REDACTED));
    }
}
