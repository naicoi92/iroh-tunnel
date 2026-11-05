//! Default config file of a role, with the macOS legacy migration.

use std::path::{Path, PathBuf};

use anyhow::Result;

#[cfg(target_os = "macos")]
use super::locations::APP_DIR;
use super::locations::{config_err, default_config_dir};
use super::set_migration_notice::notify_migrated;

/// Default config file for `role`. On macOS a legacy configuration is first
/// copied into place when the new one does not exist yet; that performs
/// filesystem I/O and prints a notice to stderr.
///
/// # Errors
///
/// [`CliError::Config`] when the home directory is unusable, or when the
/// macOS migration fails (no replacement identity is created).
pub fn default_config_path(role: &str) -> Result<PathBuf> {
    let path = default_config_dir()?.join(format!("{role}.toml"));
    #[cfg(target_os = "macos")]
    {
        let legacy = dirs::config_dir()
            .filter(|d| d.is_absolute())
            .map(|d| d.join(APP_DIR).join(format!("{role}.toml")));
        if let Some(legacy) = legacy {
            migrate_legacy(&legacy, &path)?;
        }
    }
    Ok(path)
}

/// Copy `legacy` to `dest` unless `dest` exists. Never overwrites `dest`
/// (even when racing another process), never touches `legacy`, and creates
/// `dest` owner-only. On failure nothing is created at `dest` (its parent
/// directory may have been created, owner-only).
// Only called from the macOS branch of `default_config_path`; other targets
// keep it compiled (for the shared code path) without a caller.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn migrate_legacy(legacy: &Path, dest: &Path) -> Result<()> {
    // New configuration wins without consulting the legacy location.
    match std::fs::symlink_metadata(dest) {
        Ok(_) => return Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(config_err(format!(
                "cannot access configuration {}: {e}; fix its permissions or pass --config",
                dest.display()
            )))
        }
    }
    match std::fs::symlink_metadata(legacy) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(migration_failed(legacy, dest, &e)),
    }
    copy_no_overwrite(legacy, dest).map_err(|e| migration_failed(legacy, dest, &e))?;
    notify_migrated(legacy, dest);
    Ok(())
}

fn migration_failed(legacy: &Path, dest: &Path, e: &std::io::Error) -> anyhow::Error {
    config_err(format!(
        "could not migrate legacy configuration {} to {}: {e}; no new identity was created. \
         Fix access to the legacy file, copy it manually, or pass --config",
        legacy.display(),
        dest.display()
    ))
}

fn copy_no_overwrite(legacy: &Path, dest: &Path) -> std::io::Result<()> {
    use std::io::Write;
    let content = std::fs::read(legacy)?;
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    create_private_dir(parent)?;
    let tmp = parent.join(format!(
        ".{}.migrate.{}",
        dest.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("config"),
        std::process::id()
    ));
    let result = (|| {
        let mut file = private_new_file(&tmp)?;
        file.write_all(&content)?;
        file.sync_all()?;
        // hard_link fails if `dest` exists: an atomic create-if-absent that
        // also publishes the fully written content in one step.
        match std::fs::hard_link(&tmp, dest) {
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
            other => other,
        }
    })();
    // Best-effort: the temp name is unique per process and `dest` already holds
    // the result (or the error is returned), so a failed cleanup is harmless.
    let _ = std::fs::remove_file(&tmp);
    result
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

#[cfg(unix)]
fn private_new_file(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn private_new_file(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_copies_once_and_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("legacy.toml");
        let dest = dir.path().join("new").join("serve.toml");
        std::fs::write(&legacy, "old").unwrap();
        migrate_legacy(&legacy, &dest).unwrap();
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "old");
        std::fs::write(&legacy, "changed").unwrap();
        migrate_legacy(&legacy, &dest).unwrap();
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(&legacy).unwrap(), "changed");
    }

    #[test]
    fn a_missing_legacy_file_migrates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("serve.toml");
        migrate_legacy(&dir.path().join("none.toml"), &dest).unwrap();
        assert!(!dest.exists());
    }
}
