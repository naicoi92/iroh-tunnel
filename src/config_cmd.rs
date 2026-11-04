//! Handlers for the `config` subcommand: keygen / add / remove / list / show /
//! edit / path.
//!
//! Implements T-11. Each action edits the role's TOML config file in place with
//! [`toml_edit`] so existing formatting and comments survive. Validation reuses
//! the schema's own `validate()` (via [`config::ServeConfig`] /
//! [`config::AccessConfig`]); a validation failure is surfaced as a
//! [`CliError::Config`] so it maps to exit code 2 (Page 06 v5 §6).
//!
//! ## Path resolution
//!
//! `--config` wins; otherwise [`crate::paths::default_config_path`]
//! (`~/.config/iroh-tunnel/{role}.toml`, LocalAppData on Windows, with macOS
//! legacy migration) — the same logic `main.rs` uses for `run`.
//!
//! Based on Page 06 v5 §1.2 (config subcommand) and §1.5 (config actions).

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use toml_edit::DocumentMut;

use crate::cli::AddServiceArgs;
use crate::config_store::{
    ensure_node_table, existing_secret_key, parse_doc, read_optional, resolve_target,
    strip_retired_fields, write_atomic, ConfigLock,
};
use crate::error::CliError;
use crate::render;
pub use crate::status::Role;

/// Placeholder printed by `config show` instead of a secret.
const REDACTED: &str = "<redacted>";

/// `[node]` keys `config show` hides unless `--reveal` is given.
const SECRET_FIELDS: [&str; 2] = ["secret_key", "relay_token"];

/// Whether the role's config already holds a `[node].secret_key`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityState {
    NoKey,
    HasKey,
}

/// Whether the caller agreed to replace an existing key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotationConsent {
    NotGiven,
    Given,
}

/// Result of [`keygen`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeygenOutcome {
    /// A new key was written; `rotated` is true when it replaced an old one.
    Generated {
        path: PathBuf,
        node_id: String,
        rotated: bool,
    },
    /// A key exists and consent was not given; the file is untouched.
    /// `current_node_id` is `None` when the stored key cannot be decoded.
    ConsentNeeded {
        path: PathBuf,
        current_node_id: Option<String>,
    },
}

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

/// CLI wrapper: prompt (or refuse without a terminal) before rotating, then
/// print the result.
pub fn keygen_cli(role: Role, config: Option<&Path>, yes: bool) -> Result<()> {
    let mut consent = if yes {
        RotationConsent::Given
    } else {
        RotationConsent::NotGiven
    };
    // Ask before taking the lock so a slow answer does not stall other
    // writers; the decision is re-checked under the lock in `keygen`.
    if !yes && identity_state(role, config)? == IdentityState::HasKey {
        let path = resolve_target(&resolve_config_path(role, config)?);
        confirm_rotation(&path)?;
        consent = RotationConsent::Given;
    }
    let outcome = keygen(role, config, consent)?;
    if let KeygenOutcome::ConsentNeeded { path, .. } = &outcome {
        return Err(rotation_refused(path));
    }
    print!("{}", render::render_keygen(&outcome));
    Ok(())
}

fn rotation_refused(path: &Path) -> anyhow::Error {
    CliError::Config(format!(
        "{} already holds a secret_key; replacing it changes this node's NodeId. \
         Re-run with --yes to rotate it",
        path.display()
    ))
    .into()
}

/// Ask the user to confirm replacing the key in `path`.
///
/// # Errors
///
/// [`CliError::Config`] when no terminal is available or the answer is not
/// yes; the key is left unchanged either way.
fn confirm_rotation(path: &Path) -> Result<()> {
    if !crate::service::scope::is_interactive() {
        return Err(rotation_refused(path));
    }
    eprint!(
        "{} already holds a secret_key. Replacing it changes this node's NodeId and breaks \
         peers that pin it. Replace it? [y/N] ",
        path.display()
    );
    std::io::stderr().flush().ok();
    let mut answer = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut answer)
        .context("failed to read the confirmation")?;
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        Err(CliError::Config("key rotation cancelled; the secret_key is unchanged".into()).into())
    }
}

