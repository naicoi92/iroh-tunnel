//! Shared persistence for role configuration files (CFG-04, #99 D4/D5).
//!
//! Every writer of a config file — the `config` commands and the key that
//! `run` persists — goes through this module:
//!
//! 1. [`ConfigLock`] serializes writers on an advisory lock held on a sibling
//!    `<file>.lock`. It uses [`std::fs::File::try_lock`], which is `flock` on
//!    Unix and `LockFileEx` on Windows, and waits a bounded time
//!    ([`lock_timeout`]) before reporting the file busy.
//! 2. [`write_atomic`] writes a sibling temp file, fsyncs it, renames it over
//!    the target and fsyncs the directory, so a crash leaves either the old or
//!    the new complete document and never a truncated one.
//!
//! Files holding the node secret are created owner-only (mode `0600`) on Unix.
//! Windows: the temp file inherits the parent directory ACL; an owner-only ACL
//! equivalent is not enforced and is UNVERIFIED (open item of #99).

use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use toml_edit::DocumentMut;

use crate::config::{resolve_secret_key, RoleDoc};
use crate::error::CliError;

/// How long a writer waits for another writer before giving up.
const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_secs(10);
const LOCK_POLL: Duration = Duration::from_millis(25);

#[cfg(feature = "test-utils")]
fn lock_timeout() -> Duration {
    std::env::var("IROH_TUNNEL_TEST_CONFIG_LOCK_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(DEFAULT_LOCK_TIMEOUT, Duration::from_millis)
}

#[cfg(not(feature = "test-utils"))]
fn lock_timeout() -> Duration {
    DEFAULT_LOCK_TIMEOUT
}

/// Follow a symlinked config file so the rename replaces its target, not the
/// link. A path that does not resolve (missing file) is returned unchanged.
pub(crate) fn resolve_target(path: &Path) -> PathBuf {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
        }
        _ => path.to_path_buf(),
    }
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// Create the parent directory of `path` when it is missing.
pub(crate) fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config dir: {}", parent.display()))?;
    }
    Ok(())
}

fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

/// Advisory lock serializing writers of one config file; released on drop.
#[derive(Debug)]
pub(crate) struct ConfigLock {
    _file: File,
}

impl ConfigLock {
    /// Take the lock for `config`, creating the parent directory and the
    /// owner-only `<config>.lock` file when needed.
    ///
    /// # Errors
    ///
    /// Fails when the lock file cannot be opened, or when another process
    /// still holds the lock after [`lock_timeout`].
    pub(crate) fn acquire(config: &Path) -> Result<Self> {
        ensure_parent(config)?;
        let lock_path = sibling(config, ".lock");
        let file = private_options()
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .with_context(|| format!("failed to open config lock: {}", lock_path.display()))?;
        let deadline = Instant::now() + lock_timeout();
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Self { _file: file }),
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(LOCK_POLL);
                }
                Err(TryLockError::WouldBlock) => {
                    return Err(CliError::Config(format!(
                        "another process is writing {}; retry once it finishes",
                        config.display()
                    ))
                    .into())
                }
                Err(TryLockError::Error(e)) => {
                    return Err(e)
                        .with_context(|| format!("failed to lock config: {}", lock_path.display()))
                }
            }
        }
    }
}

/// Replace `path` with `contents` atomically and owner-only. A read-only
/// target is refused.
///
/// Writes a sibling temp file, fsyncs it, renames it over `path` and fsyncs
/// the directory. On failure the temp file is removed and `path` is
/// untouched. Callers hold a [`ConfigLock`].
pub(crate) fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    // A rename would silently replace a file its owner made read-only; keep
    // that safeguard the former in-place write had.
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.permissions().readonly() {
            anyhow::bail!(
                "failed to write config: {} is read-only; make it writable first",
                path.display()
            );
        }
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let tmp = sibling(path, &format!(".{}.{stamp}.tmp", std::process::id()));
    let result = (|| -> Result<()> {
        let mut file = private_options()
            .create_new(true)
            .open(&tmp)
            .with_context(|| format!("failed to create temp config: {}", tmp.display()))?;
        file.write_all(contents)
            .and_then(|()| file.sync_all())
            .with_context(|| format!("failed to write temp config: {}", tmp.display()))?;
        drop(file);
        std::fs::rename(&tmp, path)
            .with_context(|| format!("failed to replace config: {}", path.display()))?;
        // The rename already replaced the file, so a failed directory sync
        // only weakens durability; report success rather than a misleading error.
        #[cfg(unix)]
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            if let Err(e) = File::open(parent).and_then(|dir| dir.sync_all()) {
                tracing::warn!("could not sync config dir {}: {e}", parent.display());
            }
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Read `path` as text; `None` when it does not exist.
pub(crate) fn read_optional(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("failed to read config: {}", path.display())),
    }
}

