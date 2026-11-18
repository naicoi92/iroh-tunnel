//! Default config file of a role in a tier, with the macOS legacy migration.

use std::path::{Path, PathBuf};

use anyhow::Result;

#[cfg(target_os = "macos")]
use super::locations::APP_DIR;
use super::locations::{default_config_dir, system_config_dir};
use super::set_migration_notice::notify_migrated;
use crate::error::CliError;
use crate::fs_util::create_private_new;
use crate::Tier;

/// Default config file for `role` in `tier`. The system tier is a fixed
/// machine-wide path that never consults the home directory. For the user
/// tier on macOS a legacy configuration is first copied into place when the
/// new one does not exist yet; that performs filesystem I/O and prints a
/// notice to stderr.
///
/// # Errors
///
/// [`crate::error::CliError::Config`] when the user home directory (or the
/// Windows `ProgramData` folder for the system tier) is unusable, or when the
/// macOS migration fails (no replacement identity is created).
pub fn default_config_path_for(role: &str, tier: Tier) -> Result<PathBuf> {
    if tier == Tier::System {
        return Ok(system_config_dir()?.join(format!("{role}.toml")));
    }
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
#[cfg_attr(
    all(not(test), not(target_os = "macos")),
    expect(dead_code, reason = "only macOS migrates a legacy config")
)]
fn migrate_legacy(legacy: &Path, dest: &Path) -> Result<()> {
    // New configuration wins without consulting the legacy location.
    match std::fs::symlink_metadata(dest) {
        Ok(_) => return Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(anyhow::Error::new(e).context(CliError::Config(format!(
                "cannot access configuration {}; fix its permissions or pass --config",
                dest.display()
            ))))
        }
    }
    match std::fs::symlink_metadata(legacy) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(migration_failed(legacy, dest, e)),
    }
    copy_no_overwrite(legacy, dest).map_err(|e| migration_failed(legacy, dest, e))?;
    notify_migrated(legacy, dest);
    Ok(())
}

fn migration_failed(legacy: &Path, dest: &Path, e: std::io::Error) -> anyhow::Error {
    anyhow::Error::new(e).context(CliError::Config(format!(
        "could not migrate legacy configuration {} to {}; no new identity was created. \
         Fix access to the legacy file, copy it manually, or pass --config",
        legacy.display(),
        dest.display()
    )))
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
        let mut file = create_private_new(&tmp)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_config_path_is_platform_machine_path() {
        let path = default_config_path_for("serve", Tier::System).unwrap();
        #[cfg(target_os = "linux")]
        assert_eq!(path, Path::new("/etc/iroh-tunnel/serve.toml"));
        #[cfg(target_os = "macos")]
        assert_eq!(
            path,
            Path::new("/Library/Application Support/iroh-tunnel/config/serve.toml")
        );
        #[cfg(windows)]
        assert_eq!(
            path,
            crate::service::program_data_dir()
                .unwrap()
                .join("iroh-tunnel")
                .join("config")
                .join("serve.toml")
        );
    }

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
    fn a_failed_migration_keeps_the_io_error_in_the_chain() {
        let dir = tempfile::tempdir().unwrap();
        // A directory as the legacy "file": it exists, but cannot be read.
        let legacy = dir.path().join("legacy");
        std::fs::create_dir(&legacy).unwrap();
        let err = migrate_legacy(&legacy, &dir.path().join("new").join("serve.toml")).unwrap_err();
        assert!(matches!(
            err.downcast_ref::<CliError>(),
            Some(CliError::Config(_))
        ));
        assert!(
            err.chain()
                .any(<dyn std::error::Error>::is::<std::io::Error>),
            "{err:?}"
        );
    }

    #[test]
    fn a_missing_legacy_file_migrates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("serve.toml");
        migrate_legacy(&dir.path().join("none.toml"), &dest).unwrap();
        assert!(!dest.exists());
    }
}