/// A freshly generated identity from `config keygen random`.
///
/// The secret is reachable only through [`GeneratedIdentity::secret`]; `Debug`
/// never prints it (RS-10).
pub struct GeneratedIdentity {
    secret: String,
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

/// Generate a fresh key and its node ID without touching any configuration
/// file or the default path. Nothing is logged.
pub fn random_identity() -> GeneratedIdentity {
    let key = iroh::SecretKey::generate();
    GeneratedIdentity {
        secret: crate::config::encode_secret_key(&key),
        node_id: key.public().to_string(),
    }
}

/// `config keygen random`: print the identity. The secret goes to stdout only,
/// so it can be captured (`$(…)`, pipes).
pub fn keygen_random() -> Result<()> {
    print!("{}", render::render_identity(&random_identity()));
    Ok(())
}

/// Request to append one `[[services]]` entry. Fields are raw strings so the
/// schema validation (and its messages) run at the same stage as before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddService {
    pub config: Option<PathBuf>,
    pub name: String,
    pub protocol: String,
    pub host: String,
    pub port: u16,
    /// Required only for the access role.
    pub node_id: Option<String>,
}

impl From<&AddServiceArgs> for AddService {
    fn from(args: &AddServiceArgs) -> Self {
        Self {
            config: args.config.clone(),
            name: args.name.clone(),
            protocol: args.protocol.clone(),
            host: args.host.clone(),
            port: args.port,
            node_id: args.node_id.clone(),
        }
    }
}

/// Outcome of a service add/remove: the service name and the file changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mutation {
    pub service: String,
    pub path: PathBuf,
}

/// Append a `[[services]]` entry.
///
/// Validates the new service (name regex/length, port range, node_id for
/// access, duplicate-name check) by re-loading the mutated document through the
/// schema. A duplicate or invalid entry is a config error → exit 2.
pub fn add_service(role: Role, req: &AddService) -> Result<Mutation> {
    let path = resolve_target(&resolve_config_path(role, req.config.as_deref())?);
    mutate(role, &path, |doc| {
        // Reject a duplicate name up front with a clear message.
        if find_service(doc, &req.name).is_some() {
            return Err(CliError::Config(format!(
                "a service named '{}' already exists in {}",
                req.name,
                path.display()
            ))
            .into());
        }

        let mut entry = toml_edit::Table::new();
        entry["name"] = toml_edit::value(req.name.as_str());
        entry["protocol"] = toml_edit::value(req.protocol.as_str());
        entry["host"] = toml_edit::value(req.host.as_str());
        entry["port"] = toml_edit::value(i64::from(req.port));
        if role == Role::Access {
            let node_id = req.node_id.as_deref().ok_or_else(|| {
                CliError::Config("--node-id is required when adding an access service".into())
            })?;
            entry["node_id"] = toml_edit::value(node_id);
        }

        // Append as an array-of-tables element: [[services]].
        doc["services"].or_insert(toml_edit::Item::ArrayOfTables(Default::default()));
        let services = doc["services"].as_array_of_tables_mut().ok_or_else(|| {
            CliError::Config(format!(
                "invalid config at {}: [services] must be an array of tables",
                path.display()
            ))
        })?;
        services.push(entry);
        Ok(())
    })?;
    Ok(Mutation {
        service: req.name.clone(),
        path,
    })
}

/// `config add`: add the service and print the confirmation.
pub fn add(role: Role, args: &AddServiceArgs) -> Result<()> {
    let outcome = add_service(role, &AddService::from(args))?;
    print!("{}", render::render_added(&outcome));
    Ok(())
}

/// Delete the `[[services]]` entry whose `name` matches.
pub fn remove_service(role: Role, config: Option<&Path>, name: &str) -> Result<Mutation> {
    let path = resolve_target(&resolve_config_path(role, config)?);
    // Nothing to remove from a missing config: fail without creating files.
    require_existing(role, &path)?;
    mutate(role, &path, |doc| {
        // No `[services]` table at all → treat as "not found" (config error,
        // exit 2) rather than a generic internal error.
        let not_found =
            || CliError::Config(format!("no service named '{name}' in {}", path.display()));
        let idx = doc
            .get("services")
            .and_then(|i| i.as_array_of_tables())
            .ok_or_else(not_found)?
            .iter()
            .position(|t| t.get("name").and_then(|v| v.as_str()) == Some(name))
            .ok_or_else(not_found)?;
        doc["services"]
            .as_array_of_tables_mut()
            .context("internal: [services] is not an array-of-tables")?
            .remove(idx);
        Ok(())
    })?;
    Ok(Mutation {
        service: name.to_owned(),
        path,
    })
}

