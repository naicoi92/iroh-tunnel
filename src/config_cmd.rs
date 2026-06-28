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
//! `--config` wins; otherwise the OS-specific config dir
//! (`dirs::config_dir()/iroh-tunnel/{role}.toml`) — the same logic `main.rs`
//! uses for `run`, kept consistent across the CLI. (The spec sample used
//! `~/.config/...`, which is Linux-only; `dirs::config_dir()` is the
//! cross-platform equivalent already standardised by T-01's dispatch.)
//!
//! Based on Page 06 v5 §1.2 (config subcommand) and §1.5 (config actions).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use toml_edit::DocumentMut;

use crate::cli::AddServiceArgs;
use crate::error::CliError;

/// `config keygen`: generate a new SecretKey, write it into `[node].secret_key`.
///
/// No-op (with a warning) for the access role, which uses an ephemeral key.
pub fn keygen(role: &str, config: Option<&Path>) -> Result<()> {
    if role == "access" {
        println!("WARNING: access role uses an ephemeral key; keygen is not applicable.");
        return Ok(());
    }
    let path = resolve_config_path(role, config)?;
    let key = iroh::SecretKey::generate();
    let key_str = crate::config::encode_secret_key(&key);

    let mut doc = load_or_empty(&path)?;
    ensure_node_table(&mut doc);
    doc["node"]["secret_key"] = toml_edit::value(key_str);
    write_doc(&path, &doc)?;

    println!("Generated new secret_key.");
    println!("New NodeId: {}", key.public());
    println!("WARNING: access-side configs must update their node_id to match.");
    Ok(())
}

/// `config add`: append a `[[services]]` entry.
///
/// Validates the new service (name regex/length, port range, node_id for
/// access, duplicate-name check) by re-loading the mutated document through the
/// schema. A duplicate or invalid entry is a config error → exit 2.
pub fn add(role: &str, args: &AddServiceArgs) -> Result<()> {
    let path = resolve_config_path(role, args.config.as_deref())?;
    let mut doc = load_or_empty(&path)?;

    // Reject a duplicate name up front with a clear message.
    if find_service(&doc, &args.name).is_some() {
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
    let services = doc["services"]
        .as_array_of_tables_mut()
        .context("internal: [services] is not an array-of-tables")?;
    services.push(entry);

    // Validate the whole document via the schema before persisting.
    validate_role_doc(role, &doc.to_string(), &path)?;
    write_doc(&path, &doc)?;
    println!("Added service '{}' to {}.", args.name, path.display());
    Ok(())
}

/// `config remove`: delete the `[[services]]` entry whose `name` matches.
pub fn remove(role: &str, config: Option<&Path>, name: &str) -> Result<()> {
    let path = resolve_config_path(role, config)?;
    let mut doc = load_or_empty(&path)?;

    // No `[services]` table at all → treat as "not found" (config error, exit 2)
    // rather than a generic internal error.
    let idx = doc
        .get("services")
        .and_then(|i| i.as_array_of_tables())
        .ok_or_else(|| {
            CliError::Config(format!("no service named '{name}' in {}", path.display()))
        })?
        .iter()
        .position(|t| t.get("name").and_then(|v| v.as_str()) == Some(name))
        .ok_or_else(|| {
            CliError::Config(format!("no service named '{name}' in {}", path.display()))
        })?;

    doc["services"]
        .as_array_of_tables_mut()
        .context("internal: [services] is not an array-of-tables")?
        .remove(idx);

    write_doc(&path, &doc)?;
    println!("Removed service '{name}' from {}.", path.display());
    Ok(())
}

/// `config list`: print a compact table of the configured services.
pub fn list(role: &str, config: Option<&Path>) -> Result<()> {
    let path = resolve_config_path(role, config)?;
    let doc = load_or_empty(&path)?;
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

/// `config show`: pretty-print the raw config file contents.
pub fn show(role: &str, config: Option<&Path>) -> Result<()> {
    let path = resolve_config_path(role, config)?;
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read config: {}", path.display()))?;
    print!("{content}");
    Ok(())
}

/// `config edit`: open the config file in `$EDITOR`.
pub fn edit(role: &str, config: Option<&Path>) -> Result<()> {
    let path = resolve_config_path(role, config)?;
    edit::edit_file(&path)?;
    Ok(())
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

/// Resolve a config path: explicit override wins, otherwise the OS-specific
/// default (`<config_dir>/iroh-tunnel/{role}.toml`).
fn resolve_config_path(role: &str, override_path: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = override_path {
        return Ok(p.to_path_buf());
    }
    let dir = dirs::config_dir().context("could not determine config directory")?;
    Ok(dir.join("iroh-tunnel").join(format!("{role}.toml")))
}

/// Load the file as a `toml_edit` document, or return an empty document if the
/// file does not yet exist (so `add`/`keygen` work on a fresh config).
fn load_or_empty(path: &Path) -> Result<DocumentMut> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(e).with_context(|| format!("failed to read config: {}", path.display()))
        }
    };
    let doc: DocumentMut = content
        .parse()
        .with_context(|| format!("failed to parse config: {}", path.display()))?;
    Ok(doc)
}

/// Ensure a `[node]` table exists (used by `keygen` before setting `secret_key`).
fn ensure_node_table(doc: &mut DocumentMut) {
    if doc.get("node").is_none() {
        doc["node"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
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
    let parsed: Result<(), anyhow::Error> = match role {
        "access" => {
            let cfg: crate::config::AccessConfig = toml::from_str(content)?;
            cfg.validate()
        }
        _ => {
            let cfg: crate::config::ServeConfig = toml::from_str(content)?;
            cfg.validate()
        }
    };
    parsed
        .map_err(|e| CliError::Config(format!("invalid config at {}: {e}", path.display())).into())
}

fn write_doc(path: &Path, doc: &DocumentMut) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create config dir: {}", parent.display()))?;
        }
    }
    std::fs::write(path, doc.to_string())
        .with_context(|| format!("failed to write config: {}", path.display()))?;
    Ok(())
}
