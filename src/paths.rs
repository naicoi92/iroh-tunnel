//! Platform configuration and state locations, plus the macOS legacy
//! configuration migration (CFG-01, CFG-02; issue #84).
//!
//! macOS and Linux use `~/.config/iroh-tunnel` and `~/.local/state/iroh-tunnel`
//! and ignore the XDG variables. Windows uses the `LocalAppData` known folder
//! (`iroh-tunnel/config`, `iroh-tunnel/state`). Explicit overrides (`--config`,
//! `IROH_TUNNEL_STATE_DIR`) are applied by the callers before these defaults.
//!
//! The legacy macOS location is `dirs::config_dir()` (`~/Library/Application
//! Support`), which every released tag (v0.1.0–v0.4.2) used for `<role>.toml`.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::error::CliError;

const APP_DIR: &str = "iroh-tunnel";

fn config_err(msg: String) -> anyhow::Error {
    CliError::Config(msg).into()
}

#[cfg(not(windows))]
fn home_dir() -> Result<PathBuf> {
    match dirs::home_dir() {
        Some(home) if home.is_absolute() => Ok(home),
        _ => Err(config_err(
            "could not determine an absolute home directory (is HOME set?); \
             set HOME, or pass --config / IROH_TUNNEL_STATE_DIR explicitly"
                .into(),
        )),
    }
}

#[cfg(windows)]
fn local_app_data() -> Result<PathBuf> {
    match dirs::data_local_dir() {
        Some(dir) if dir.is_absolute() => Ok(dir.join(APP_DIR)),
        _ => Err(config_err(
            "could not resolve the LocalAppData folder; \
             pass --config / IROH_TUNNEL_STATE_DIR explicitly"
                .into(),
        )),
    }
}

/// Default directory holding `<role>.toml`.
pub fn default_config_dir() -> Result<PathBuf> {
    #[cfg(windows)]
    return Ok(local_app_data()?.join("config"));
    #[cfg(not(windows))]
    Ok(home_dir()?.join(".config").join(APP_DIR))
}

/// Default directory holding the `<role>-status.json` files.
pub fn default_state_dir() -> Result<PathBuf> {
    #[cfg(windows)]
    return Ok(local_app_data()?.join("state"));
    #[cfg(not(windows))]
    Ok(home_dir()?.join(".local").join("state").join(APP_DIR))
}

/// Default config file for `role`. On macOS a legacy configuration is first
/// copied into place when the new one does not exist yet.
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
/// `dest` owner-only. On failure nothing is created at `dest`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn migrate_legacy(legacy: &Path, dest: &Path) -> Result<()> {
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
    eprintln!(
        "migrated configuration {} -> {} (original kept)",
        legacy.display(),
        dest.display()
    );
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