/// `config remove`: remove the service and print the confirmation.
pub fn remove(role: Role, config: Option<&Path>, name: &str) -> Result<()> {
    let outcome = remove_service(role, config, name)?;
    print!("{}", render::render_removed(&outcome));
    Ok(())
}

/// One `[[services]]` entry as listed; a missing or mistyped field is `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceRow {
    pub name: Option<String>,
    pub node_id: Option<String>,
    pub protocol: Option<String>,
    pub host: Option<String>,
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
    pub path: PathBuf,
    pub rows: Option<Vec<ServiceRow>>,
}

/// Read the configured services. The config must exist ([`require_existing`]).
pub fn list_services(role: Role, config: Option<&Path>) -> Result<ServiceList> {
    let path = resolve_config_path(role, config)?;
    let doc = parse_doc(&require_existing(role, &path)?, &path)?;
    let rows = doc
        .get("services")
        .and_then(|i| i.as_array_of_tables())
        .map(|services| {
            let text = |t: &toml_edit::Table, key: &str| {
                t.get(key).and_then(|v| v.as_str()).map(str::to_owned)
            };
            services
                .iter()
                .map(|t| ServiceRow {
                    name: text(t, "name"),
                    node_id: text(t, "node_id"),
                    protocol: text(t, "protocol"),
                    host: text(t, "host"),
                    port: t.get("port").and_then(|v| v.as_integer()),
                })
                .collect()
        });
    Ok(ServiceList { path, rows })
}

/// `config list`: print a compact table of the configured services.
pub fn list(role: Role, config: Option<&Path>) -> Result<()> {
    let list = list_services(role, config)?;
    print!("{}", render::render_service_list(role, &list));
    Ok(())
}

/// Whether `config show` reveals secrets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disclosure {
    Redacted,
    Reveal,
}

/// Config file text for display. `Debug` never prints the text, which may
/// hold secrets (RS-10).
pub struct ShownConfig(String);

impl ShownConfig {
    pub fn text(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for ShownConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ShownConfig([redacted])")
    }
}

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

/// `config show`: print the config, redacted unless `reveal`.
pub fn show(role: Role, config: Option<&Path>, reveal: bool) -> Result<()> {
    let disclosure = if reveal {
        Disclosure::Reveal
    } else {
        Disclosure::Redacted
    };
    print!("{}", show_config(role, config, disclosure)?.text());
    Ok(())
}

/// Replace the value of every [`SECRET_FIELDS`] key in `[node]`, keeping the
/// surrounding whitespace and comments.
fn redact_secrets(doc: &mut DocumentMut) {
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

/// `config edit`: open the config in `$EDITOR`, then validate and save it.
///
/// The editor works on a private copy next to the config. The result is
/// applied only if the live file is unchanged since the copy was taken and the
/// edited document is valid; otherwise the live file is left as it was and the
/// edited text is kept in `<config>.rejected` (owner-only) so it is not lost.
pub fn edit(role: Role, config: Option<&Path>) -> Result<()> {
    let path = resolve_target(&resolve_config_path(role, config)?);
    let original = read_optional(&path)?;
    let original_text = original.clone().unwrap_or_default();

    crate::config_store::ensure_parent(&path)?;
    let scratch = scratch_path(&path);
    write_atomic(&scratch, original_text.as_bytes())?;
    let edited = edit::edit_file(&scratch)
        .map_err(anyhow::Error::from)
        .and_then(|()| {
            std::fs::read_to_string(&scratch)
                .with_context(|| format!("failed to read edited copy: {}", scratch.display()))
        });
    let _ = std::fs::remove_file(&scratch);
    let edited = edited?;

    if edited == original_text {
        println!("No changes.");
        return Ok(());
    }

    let _lock = ConfigLock::acquire(&path)?;
    if read_optional(&path)? != original {
        let kept = keep_rejected(&path, &edited);
        return Err(CliError::Config(format!(
            "{} changed while it was being edited; your edit was not saved{kept}. \
             Re-apply it to the current file",
            path.display()
        ))
        .into());
    }
    let checked = parse_doc(&edited, &path).and_then(|mut doc| {
        strip_retired_fields(&mut doc);
        validate_role_doc(role, &doc.to_string(), &path)?;
        Ok(doc)
    });
    match checked {
        Ok(doc) => {
            write_atomic(&path, doc.to_string().as_bytes())?;
            println!("Saved {}.", path.display());
            Ok(())
        }
        Err(e) => {
            let kept = keep_rejected(&path, &edited);
            Err(CliError::Config(format!("{e:#}; the file was not changed{kept}")).into())
        }
    }
}

/// Private copy of the config handed to the editor.
fn scratch_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".edit-{}.toml", std::process::id()));
    path.with_file_name(name)
}

