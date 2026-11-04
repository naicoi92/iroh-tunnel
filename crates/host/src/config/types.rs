//! Request, outcome and handle types of the config operations.

use std::path::PathBuf;

use crate::role::Role;

/// Whether the role's config already holds a `[node].secret_key`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityState {
    /// The config has no `[node].secret_key`.
    NoKey,
    /// The config already holds a `[node].secret_key`.
    HasKey,
}

/// Whether the caller agreed to replace an existing key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotationConsent {
    /// Replacing an existing key was not agreed to.
    NotGiven,
    /// Replacing an existing key was agreed to.
    Given,
}

/// Result of [`keygen`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeygenOutcome {
    /// A new key was written; `rotated` is true when it replaced an old one.
    Generated {
        /// The config file that was written.
        path: PathBuf,
        /// Public node id of the new key.
        node_id: String,
        /// Whether the new key replaced an existing one.
        rotated: bool,
    },
    /// A key exists and consent was not given; the file is untouched.
    /// `current_node_id` is `None` when the stored key cannot be decoded.
    ConsentNeeded {
        /// The config file, left untouched.
        path: PathBuf,
        /// Node id of the stored key, when it can be decoded.
        current_node_id: Option<String>,
    },
}

/// A freshly generated identity from `config keygen random`.
///
/// The secret is reachable only through [`GeneratedIdentity::secret`]; `Debug`
/// never prints it (RS-10).
pub struct GeneratedIdentity {
    pub(super) secret: String,
    /// Public node id derived from the secret.
    pub node_id: String,
}

impl GeneratedIdentity {
    /// The base64 secret key, exactly as it is stored in `[node].secret_key`.
    pub fn secret(&self) -> &str {
        &self.secret
    }
}

impl std::fmt::Debug for GeneratedIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GeneratedIdentity")
            .field("secret", &"[redacted]")
            .field("node_id", &self.node_id)
            .finish()
    }
}

/// Request to append one `[[services]]` entry. Fields are raw strings so the
/// schema validation (and its messages) run at the same stage as before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddService {
    /// Config file; `None` selects the role default.
    pub config: Option<PathBuf>,
    /// Service name.
    pub name: String,
    /// Protocol text, validated when the entry is written.
    pub protocol: String,
    /// Target (serve) or listen (access) host.
    pub host: String,
    /// Target (serve) or listen (access) port.
    pub port: u16,
    /// Required only for the access role.
    pub node_id: Option<String>,
}

/// Outcome of a service add/remove: the service name and the file changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mutation {
    /// Name of the added or removed service.
    pub service: String,
    /// Config file that was changed.
    pub path: PathBuf,
}

/// One `[[services]]` entry as listed; a missing or mistyped field is `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceRow {
    /// Service name, if present and textual.
    pub name: Option<String>,
    /// Peer node id, if present.
    pub node_id: Option<String>,
    /// Protocol text, if present.
    pub protocol: Option<String>,
    /// Host, if present.
    pub host: Option<String>,
    /// Port, if present and an integer.
    pub port: Option<i64>,
}

/// The configured services of one config file.
///
/// `rows` is `None` when `services` is not an array of tables: the key is
/// absent, or it is any other TOML shape — including the inline `services = []`
/// (an empty inline array is not an array of tables). The CLI prints the
/// "No services configured" line for `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceList {
    /// The config file that was read.
    pub path: PathBuf,
    /// The rows, or `None` when the file has no `services` array.
    pub rows: Option<Vec<ServiceRow>>,
}

/// Whether `config show` reveals secrets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disclosure {
    /// Replace `secret_key` and `relay_token` values with a placeholder.
    Redacted,
    /// Show the text as stored.
    Reveal,
}

/// Config file text for display. `Debug` never prints the text, which may
/// hold secrets (RS-10).
pub struct ShownConfig(pub(super) String);

impl ShownConfig {
    /// The text to display.
    pub fn text(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for ShownConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ShownConfig([redacted])")
    }
}

/// Why an edit was not applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditRejection {
    /// The live file changed after the scratch copy was taken.
    Conflict,
    /// The edited text failed parsing or validation.
    Invalid,
}

/// Result of [`EditAttempt::commit`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditOutcome {
    /// The edit equals the original; nothing was written.
    Unchanged,
    /// The edit was valid and replaced the config.
    Saved {
        /// The config file that was replaced.
        path: PathBuf,
    },
    /// The edit was not applied; the live file is unchanged.
    Rejected {
        /// Why the edit was rejected.
        reason: EditRejection,
        /// `<config>.rejected` when the edited text was kept there.
        sidecar: Option<PathBuf>,
        /// User-facing explanation, naming where the text was kept.
        message: String,
    },
}

/// A private scratch copy of the config, ready to be edited by any editor.
///
/// Dropping it removes the scratch file. No lock is held while it exists.
pub struct EditAttempt {
    pub(super) role: Role,
    pub(super) path: PathBuf,
    pub(super) scratch: PathBuf,
    pub(super) original: Option<String>,
}

impl std::fmt::Debug for EditAttempt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EditAttempt")
            .field("role", &self.role)
            .field("path", &self.path)
            .field("scratch", &self.scratch)
            .field("original", &"<redacted>")
            .finish()
    }
}

impl Drop for EditAttempt {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.scratch);
    }
}
