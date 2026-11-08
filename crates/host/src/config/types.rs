//! Request, outcome and handle types of the config operations.

use std::path::PathBuf;

use super::Protocol;
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

/// A freshly generated identity from `config keygen --print`.
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

/// Outcome of a service removal: the service name and the file changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mutation {
    /// Name of the removed service.
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

/// Request to create or update one `[[services]]` entry by name.
///
/// `None` means "not passed". `host` is used only when `port` is `Some`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetService {
    /// Config file; `None` selects the role default.
    pub config: Option<PathBuf>,
    /// Service name.
    pub name: String,
    /// Target (serve) or listen (access) host.
    pub host: Option<String>,
    /// Target (serve) or listen (access) port.
    pub port: Option<u16>,
    /// Transport protocol.
    pub protocol: Option<Protocol>,
    /// Peer node id (access role).
    pub node_id: Option<String>,
}

/// Plain inputs of a quick-mode serve run, for [`quick_serve_config`].
///
/// [`quick_serve_config`]: super::quick_serve_config
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickServe {
    /// Service name.
    pub name: String,
    /// Transport protocol.
    pub protocol: Protocol,
    /// Target host; `None` means `127.0.0.1`.
    pub host: Option<String>,
    /// Target port.
    pub port: u16,
    /// Relay URL used by both peers; `None` uses the default relays.
    pub relay_url: Option<String>,
    /// File holding a persistent secret key; `None` means an ephemeral identity.
    pub key_file: Option<PathBuf>,
}

/// Plain inputs of a quick-mode access run, for [`quick_access_config`].
///
/// [`quick_access_config`]: super::quick_access_config
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickAccess {
    /// Service name (must match the serve side).
    pub name: String,
    /// Transport protocol.
    pub protocol: Protocol,
    /// Listen host; `None` means `127.0.0.1`.
    pub host: Option<String>,
    /// Listen port.
    pub port: u16,
    /// Peer node id, as typed; validated by the builder.
    pub node_id: String,
    /// Relay URL used by both peers; `None` uses the default relays.
    pub relay_url: Option<String>,
    /// File holding a persistent secret key; `None` means an ephemeral identity.
    pub key_file: Option<PathBuf>,
}

/// Result of `set_service`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetOutcome {
    /// A new entry was appended.
    Created {
        /// Service name.
        service: String,
        /// Config file that was written.
        path: PathBuf,
    },
    /// An existing entry changed.
    Updated {
        /// Service name.
        service: String,
        /// Config file that was written.
        path: PathBuf,
        /// Changed fields in the order address, protocol, node_id.
        changes: Vec<FieldChange>,
    },
    /// The entry already had every requested value; the file is untouched.
    Unchanged {
        /// Service name.
        service: String,
        /// Config file that was inspected.
        path: PathBuf,
    },
}

/// One changed field of an updated service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldChange {
    /// The field that changed.
    pub field: ServiceField,
    /// Previous value; addresses are rendered by `display_addr`.
    pub old: String,
    /// New value; addresses are rendered by `display_addr`.
    pub new: String,
}

/// A field of a `[[services]]` entry that `set` can change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceField {
    /// The target (serve) or listen (access) address.
    Address,
    /// The transport protocol.
    Protocol,
    /// The peer node id.
    NodeId,
}

impl ServiceField {
    /// The label shown in the change report for `role`.
    pub fn label(self, role: Role) -> &'static str {
        match (self, role) {
            (ServiceField::Address, Role::Serve) => "target",
            (ServiceField::Address, Role::Access) => "listen",
            (ServiceField::Protocol, _) => "protocol",
            (ServiceField::NodeId, _) => "node_id",
        }
    }
}

/// Typed failures of `set_service`, returned inside `anyhow::Error`.
#[derive(thiserror::Error, Debug)]
pub enum SetError {
    /// A create request lacks required flags.
    #[error("creating service '{name}' requires {}", .flags.join(" and "))]
    MissingForCreate {
        /// Service name.
        name: String,
        /// Missing flags, e.g. `--target`, then `--node-id`.
        flags: Vec<&'static str>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_for_create_names_flags() {
        let e = SetError::MissingForCreate {
            name: "web".into(),
            flags: vec!["--listen", "--node-id"],
        };
        assert_eq!(
            e.to_string(),
            "creating service 'web' requires --listen and --node-id"
        );
    }

    #[test]
    fn service_field_labels() {
        assert_eq!(ServiceField::Address.label(Role::Serve), "target");
        assert_eq!(ServiceField::Address.label(Role::Access), "listen");
        assert_eq!(ServiceField::Protocol.label(Role::Serve), "protocol");
        assert_eq!(ServiceField::Protocol.label(Role::Access), "protocol");
        assert_eq!(ServiceField::NodeId.label(Role::Serve), "node_id");
        assert_eq!(ServiceField::NodeId.label(Role::Access), "node_id");
    }
}