/// Keep the rejected edit in `<config>.rejected` and describe where, for the
/// error message. A failure to keep it is reported instead of hidden.
fn keep_rejected(path: &Path, edited: &str) -> String {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".rejected");
    let sidecar = path.with_file_name(name);
    match write_atomic(&sidecar, edited.as_bytes()) {
        Ok(()) => format!("; your text is in {}", sidecar.display()),
        Err(e) => format!("; could not keep your text ({e:#})"),
    }
}

/// `config path`: print the resolved config file path.
pub fn path(role: Role, config: Option<&Path>) -> Result<()> {
    let path = resolve_config_path(role, config)?;
    print!("{}", render::render_path(&path));
    Ok(())
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// Resolve a role configuration path. An explicit path takes precedence over
/// the platform default. With `test-utils`, an absolute test config root may
/// replace the platform root; a relative override is rejected.
///
/// Returns an error when the platform root is unavailable or the test root is
/// not absolute. Production builds resolve defaults with [`crate::paths`].
pub fn resolve_config_path(role: Role, override_path: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = override_path {
        return Ok(path.to_path_buf());
    }
    #[cfg(feature = "test-utils")]
    if let Some(dir) = std::env::var_os("IROH_TUNNEL_TEST_CONFIG_DIR") {
        let dir = std::path::PathBuf::from(dir);
        anyhow::ensure!(dir.is_absolute(), "test config directory must be absolute");
        return Ok(dir
            .join("iroh-tunnel")
            .join(format!("{}.toml", role.name())));
    }
    crate::paths::default_config_path(role.name())
}

/// Read `path`, which a read command needs to exist (D3).
///
/// # Errors
///
/// [`CliError::Config`] with guidance naming the path and the commands that
/// create a configuration. Nothing is created and no identity is generated.
pub fn require_existing(role: Role, path: &Path) -> Result<String> {
    read_optional(path)?.ok_or_else(|| {
        CliError::Config(format!(
            "no configuration at {path}. Create one with `config add` or `config keygen` \
             (`iroh-tunnel {role} config add`, or `iroh-tunnel-{role} config add`), or pass --config",
            path = path.display(),
            role = role.name()
        ))
        .into()
    })
}

/// Apply `change` to the config at `path` as one validated, atomic, serialized
/// mutation (CFG-04).
///
/// Takes the writer lock, loads the document (empty when the file does not
/// exist yet), applies `change`, drops retired fields, validates the result
/// through the role schema and replaces the file atomically and owner-only.
/// If `change` or validation fails the file is left byte-for-byte unchanged.
fn mutate(
    role: Role,
    path: &Path,
    change: impl FnOnce(&mut DocumentMut) -> Result<()>,
) -> Result<()> {
    mutate_deciding(role, path, |doc| change(doc).map(|()| Edit::Apply(())))
}

/// What a [`mutate_deciding`] closure decided about the document.
enum Edit<T> {
    /// Validate and write the edited document.
    Apply(T),
    /// Leave the file untouched.
    Keep(T),
}

/// [`mutate`] where the closure may decide, under the lock, to write nothing.
fn mutate_deciding<T>(
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

/// Return the first `[[services]]` table whose `name` matches, if any.
/// (Used only for the duplicate check in `add`.)
fn find_service<'a>(doc: &'a DocumentMut, name: &str) -> Option<&'a toml_edit::Table> {
    doc.get("services")
        .and_then(|i| i.as_array_of_tables())?
        .iter()
        .find(|t| t.get("name").and_then(|v| v.as_str()) == Some(name))
}

