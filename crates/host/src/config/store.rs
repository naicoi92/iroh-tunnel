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
use iroh::SecretKey;
use toml_edit::DocumentMut;

use super::dto::RoleDoc;
use super::encode_secret_key::encode_secret_key;
use super::resolve_secret_key::resolve_secret_key;
use crate::error::CliError;
use crate::fs_util::create_private_new;

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

/// How a missing config directory is created. An existing directory is
/// never touched in either case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirCreation {
    /// Platform defaults (umask on Unix, inherited DACL on Windows).
    Inherited,
    /// The system tier's config directory: on Unix every missing component
    /// gets mode `0755` whatever the umask; on Windows the directory gets the
    /// protected admin-only descriptor, never a readable parent's ACEs.
    SystemTier,
}

/// The one decision for a write to `config`: the path to write through and
/// how a missing parent is created. On Windows an elevated write into the
/// system-tier config directory is also confidentiality-checked (see
/// [`ConfigLock::acquire`]); both follow from this same decision.
///
/// # Errors
///
/// Windows, elevated: the target or the system config directory cannot be
/// resolved. The write is refused rather than created with inherited
/// permissions (fail closed).
fn plan_write(config: &Path) -> Result<(PathBuf, DirCreation)> {
    #[cfg(windows)]
    {
        let elevated = crate::Tier::current() == crate::Tier::System;
        let target =
            system_config_target(config, elevated, std::env::current_dir, system_config_dir)?;
        Ok(match target {
            Some(target) => (target, DirCreation::SystemTier),
            None => (config.to_path_buf(), DirCreation::Inherited),
        })
    }
    #[cfg(not(windows))]
    Ok((
        config.to_path_buf(),
        if crate::Tier::current() == crate::Tier::System {
            DirCreation::SystemTier
        } else {
            DirCreation::Inherited
        },
    ))
}

/// The system tier's config directory.
#[cfg(windows)]
fn system_config_dir() -> Result<PathBuf> {
    crate::paths::default_config_path_for("serve", crate::Tier::System)?
        .parent()
        .map(Path::to_path_buf)
        .context("the system config path has no directory")
}

/// Create the parent directory of `path` when it is missing; a write into
/// the system tier's config directory creates it as
/// [`DirCreation::SystemTier`].
///
/// # Errors
///
/// As [`plan_write`], or the directory cannot be created.
pub(crate) fn ensure_parent(path: &Path) -> Result<()> {
    let (target, creation) = plan_write(path)?;
    ensure_parent_with(&target, creation)
}

/// [`ensure_parent`] with the creation mode injected.
fn ensure_parent_with(path: &Path, creation: DirCreation) -> Result<()> {
    let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) else {
        return Ok(());
    };
    match creation {
        DirCreation::Inherited => std::fs::create_dir_all(parent).map_err(anyhow::Error::from),
        DirCreation::SystemTier => create_system_config_dir(parent),
    }
    .with_context(|| format!("failed to create config dir: {}", parent.display()))
}

/// Create the missing system config directory `dir`: every missing component
/// `0755`, set on the created directory itself so the umask cannot narrow it.
#[cfg(not(windows))]
fn create_system_config_dir(dir: &Path) -> Result<()> {
    Ok(crate::ownership::create_dir(dir, 0o755)?)
}

