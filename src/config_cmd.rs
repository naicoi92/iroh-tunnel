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

/// Placeholder printed by `config show` instead of a secret.
const REDACTED: &str = "<redacted>";

/// `[node]` keys `config show` hides unless `--reveal` is given.
const SECRET_FIELDS: [&str; 2] = ["secret_key", "relay_token"];

/// `config keygen`: generate a new SecretKey, write it into `[node].secret_key`.
///
/// Works for both roles: serve pins the node identity that access peers dial,
/// and access pins its own identity so the serve side sees a stable peer. Either
/// way, downstream configs that reference this node's NodeId must be updated to
/// match the newly generated key.
///
/// Replacing an existing key changes the NodeId, so it needs confirmation
/// (D1): `yes` skips the prompt; without a terminal and without `yes` the
/// command fails and the key is kept. A first key needs no confirmation.
pub fn keygen(role: &str, config: Option<&Path>, yes: bool) -> Result<()> {
    let path = resolve_target(&resolve_config_path(role, config)?);
    let mut confirmed = yes;
    // Ask before taking the lock so a slow answer does not stall other
    // writers; the decision is re-checked under the lock below.
    if !confirmed
        && read_optional(&path)?
            .and_then(|text| text.parse::<DocumentMut>().ok())
            .is_some_and(|doc| existing_secret_key(&doc).is_some())
    {
        confirm_rotation(&path)?;
        confirmed = true;
    }

    let key = iroh::SecretKey::generate();
    let key_str = crate::config::encode_secret_key(&key);
    mutate(role, &path, |doc| {
        if !confirmed && existing_secret_key(doc).is_some() {
            return Err(rotation_refused(&path));
        }
        ensure_node_table(doc, &path)?;
        doc["node"]["secret_key"] = toml_edit::value(key_str.as_str());
        Ok(())
    })?;

    println!("Generated new secret_key.");
    println!("New NodeId: {}", key.public());
    println!("WARNING: configs that reference this node's node_id must be updated to match.");
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

/// `config keygen random`: print a fresh key and its node ID without touching
/// any configuration file or the default path.
///
/// The secret goes to stdout only, so it can be captured (`$(…)`, pipes). It is
/// never logged.
pub fn keygen_random() -> Result<()> {
    let key = iroh::SecretKey::generate();
    println!("Secret key: {}", crate::config::encode_secret_key(&key));
    println!("Node ID: {}", key.public());
    Ok(())
}

/// `config add`: append a `[[services]]` entry.
///
/// Validates the new service (name regex/length, port range, node_id for
/// access, duplicate-name check) by re-loading the mutated document through the
/// schema. A duplicate or invalid entry is a config error → exit 2.
pub fn add(role: &str, args: &AddServiceArgs) -> Result<()> {
    let path = resolve_target(&resolve_config_path(role, args.config.as_deref())?);
    mutate(role, &path, |doc| {
        // Reject a duplicate name up front with a clear message.
        if find_service(doc, &args.name).is_some() {
            return Err(CliError::Config(format!(
                "a service named '{}' already exists in {}",
                args.name,
                path.display()
            ))
            .into());
        }

        let mut entry = toml_edit::Table::new();
        entry["name"] = toml_edit::value(args.name.as_str());
        entry["protocol"] = toml_edit::value(args.protocol.as_str());
        entry["host"] = toml_edit::value(args.host.as_str());
        entry["port"] = toml_edit::value(i64::from(args.port));
        if role == "access" {
            let node_id = args.node_id.as_deref().ok_or_else(|| {
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
    println!("Added service '{}' to {}.", args.name, path.display());
    Ok(())
}

/// `config remove`: delete the `[[services]]` entry whose `name` matches.
pub fn remove(role: &str, config: Option<&Path>, name: &str) -> Result<()> {
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
    println!("Removed service '{name}' from {}.", path.display());
    Ok(())
}

/// `config list`: print a compact table of the configured services.
pub fn list(role: &str, config: Option<&Path>) -> Result<()> {
    let path = resolve_config_path(role, config)?;
    let doc = parse_doc(&require_existing(role, &path)?, &path)?;
    let Some(services) = doc.get("services").and_then(|i| i.as_array_of_tables()) else {
        println!("No services configured ({}).", path.display());
        return Ok(());
    };

    if role == "access" {
        println!(
            "{:<24} {:<14} {:<22} {:<8}",
            "NAME", "NODE_ID", "PROTO", "PORT"
        );
        for t in services {
            println!(
                "{:<24} {:<14} {:<22} {:<8}",
                t.get("name").and_then(|v| v.as_str()).unwrap_or("?"),
                t.get("node_id").and_then(|v| v.as_str()).unwrap_or("?"),
                t.get("protocol").and_then(|v| v.as_str()).unwrap_or("?"),
                t.get("port").and_then(|v| v.as_integer()).unwrap_or(0),
            );
        }
    } else {
        println!(
            "{:<24} {:<22} {:<18} {:<8}",
            "NAME", "PROTO", "HOST", "PORT"
        );
        for t in services {
            println!(
                "{:<24} {:<22} {:<18} {:<8}",
                t.get("name").and_then(|v| v.as_str()).unwrap_or("?"),
                t.get("protocol").and_then(|v| v.as_str()).unwrap_or("?"),
                t.get("host").and_then(|v| v.as_str()).unwrap_or("?"),
                t.get("port").and_then(|v| v.as_integer()).unwrap_or(0),
            );
        }
    }
    Ok(())
}

/// `config show`: print the config file contents with `secret_key` and
/// `relay_token` replaced by a placeholder, unless `reveal` is set (D2).
/// Comments and formatting are kept; the file itself is never changed.
pub fn show(role: &str, config: Option<&Path>, reveal: bool) -> Result<()> {
    let path = resolve_config_path(role, config)?;
    let text = require_existing(role, &path)?;
    if reveal {
        print!("{text}");
        return Ok(());
    }
    // A document that does not parse cannot be redacted, so it is never echoed.
    let mut doc = parse_doc(&text, &path)?;
    redact_secrets(&mut doc);
    print!("{doc}");
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
pub fn edit(role: &str, config: Option<&Path>) -> Result<()> {
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
pub fn path(role: &str, config: Option<&Path>) -> Result<()> {
    let path = resolve_config_path(role, config)?;
    println!("{}", path.display());
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
pub fn resolve_config_path(role: &str, override_path: Option<&Path>) -> Result<std::path::PathBuf> {
    if let Some(path) = override_path {
        return Ok(path.to_path_buf());
    }
    #[cfg(feature = "test-utils")]
    if let Some(dir) = std::env::var_os("IROH_TUNNEL_TEST_CONFIG_DIR") {
        let dir = std::path::PathBuf::from(dir);
        anyhow::ensure!(dir.is_absolute(), "test config directory must be absolute");
        return Ok(dir.join("iroh-tunnel").join(format!("{role}.toml")));
    }
    crate::paths::default_config_path(role)
}

/// Read `path`, which a read command needs to exist (D3).
///
/// # Errors
///
/// [`CliError::Config`] with guidance naming the path and the commands that
/// create a configuration. Nothing is created and no identity is generated.
pub fn require_existing(role: &str, path: &Path) -> Result<String> {
    read_optional(path)?.ok_or_else(|| {
        CliError::Config(format!(
            "no configuration at {path}. Create one with `iroh-tunnel {role} config add` or \
             `iroh-tunnel {role} config keygen`, or pass --config",
            path = path.display()
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
    role: &str,
    path: &Path,
    change: impl FnOnce(&mut DocumentMut) -> Result<()>,
) -> Result<()> {
    let _lock = ConfigLock::acquire(path)?;
    let text = read_optional(path)?.unwrap_or_default();
    let mut doc = parse_doc(&text, path)?;
    change(&mut doc)?;
    strip_retired_fields(&mut doc);
    let rendered = doc.to_string();
    validate_role_doc(role, &rendered, path)?;
    write_atomic(path, rendered.as_bytes())
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
fn validate_role_doc(role: &str, content: &str, path: &Path) -> Result<()> {
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
        "access" => toml::from_str::<crate::config::AccessConfig>(content)
            .map_err(parse_failure)?
            .validate(),
        _ => toml::from_str::<crate::config::ServeConfig>(content)
            .map_err(parse_failure)?
            .validate(),
    }
    .map_err(|e| invalid(format!(": {e}")))
}