/// Re-parse `content` through the role's schema to enforce all validation rules
/// (name regex/length, port range, node_id length, duplicate names, https
/// relay_urls). A failure is a `CliError::Config` → exit 2.
fn validate_role_doc(role: Role, content: &str, path: &Path) -> Result<()> {
    let invalid = |detail: String| -> anyhow::Error {
        CliError::Config(format!("invalid config at {}{detail}", path.display())).into()
    };
    // `toml` errors quote the offending source line, which can hold a secret;
    // report the message and line only (RS-10).
    let parse_failure = |e: toml::de::Error| {
        invalid(format!(
            "{}: {}",
            crate::config_store::describe_location(content, e.span()),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn add_web(config: &Path) -> AddService {
        AddService {
            config: Some(config.to_path_buf()),
            name: "web".into(),
            protocol: "tcp".into(),
            host: "127.0.0.1".into(),
            port: 8080,
            node_id: None,
        }
    }

    #[test]
    fn add_service_returns_name_and_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.toml");
        let outcome = add_service(Role::Serve, &add_web(&path)).unwrap();
        assert_eq!(
            outcome,
            Mutation {
                service: "web".into(),
                path: path.clone()
            }
        );
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("name = \"web\""));
    }

    #[test]
    fn invalid_access_add_leaves_bytes_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("access.toml");
        let before = "# keep me\n";
        std::fs::write(&path, before).unwrap();
        let err = add_service(Role::Access, &add_web(&path)).unwrap_err();
        assert!(matches!(
            err.downcast_ref::<CliError>(),
            Some(CliError::Config(_))
        ));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn list_services_returns_rows_without_terminal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.toml");
        add_service(Role::Serve, &add_web(&path)).unwrap();
        let list = list_services(Role::Serve, Some(&path)).unwrap();
        assert_eq!(list.path, path);
        assert_eq!(
            list.rows,
            Some(vec![ServiceRow {
                name: Some("web".into()),
                node_id: None,
                protocol: Some("tcp".into()),
                host: Some("127.0.0.1".into()),
                port: Some(8080),
            }])
        );

        let empty = dir.path().join("empty.toml");
        std::fs::write(&empty, "# nothing\n").unwrap();
        assert_eq!(list_services(Role::Serve, Some(&empty)).unwrap().rows, None);
    }

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

    /// `random_identity` takes no path and no environment input, so there is
    /// nothing to point it at; setting `HOME` would race other threads of the
    /// test binary. Instead assert the identity is self-consistent and that a
    /// tempdir this test controls stays empty across the call.
    #[test]
    fn random_identity_touches_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let identity = random_identity();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        let (key, _) = crate::config::resolve_secret_key(identity.secret()).unwrap();
        assert_eq!(identity.node_id, key.public().to_string());
        assert!(!format!("{identity:?}").contains(identity.secret()));
    }

    const KEYED: &str = "[node]\nsecret_key = \"\"\n";

    fn write_keyed(path: &Path) -> String {
        let key = iroh::SecretKey::generate();
        let text = format!(
            "[node]\nsecret_key = \"{}\"\n",
            crate::config::encode_secret_key(&key)
        );
        std::fs::write(path, &text).unwrap();
        key.public().to_string()
    }

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

    #[cfg(unix)]
    #[test]
    fn add_service_accepts_non_utf8_config_path() {
        use std::os::unix::ffi::OsStrExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join(std::ffi::OsStr::from_bytes(b"serve-\xff.toml"));
        // APFS (macOS) rejects non-UTF-8 names outright; there is nothing to
        // test on a filesystem that cannot hold such a path.
        if std::fs::write(&path, "").is_err() {
            return;
        }
        std::fs::remove_file(&path).unwrap();
        let outcome = add_service(Role::Serve, &add_web(&path)).unwrap();
        assert_eq!(outcome.path, path);
        assert!(path.exists());
    }
}