/// Parse config text into an editable document; a syntax error is a config
/// error (exit 2).
pub(crate) fn parse_doc(text: &str, path: &Path) -> Result<DocumentMut> {
    text.parse::<DocumentMut>().map_err(|e| {
        let at = describe_location(text, e.span());
        CliError::Config(format!(
            "failed to parse config {}{at}: {}",
            path.display(),
            e.message()
        ))
        .into()
    })
}

/// `" (line N)"` for a parse error span. The error's own `Display` quotes the
/// offending source line, which may hold `secret_key` or `relay_token`, so
/// diagnostics carry only the line number and message (RS-10).
pub(crate) fn describe_location(text: &str, span: Option<std::ops::Range<usize>>) -> String {
    span.map(|s| text.get(..s.start).unwrap_or(text))
        .map(|before| format!(" (line {})", before.matches('\n').count() + 1))
        .unwrap_or_default()
}

/// Drop fields retired from the schema (legacy `multiplex`, CFG-03) so every
/// write removes them while keeping comments and formatting.
pub(crate) fn strip_retired_fields(doc: &mut DocumentMut) {
    let Some(services) = doc.get_mut("services") else {
        return;
    };
    if let Some(tables) = services.as_array_of_tables_mut() {
        for service in tables.iter_mut() {
            service.remove("multiplex");
        }
    } else if let Some(array) = services.as_array_mut() {
        for service in array
            .iter_mut()
            .filter_map(|value| value.as_inline_table_mut())
        {
            service.remove("multiplex");
        }
    }
}

/// Ensure a `[node]` table exists. A non-table `node` is a malformed config
/// (exit 2) rather than a panic on the next index.
pub(crate) fn ensure_node_table(doc: &mut DocumentMut, path: &Path) -> Result<()> {
    match doc.get("node") {
        None => {
            doc["node"] = toml_edit::Item::Table(toml_edit::Table::new());
            Ok(())
        }
        Some(item) if item.is_table_like() => Ok(()),
        Some(_) => Err(CliError::Config(format!(
            "invalid config at {}: [node] must be a table",
            path.display()
        ))
        .into()),
    }
}

/// The non-empty `[node].secret_key` of `doc`, if any.
pub(crate) fn existing_secret_key(doc: &DocumentMut) -> Option<&str> {
    doc.get("node")
        .and_then(|n| n.as_table_like())
        .and_then(|n| n.get("secret_key"))
        .and_then(|k| k.as_str())
        .filter(|k| !k.is_empty())
}

/// Persist a key generated by `run` into `[node].secret_key`.
///
/// Edits the file in place under the writer lock, so comments survive and the
/// file ends up owner-only. If another writer stored a key first, that key
/// wins and is returned, so two racing starts never disagree on the identity.
/// The resulting document is validated as `D` before it is written.
///
/// # Errors
///
/// Config errors for a malformed or invalid document; I/O errors otherwise.
pub(crate) fn persist_generated_key<D: RoleDoc>(path: &Path, encoded: &str) -> Result<String> {
    let path = resolve_target(path);
    let _lock = ConfigLock::acquire(&path)?;
    let text = read_optional(&path)?.unwrap_or_default();
    let mut doc = parse_doc(&text, &path)?;
    if let Some(existing) = existing_secret_key(&doc) {
        resolve_secret_key(existing)?;
        return Ok(existing.to_string());
    }
    ensure_node_table(&mut doc, &path)?;
    doc["node"]["secret_key"] = toml_edit::value(encoded);
    strip_retired_fields(&mut doc);
    let text = doc.to_string();
    let parsed: D = toml::from_str(&text).map_err(|e| {
        CliError::Config(format!(
            "invalid config at {}{}: {}",
            path.display(),
            describe_location(&text, e.span()),
            e.message()
        ))
    })?;
    parsed
        .validate()
        .map_err(|e| CliError::Config(format!("invalid config at {}: {e}", path.display())))?;
    write_atomic(&path, text.as_bytes())?;
    Ok(encoded.to_string())
}
