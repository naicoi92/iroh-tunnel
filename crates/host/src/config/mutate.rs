//! Shared mechanics of the config-editing operations: one validated, atomic,
//! serialized mutation of a role document.

use std::path::Path;

use anyhow::Result;
use toml_edit::DocumentMut;

use super::dto::{AccessConfig, ServeConfig};
use super::store::{
    check_role_doc, parse_doc, read_optional, render_role_doc, write_atomic, ConfigLock,
};
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
    let rendered = render_valid(role, &mut doc, path)?;
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

/// Check `content` against the role's schema and validation rules (name
/// regex/length, port range, node_id length, duplicate names, https
/// relay_urls). A failure is a `CliError::Config` → exit 2.
pub(super) fn validate_role_doc(role: Role, content: &str, path: &Path) -> Result<()> {
    match role {
        Role::Access => check_role_doc::<AccessConfig>(content, path).map(drop),
        Role::Serve => check_role_doc::<ServeConfig>(content, path).map(drop),
    }
}

/// Drop retired fields from `doc` and render it, refusing a result that is
/// not valid for `role`. Returns the text to write.
pub(super) fn render_valid(role: Role, doc: &mut DocumentMut, path: &Path) -> Result<String> {
    match role {
        Role::Access => render_role_doc::<AccessConfig>(doc, path),
        Role::Serve => render_role_doc::<ServeConfig>(doc, path),
    }
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