/// Create the missing system config directory `dir` with the protected
/// admin-only descriptor ([`crate::service::CONFIG_DIR_SDDL`]), so it never
/// inherits a Users-readable parent DACL. A missing machine directory
/// (`%ProgramData%\iroh-tunnel`) is created first with its own protected
/// descriptor; an existing `dir` is left as it is.
#[cfg(windows)]
fn create_system_config_dir(dir: &Path) -> Result<()> {
    if dir.exists() {
        return Ok(());
    }
    if let Some(parent) = dir.parent().filter(|p| !p.as_os_str().is_empty()) {
        if let Some(machine) = crate::ownership::machine_dir()? {
            if same_windows_path(&machine.path, parent) {
                crate::ownership::ensure_machine_dir(&machine)?;
            }
        }
        std::fs::create_dir_all(parent)?;
    }
    match crate::service::create_dir_with_sddl(dir, crate::service::CONFIG_DIR_SDDL) {
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        other => Ok(other?),
    }
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

/// Whether two paths name the same location the way Windows resolves
/// paths: case-insensitive, `/` and `\\` alike, a verbatim `\\?\\` prefix
/// and trailing separators ignored. Callers pass absolute identities
/// ([`absolute_identity`]).
#[cfg(any(test, windows))]
fn same_windows_path(a: &Path, b: &Path) -> bool {
    fn normalized(path: &Path) -> String {
        let text = path.to_string_lossy().replace('/', "\\");
        let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
        text.trim_end_matches('\\').to_lowercase()
    }
    normalized(a) == normalized(b)
}

/// The absolute identity of `path`, whether or not it exists: a relative
/// path is joined to `cwd` (the working directory, injected), its longest existing ancestor
/// canonicalized (links resolved), the missing rest appended with `.` and
/// `..` resolved lexically (nothing below a missing component can be a
/// link). A verbatim `\\?\\` prefix that `canonicalize` adds on Windows is
/// dropped, except for UNC paths.
///
/// # Errors
///
/// The existing ancestor cannot be canonicalized.
#[cfg(any(test, windows))]
fn absolute_identity(path: &Path, cwd: &Path) -> std::io::Result<PathBuf> {
    use std::path::Component;
    let absolute = std::path::absolute(cwd.join(path))?;
    let existing = absolute
        .ancestors()
        .find(|a| std::fs::symlink_metadata(a).is_ok())
        .unwrap_or(&absolute);
    let mut identity = std::fs::canonicalize(existing)?;
    let rest = absolute.strip_prefix(existing).unwrap_or(Path::new(""));
    for component in rest.components() {
        match component {
            Component::ParentDir => {
                identity.pop();
            }
            Component::Normal(name) => identity.push(name),
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
        }
    }
    Ok(crate::fs_util::strip_verbatim_prefix(identity))
}

/// The absolute identity of `config` when an elevated process writes it
/// into the system-tier config directory (`system_dir`), else `None`. A
/// relative path or a missing tail is resolved first, so
/// `config\serve.toml` from `%ProgramData%\iroh-tunnel` is recognised.
///
/// # Errors
///
/// Elevated only: the working directory (`cwd`), `config` or the system
/// directory cannot be resolved. The caller must refuse the write, never
/// fall back to inherited permissions.
#[cfg(any(test, windows))]
fn system_config_target(
    config: &Path,
    elevated: bool,
    cwd: impl FnOnce() -> std::io::Result<PathBuf>,
    system_dir: impl FnOnce() -> Result<PathBuf>,
) -> Result<Option<PathBuf>> {
    if !elevated {
        return Ok(None);
    }
    let cwd = cwd().context("cannot read the working directory to resolve the config path")?;
    let resolve = |p: &Path| {
        absolute_identity(p, &cwd).with_context(|| {
            format!(
                "cannot resolve {} to decide whether it is the system config directory",
                p.display()
            )
        })
    };
    let target = resolve(config)?;
    let system = resolve(&system_dir()?)?;
    let in_system = target
        .parent()
        .is_some_and(|dir| same_windows_path(dir, &system));
    Ok(in_system.then_some(target))
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
    ///
    /// On Windows an elevated write into the system-tier config directory
    /// first refuses a directory or file readable by non-admins, before any
    /// file is created.
    pub(crate) fn acquire(config: &Path) -> Result<Self> {
        let (target, creation) = plan_write(config)?;
        #[cfg(windows)]
        if creation == DirCreation::SystemTier {
            let trusted = crate::service::admin_trust(&crate::service::process_env)?;
            return Self::acquire_with(&target, Some(&trusted));
        }
        Self::acquire_unchecked(&target, creation)
    }

    /// [`Self::acquire`] with the decision injected: `Some` is a write into
    /// the system config directory, checked against that trusted set before
    /// any file is created and creating a missing directory admin-only;
    /// `None` is any other write.
    #[cfg(windows)]
    fn acquire_with(
        config: &Path,
        confidential: Option<&crate::service::trust_windows::model::TrustedSet>,
    ) -> Result<Self> {
        let Some(trusted) = confidential else {
            return Self::acquire_unchecked(config, DirCreation::Inherited);
        };
        // A write creates what is missing, so only existing objects are checked.
        crate::service::trust_windows::verify_config_confidential(
            config,
            crate::service::trust_windows::MissingObject::Skip,
            &|path| crate::service::trust_windows::verify_confidential_with(path, trusted),
        )?;
        Self::acquire_unchecked(config, DirCreation::SystemTier)
    }

    /// Take the lock without any confidentiality check, creating a missing
    /// parent as `creation`.
    fn acquire_unchecked(config: &Path, creation: DirCreation) -> Result<Self> {
        ensure_parent_with(config, creation)?;
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
        let mut file = create_private_new(&tmp)
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
fn strip_retired_fields(doc: &mut DocumentMut) {
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

/// Store `key` as `[node].secret_key`, creating `[node]` when absent.
///
/// # Errors
///
/// [`CliError::Config`] (exit 2) when `node` exists but is not a table.
pub(crate) fn set_node_secret_key(doc: &mut DocumentMut, path: &Path, key: &str) -> Result<()> {
    let node = node_table_mut(doc, path)?;
    // Assign in place when present: `insert` would reset the key's decor
    // and drop the operator's comment above it.
    match node.get_mut("secret_key") {
        Some(item) => *item = toml_edit::value(key),
        None => {
            node.insert("secret_key", toml_edit::value(key));
        }
    }
    Ok(())
}

/// The `[node]` table of `doc`, created when absent.
///
/// A `[node]` that is not a table is a config error naming `path`.
pub(crate) fn node_table_mut<'a>(
    doc: &'a mut DocumentMut,
    path: &Path,
) -> Result<&'a mut dyn toml_edit::TableLike> {
    let node = doc
        .entry("node")
        .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_like_mut()
        .ok_or_else(|| {
            CliError::Config(format!(
                "invalid config at {}: [node] must be a table",
                path.display()
            ))
        })?;
    Ok(node)
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
    set_node_secret_key(&mut doc, &path, encoded)?;
    let text = render_role_doc::<D>(&mut doc, &path)?;
    write_atomic(&path, text.as_bytes())?;
    Ok(encoded.to_string())
}

/// The one parse-and-validate path of every role config entry point.
///
/// A malformed document reads `<malformed> <path> (line N): <message>`, an
/// invalid one `invalid config at <path>: <reason>`; both are config errors
/// (exit 2). `toml` errors quote the offending source line, which can hold a
/// secret, so the diagnostic carries the message and line only (RS-10).
fn parse_role_doc<D: RoleDoc>(text: &str, path: &Path, malformed: &str) -> Result<D> {
    let doc: D = toml::from_str(text).map_err(|e| {
        CliError::Config(format!(
            "{malformed} {}{}: {}",
            path.display(),
            describe_location(text, e.span()),
            e.message()
        ))
    })?;
    doc.validate()
        .map_err(|e| CliError::Config(format!("invalid config at {}: {e:#}", path.display())))?;
    Ok(doc)
}

/// Validate a document about to be written or checked; any failure is an
/// `invalid config at <path>` config error (exit 2).
///
/// # Errors
///
/// [`CliError::Config`] when `text` is not valid TOML for `D` or fails
/// validation.
pub(crate) fn check_role_doc<D: RoleDoc>(text: &str, path: &Path) -> Result<D> {
    parse_role_doc(text, path, "invalid config at")
}

/// Drop retired fields from `doc` and render it, refusing a result that is
/// not a valid `D`. Returns the text to write.
///
/// # Errors
///
/// See [`check_role_doc`].
pub(crate) fn render_role_doc<D: RoleDoc>(doc: &mut DocumentMut, path: &Path) -> Result<String> {
    strip_retired_fields(doc);
    let text = doc.to_string();
    check_role_doc::<D>(&text, path)?;
    Ok(text)
}

/// Read, parse and validate a role config from a TOML file.
///
/// # Errors
///
/// Fails when the file cannot be read; otherwise the errors of
/// [`parse_role_config`].
pub(crate) fn load_role_config<D: RoleDoc>(path: &Path) -> Result<D> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read config: {}", path.display()))?;
    parse_role_config(&content, path)
}

/// Parse and validate config text already read from `path` (used when the
/// bytes come from a verified handle rather than a path lookup).
///
/// # Errors
///
/// [`CliError::Config`] naming `path` when the text is not valid TOML for `D`
/// (the message carries the path and line only, never the offending source
/// line) or fails validation.
pub(crate) fn parse_role_config<D: RoleDoc>(content: &str, path: &Path) -> Result<D> {
    parse_role_doc(content, path, "failed to parse config:")
}

/// Resolve the node secret key; if it was just generated, persist it back
/// into the config file. Returns the resolved key.
///
/// The key is stored by editing the file in place, owner-only, under the
/// writer lock; if another process stored a key first, that key is adopted.
/// The key value itself is never logged (NFR-05).
///
/// Pinned access identity: an empty access `secret_key` is generated once and
/// persisted, so the access NodeId is stable across restarts (mirroring the
/// serve role).
///
/// # Errors
///
/// Fails when the configured key cannot be decoded, or when persisting the
/// generated key fails (see [`persist_generated_key`]).
pub(crate) fn resolve_and_persist_key<D: RoleDoc>(cfg: &mut D, path: &Path) -> Result<SecretKey> {
    let (key, needs_save) = resolve_secret_key(&cfg.node_mut().secret_key)?;
    if !needs_save {
        return Ok(key);
    }
    tracing::warn!("secret_key empty, generated new key, saving to config");
    let stored = persist_generated_key::<D>(path, &encode_secret_key(&key))?;
    cfg.node_mut().secret_key = stored;
    Ok(resolve_secret_key(&cfg.node_mut().secret_key)?.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The process working directory, injected for [`system_config_target`].
    fn here() -> std::io::Result<PathBuf> {
        std::env::current_dir()
    }

    /// A system directory injected for [`system_config_target`].
    fn system_dir(path: &Path) -> impl FnOnce() -> Result<PathBuf> + '_ {
        move || Ok(path.to_path_buf())
    }

    #[test]
    fn elevated_write_outside_the_system_config_dir_is_not_checked() {
        let tmp = tempfile::tempdir().unwrap();
        let system = tmp.path().join("machine").join("config");
        let config = tmp.path().join("serve.toml");
        assert_eq!(
            system_config_target(&config, true, here, system_dir(&system)).unwrap(),
            None
        );
        let relative = Path::new("serve.toml");
        assert_eq!(
            system_config_target(relative, true, here, system_dir(&system)).unwrap(),
            None
        );
    }

    #[test]
    fn elevated_write_into_the_system_config_dir_is_checked() {
        let tmp = tempfile::tempdir().unwrap();
        let system = tmp.path().join("machine").join("config");
        let config = system.join("access.toml");
        let target = system_config_target(&config, true, here, system_dir(&system)).unwrap();
        assert!(target.is_some_and(|t| t.ends_with("machine/config/access.toml")));
        assert_eq!(
            system_config_target(&config, false, here, system_dir(&system)).unwrap(),
            None
        );
    }

    /// Review round 1: a relative `--config` from the machine directory with
    /// the `config` directory still missing names the system config dir.
    #[test]
    fn relative_target_with_missing_tail_is_the_system_config_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let machine = tmp.path().join("machine");
        std::fs::create_dir(&machine).unwrap();
        let system = machine.join("config");
        let cwd = || Ok(machine.clone());
        let expected = absolute_identity(&machine, &machine)
            .unwrap()
            .join("config")
            .join("serve.toml");
        for relative in [
            "config/serve.toml",
            r"config\serve.toml",
            "./config/x/../serve.toml",
        ] {
            if relative.contains('\\') && !cfg!(windows) {
                continue; // `\` is a separator only on Windows
            }
            let target = system_config_target(Path::new(relative), true, cwd, system_dir(&system))
                .unwrap()
                .unwrap_or_else(|| panic!("{relative} is the system config dir"));
            assert!(target.is_absolute(), "{}", target.display());
            assert!(
                same_windows_path(&target, &expected),
                "{}",
                target.display()
            );
        }
        assert!(!system.exists(), "deciding creates nothing");
    }

    #[test]
    fn unreadable_working_directory_fails_closed_when_elevated() {
        let missing = || Err(std::io::Error::other("cwd gone"));
        let system = PathBuf::from("/machine/config");
        let relative = Path::new("config/serve.toml");
        assert!(system_config_target(relative, true, missing, system_dir(&system)).is_err());
    }

    #[test]
    fn unresolvable_system_dir_fails_closed_when_elevated() {
        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path().join("serve.toml");
        let failing = || -> Result<PathBuf> { anyhow::bail!("no ProgramData") };
        assert!(system_config_target(&config, true, here, failing).is_err());
        let failing = || -> Result<PathBuf> { anyhow::bail!("no ProgramData") };
        assert_eq!(
            system_config_target(&config, false, here, failing).unwrap(),
            None
        );
    }

    #[test]
    fn absolute_identity_resolves_links_in_the_existing_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let identity = absolute_identity(&real.join("missing/../a/b"), Path::new("/")).unwrap();
        // The identity drops the verbatim prefix `canonicalize` adds on Windows.
        let real = crate::fs_util::strip_verbatim_prefix(std::fs::canonicalize(&real).unwrap());
        assert_eq!(identity, real.join("a").join("b"));
    }

    #[test]
    fn system_config_dir_match_ignores_case_separators_and_prefix() {
        let system = Path::new(r"C:\ProgramData\iroh-tunnel\config");
        for same in [
            r"C:\ProgramData\iroh-tunnel\config",
            r"c:\programdata\IROH-TUNNEL\Config\",
            r"\\?\C:\ProgramData\iroh-tunnel\config",
            "C:/ProgramData/iroh-tunnel/config",
        ] {
            assert!(same_windows_path(Path::new(same), system), "{same}");
        }
        for other in [
            r"C:\ProgramData\iroh-tunnel",
            r"C:\ProgramData\iroh-tunnel\config\sub",
            r"C:\Users\alice\AppData\Local\iroh-tunnel",
            r"D:\ProgramData\iroh-tunnel\config",
        ] {
            assert!(!same_windows_path(Path::new(other), system), "{other}");
        }
    }
    use crate::config::fixtures::VALID_SERVE;
    use crate::config::{AccessConfig, NodeConfig, ServeConfig};

    fn with_key(text: &str) -> Result<String> {
        let mut doc: DocumentMut = text.parse()?;
        set_node_secret_key(&mut doc, Path::new("c.toml"), "k")?;
        Ok(doc.to_string())
    }

    #[test]
    fn set_node_secret_key_creates_the_node_table() {
        assert_eq!(with_key("").unwrap(), "[node]\nsecret_key = \"k\"\n");
    }

    #[test]
    fn set_node_secret_key_keeps_an_inline_node_table() {
        assert_eq!(
            with_key("node = { relay_urls = [] }\n").unwrap(),
            "node = { relay_urls = [] , secret_key = \"k\" }\n"
        );
    }

    #[test]
    fn set_node_secret_key_keeps_the_comment_above_an_existing_key() {
        assert_eq!(
            with_key("[node]\n# rotated 2024\nsecret_key   = \"\"\n").unwrap(),
            "[node]\n# rotated 2024\nsecret_key   = \"k\"\n"
        );
    }

    #[test]
    fn set_node_secret_key_rejects_a_non_table_node() {
        let err = with_key("node = 5\n").unwrap_err();
        assert!(format!("{err}").contains("[node] must be a table"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn system_tier_creates_missing_dirs_0755_and_never_remodes_existing() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let tmp = tempfile::tempdir().unwrap();
        let existing = tmp.path().join("existing");
        std::fs::create_dir(&existing).unwrap();
        std::fs::set_permissions(&existing, std::fs::Permissions::from_mode(0o700)).unwrap();
        let config = existing.join("a/b/serve.toml");
        ensure_parent_with(&config, DirCreation::SystemTier).unwrap();
        assert_eq!(
            mode(&existing),
            0o700,
            "an existing directory keeps its mode"
        );
        assert_eq!(mode(&existing.join("a")), 0o755);
        assert_eq!(mode(&existing.join("a/b")), 0o755);
        // A second call on the existing tree changes nothing.
        std::fs::set_permissions(existing.join("a/b"), std::fs::Permissions::from_mode(0o750))
            .unwrap();
        ensure_parent_with(&config, DirCreation::SystemTier).unwrap();
        assert_eq!(mode(&existing.join("a/b")), 0o750);
    }

    #[test]
    fn load_role_config_reads_and_validates() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("serve.toml");
        std::fs::write(&p, VALID_SERVE).unwrap();
        let cfg = load_role_config::<ServeConfig>(&p).unwrap();
        assert_eq!(cfg.services.len(), 1);
    }

    #[test]
    fn load_role_config_missing_file_names_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("missing.toml");
        let err = load_role_config::<ServeConfig>(&p).unwrap_err();
        assert_eq!(
            format!("{err}"),
            format!("failed to read config: {}", p.display())
        );
    }

    fn config_error_message(err: &anyhow::Error) -> &str {
        match err.downcast_ref::<CliError>() {
            Some(CliError::Config(m)) => m,
            _ => panic!("expected CliError::Config, got {err:?}"),
        }
    }

    #[test]
    fn parse_role_config_reports_path_and_line_without_secret() {
        let text = "[node]\nsecret_key = \"SECRET-XYZ\" oops\n";
        let err = parse_role_config::<ServeConfig>(text, Path::new("serve.toml")).unwrap_err();
        let msg = toml::from_str::<toml::Value>(text)
            .unwrap_err()
            .message()
            .to_string();
        assert_eq!(
            config_error_message(&err),
            format!("failed to parse config: serve.toml (line 2): {msg}")
        );
        for shown in [format!("{err:#}"), format!("{err:?}")] {
            assert!(!shown.contains("SECRET-XYZ"), "{shown}");
        }
    }

    #[test]
    fn parse_role_config_validation_failure_is_a_config_error_naming_the_path() {
        let text = "[[services]]\nname = \"db\"\nprotocol = \"tcp\"\nport = 0\n";
        let err = parse_role_config::<ServeConfig>(text, Path::new("serve.toml")).unwrap_err();
        assert_eq!(
            config_error_message(&err),
            "invalid config at serve.toml: invalid port: must be 1-65535"
        );
    }

    /// Rejected text, per entry point: `(text, load, check, writers)`, where
    /// the writers are a mutation, an edit commit and key persistence.
    fn entry_point_diagnostics(p: &str) -> Vec<(&'static str, String, String, String)> {
        const SCHEMA: &str = "[[services]]\nname = \"a\"\nprotocol = \"tcp\"\nport = 1\nmultiplex = true\n\n[[services]]\nname = \"b\"\nprotocol = \"tcp\"\nport = \"x\"\n";
        let syntax = "unclosed array, expected `]`";
        let schema = toml::from_str::<ServeConfig>(SCHEMA)
            .unwrap_err()
            .message()
            .to_string();
        let port = "invalid port: must be 1-65535";
        vec![
            (
                "not = [valid",
                format!("failed to parse config: {p} (line 1): {syntax}"),
                format!("invalid config at {p} (line 1): {syntax}"),
                format!("failed to parse config {p} (line 1): {syntax}"),
            ),
            // Writers check the document after dropping `multiplex`, so
            // their line is one less than the source line.
            (
                SCHEMA,
                format!("failed to parse config: {p} (line 10): {schema}"),
                format!("invalid config at {p} (line 10): {schema}"),
                format!("invalid config at {p} (line 9): {schema}"),
            ),
            (
                "[[services]]\nname = \"db\"\nprotocol = \"tcp\"\nport = 0\n",
                format!("invalid config at {p}: {port}"),
                format!("invalid config at {p}: {port}"),
                format!("invalid config at {p}: {port}"),
            ),
        ]
    }

    #[test]
    fn every_entry_point_keeps_its_rejection_diagnostic() {
        use crate::role::Role;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("serve.toml");
        let shown = p.display().to_string();
        for (text, load, check, writers) in entry_point_diagnostics(&shown) {
            std::fs::write(&p, text).unwrap();
            let msg = |r: Result<()>| config_error_message(&r.unwrap_err()).to_string();
            assert_eq!(msg(load_role_config::<ServeConfig>(&p).map(drop)), load);
            assert_eq!(
                msg(crate::config::check_config(Role::Serve, Some(&p)).map(drop)),
                check
            );
            assert_eq!(
                msg(super::super::mutate::mutate(Role::Serve, &p, |_| Ok(()))),
                writers
            );
            assert_eq!(
                msg(persist_generated_key::<ServeConfig>(&p, "k").map(drop)),
                writers
            );
            let attempt = crate::config::prepare_edit(Role::Serve, Some(&p)).unwrap();
            std::fs::write(attempt.scratch_path(), format!("{text}\n")).unwrap();
            match attempt.commit().unwrap() {
                crate::config::EditOutcome::Rejected { message, .. } => assert!(
                    message.starts_with(&format!("config error: {writers}; ")),
                    "{message}"
                ),
                other => panic!("unexpected {other:?}"),
            }
            assert_eq!(std::fs::read_to_string(&p).unwrap(), text);
        }
    }

    #[test]
    fn resolve_and_persist_key_generates_and_persists() {
        // An empty access secret_key is generated once and written back, so the
        // access NodeId is stable across restarts (mirrors serve behavior).
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("access.toml");
        std::fs::write(&p, "").unwrap();
        let mut cfg = AccessConfig::default();
        assert!(cfg.node.secret_key.is_empty());

        let key = resolve_and_persist_key(&mut cfg, &p).unwrap();
        assert!(!cfg.node.secret_key.is_empty(), "key should be persisted");

        // The file on disk now carries the encoded key.
        let reloaded = load_role_config::<AccessConfig>(&p).unwrap();
        assert!(
            reloaded.node.secret_key == cfg.node.secret_key,
            "persisted key differs"
        );

        // Re-resolving off the persisted value is a no-op (needs_save == false)
        // and yields the same public key.
        let (key2, needs_save) = resolve_secret_key(&reloaded.node.secret_key).unwrap();
        assert!(!needs_save);
        assert_eq!(key.public(), key2.public());
    }

    #[test]
    fn resolve_and_persist_key_no_save_when_present() {
        // A pre-existing secret_key is reused verbatim; the file is not rewritten.
        let (key, _) = resolve_secret_key("").unwrap();
        let enc = encode_secret_key(&key);
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("access.toml");
        std::fs::write(&p, "").unwrap();
        let mut cfg = AccessConfig {
            node: NodeConfig {
                secret_key: enc.clone(),
                ..Default::default()
            },
            ..Default::default()
        };
        let resolved = resolve_and_persist_key(&mut cfg, &p).unwrap();
        assert_eq!(resolved.public(), key.public());
        assert!(cfg.node.secret_key == enc, "value changed when already set");
    }

    #[test]
    fn resolve_and_persist_key_adopts_concurrently_persisted_key() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("access.toml");
        std::fs::write(&p, "").unwrap();
        let mut cfg = load_role_config::<AccessConfig>(&p).unwrap();

        // Another process stores a key after this one loaded a keyless config.
        let (other, _) = resolve_secret_key("").unwrap();
        let enc = encode_secret_key(&other);
        let written = format!("[node]\nsecret_key = \"{enc}\"\n");
        std::fs::write(&p, &written).unwrap();

        let key = resolve_and_persist_key(&mut cfg, &p).unwrap();
        assert_eq!(key.public(), other.public());
        assert!(cfg.node.secret_key == enc, "adopted key differs");
        assert!(
            std::fs::read_to_string(&p).unwrap() == written,
            "adoption rewrote the config"
        );
    }

    #[test]
    fn resolve_and_persist_key_drops_legacy_fields_for_both_roles() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("role.toml");
        let service = format!(
            "[[services]]\nname = \"echo\"\nprotocol = \"tcp\"\nport = 5432\nnode_id = {:?}\nmultiplex = false\n",
            iroh::SecretKey::generate().public().to_string()
        );
        for access in [true, false] {
            std::fs::write(&p, &service).unwrap();
            let (key, persisted) = if access {
                let mut cfg = load_role_config::<AccessConfig>(&p).unwrap();
                let key = resolve_and_persist_key(&mut cfg, &p).unwrap();
                (
                    key,
                    load_role_config::<AccessConfig>(&p)
                        .unwrap()
                        .node
                        .secret_key,
                )
            } else {
                let mut cfg = load_role_config::<ServeConfig>(&p).unwrap();
                let key = resolve_and_persist_key(&mut cfg, &p).unwrap();
                (
                    key,
                    load_role_config::<ServeConfig>(&p).unwrap().node.secret_key,
                )
            };
            assert!(!std::fs::read_to_string(&p).unwrap().contains("multiplex"));
            assert!(
                persisted == encode_secret_key(&key),
                "persisted key differs"
            );
        }
    }
}

#[cfg(test)]
#[cfg(windows)]
mod windows_tests {
    use super::*;
    use crate::service::trust_windows::model::TrustedSet;
    use crate::service::trust_windows::token_user;

    fn admin_and_me() -> TrustedSet {
        TrustedSet::admin_with_test_owner(token_user().unwrap())
    }

    fn icacls(args: &[&str]) -> Vec<u8> {
        let out = std::process::Command::new("icacls")
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "icacls {args:?}");
        out.stdout
    }

    #[test]
    fn elevated_store_write_refuses_readable_dir_before_creating_files() {
        // `(OI)(IO)R` does not apply to the directory itself but to the secret
        // temp/config the write would create in it.
        for grant in ["*S-1-5-32-545:(OI)(CI)R", "*S-1-5-32-545:(OI)(IO)R"] {
            let tmp = tempfile::tempdir().unwrap();
            let dir = tmp.path().join("config");
            std::fs::create_dir(&dir).unwrap();
            let dir_text = dir.to_str().unwrap();
            icacls(&[dir_text, "/grant", grant]);
            let before = icacls(&[dir_text]);
            let config = dir.join("serve.toml");
            assert!(!config.exists());
            let err = ConfigLock::acquire_with(&config, Some(&admin_and_me())).unwrap_err();
            assert!(
                matches!(err.downcast_ref::<CliError>(), Some(CliError::Permission(m)) if m.contains("S-1-5-32-545") && m.contains("Users")),
                "{grant}: {err:#}"
            );
            assert_eq!(
                std::fs::read_dir(&dir).unwrap().count(),
                0,
                "{grant}: no lock/temp/config"
            );
            assert_eq!(icacls(&[dir_text]), before, "{grant}: DACL unchanged");
        }
    }

    #[test]
    fn elevated_store_write_into_a_confidential_dir_takes_the_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path().join("serve.toml");
        let _lock = ConfigLock::acquire_with(&config, Some(&admin_and_me())).unwrap();
        assert!(tmp.path().join("serve.toml.lock").exists());
    }

    /// Ledger (Task 3 review): a missing system config directory below a
    /// Users-readable parent is created with the protected admin-only
    /// descriptor, so the secret written into it is admin-only. Needs an
    /// elevated token to own the SDDL's Administrators owner.
    #[test]
    fn missing_config_dir_below_readable_parent_is_admin_only() {
        if !crate::service::is_elevated() {
            return; // `O:BA` can only be set by an elevated token
        }
        let tmp = tempfile::tempdir().unwrap();
        let parent = tmp.path().join("iroh-tunnel");
        std::fs::create_dir(&parent).unwrap();
        icacls(&[
            parent.to_str().unwrap(),
            "/grant",
            "*S-1-5-32-545:(OI)(CI)R",
        ]);
        let config = parent.join("config").join("serve.toml");
        ensure_parent_with(&config, DirCreation::SystemTier).unwrap();
        let trusted = TrustedSet::machine();
        let _lock = ConfigLock::acquire_with(&config, Some(&trusted)).unwrap();
        write_atomic(&config, b"[node]\nsecret_key = \"x\"\n").unwrap();
        crate::service::trust_windows::verify_confidential_with(&parent.join("config"), &trusted)
            .unwrap();
        crate::service::trust_windows::verify_confidential_with(&config, &trusted).unwrap();
        // The parent itself is never re-DACLed.
        let err =
            crate::service::trust_windows::verify_confidential_with(&parent, &trusted).unwrap_err();
        assert!(format!("{err:#}").contains("S-1-5-32-545"), "{err:#}");
    }

    #[test]
    fn unchecked_write_ignores_a_readable_dir() {
        let tmp = tempfile::tempdir().unwrap();
        icacls(&[
            tmp.path().to_str().unwrap(),
            "/grant",
            "*S-1-5-32-545:(OI)(CI)R",
        ]);
        let _lock = ConfigLock::acquire_with(&tmp.path().join("serve.toml"), None).unwrap();
    }
}
