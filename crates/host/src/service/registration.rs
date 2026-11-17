//! Registration-file helpers shared by the backends (private).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;

use crate::error::CliError;

/// Name suffix that keeps a test registration apart from any real service.
/// Empty in production builds; `test-utils` reads it from the environment.
#[allow(dead_code)] // unused on targets without a service backend
pub(crate) fn instance_suffix() -> String {
    #[cfg(feature = "test-utils")]
    if let Some(suffix) = std::env::var_os("IROH_TUNNEL_TEST_SERVICE_SUFFIX") {
        return suffix.to_string_lossy().into_owned();
    }
    String::new()
}

/// Write a unit/plist/script, creating its directory.
///
/// # Errors
///
/// [`CliError::Permission`] when the OS denies access. The scope and account
/// are left as requested and the message says how to proceed, because
/// retrying under another scope or account would silently broaden access.
#[allow(dead_code)] // unused on targets without a service backend
pub(crate) fn write_registration(path: &Path, contents: &str) -> Result<()> {
    let denied = |e: std::io::Error, what: &str, target: &Path| -> anyhow::Error {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            CliError::Permission(format!(
                "permission denied {what} {}: {e}. The service scope and account were not \
                 changed and nothing was registered. Fix the permissions of that location and \
                 re-run; use --system (as root) only if you intend a system service",
                target.display()
            ))
            .into()
        } else {
            anyhow::Error::new(e).context(format!("failed {what} {}", target.display()))
        }
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| denied(e, "creating", parent))?;
    }
    // Atomic and never group/world-writable whatever the umask: a root-owned
    // unit that a group could edit would be a path to root (SM-04), and a
    // crash mid-write must not leave a truncated unit.
    StagedRegistration::create(path, contents)
        .and_then(|staged| staged.persist(path))
        .map_err(|e| denied(e, "writing", path))
}

/// Per-process sequence making each staging file name unique, so concurrent
/// writes in one process never share a temp file.
static STAGING_SEQ: AtomicU64 = AtomicU64::new(0);

/// A registration staged in a sibling temp file this call created; removed
/// on drop unless [`persist`](Self::persist) renamed it into place.
struct StagedRegistration {
    path: PathBuf,
    armed: bool,
}

impl StagedRegistration {
    /// Create the staging file exclusively (`O_EXCL`: a pre-planted file or
    /// symlink is never followed, truncated or chmodded), set its mode on the
    /// handle, write `contents` and flush them to disk.
    fn create(target: &Path, contents: &str) -> std::io::Result<Self> {
        let path = staging_path(target)?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        // Armed from here on: the file exists and is ours to clean up.
        let staged = Self { path, armed: true };
        #[cfg(unix)]
        file.set_permissions(registration_permissions(target))?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        Ok(staged)
    }

    /// Atomically rename the staged file over `target`.
    fn persist(mut self, target: &Path) -> std::io::Result<()> {
        std::fs::rename(&self.path, target)?;
        self.armed = false;
        Ok(())
    }
}

impl Drop for StagedRegistration {
    fn drop(&mut self) {
        if self.armed {
            // Best effort: the original error is what the caller reports.
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Unique hidden sibling of `target`. The leading dot and `.tmp` suffix keep
/// a crash leftover out of `S??*` init globs and unit/plist directory scans.
fn staging_path(target: &Path) -> std::io::Result<PathBuf> {
    let name = target.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "registration path has no file name",
        )
    })?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let seq = STAGING_SEQ.fetch_add(1, Ordering::Relaxed);
    let staged_name = format!(
        ".{}.{}-{nonce:08x}-{seq}.tmp",
        name.to_string_lossy(),
        std::process::id()
    );
    Ok(target.with_file_name(staged_name))
}

/// 0755 for extension-less files (init scripts), 0644 otherwise.
#[cfg(unix)]
fn registration_permissions(target: &Path) -> std::fs::Permissions {
    use std::os::unix::fs::PermissionsExt;
    let mode = if target.extension().is_none() {
        0o755
    } else {
        0o644
    };
    std::fs::Permissions::from_mode(mode)
}

/// The text of an installed registration. Absent is `None`; a registration
/// that exists but cannot be read counts as one with an unknown owner (empty
/// text), never as "nothing installed", so the retarget prompt is not skipped.
#[allow(dead_code)] // unused on targets without a service backend
pub(crate) fn read_registration(path: &Path) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => Some(String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_registration_creates_the_directory_and_replaces_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("units").join("x.service");
        write_registration(&path, "one").unwrap();
        write_registration(&path, "two").unwrap();
        assert_eq!(read_registration(&path).as_deref(), Some("two"));
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
    }

    /// Issue #224: a file or symlink planted at the old predictable temp name
    /// is neither followed, truncated, chmodded nor removed.
    #[cfg(unix)]
    #[test]
    fn a_symlink_at_the_old_temp_name_is_never_followed() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let sentinel = dir.path().join("sentinel");
        std::fs::write(&sentinel, "keep").unwrap();
        std::fs::set_permissions(&sentinel, std::fs::Permissions::from_mode(0o600)).unwrap();
        let path = dir.path().join("x.service");
        let planted = path.with_extension(format!("tmp{}", std::process::id()));
        std::os::unix::fs::symlink(&sentinel, &planted).unwrap();

        write_registration(&path, "unit").unwrap();

        assert_eq!(std::fs::read_to_string(&sentinel).unwrap(), "keep");
        let sentinel_mode = std::fs::metadata(&sentinel).unwrap().permissions().mode();
        assert_eq!(sentinel_mode & 0o777, 0o600);
        assert!(std::fs::symlink_metadata(&planted).unwrap().is_symlink());
        assert_eq!(read_registration(&path).as_deref(), Some("unit"));
        let meta = std::fs::symlink_metadata(&path).unwrap();
        assert!(meta.is_file());
        assert_eq!(meta.permissions().mode() & 0o777, 0o644);
    }

    #[cfg(unix)]
    #[test]
    fn an_init_script_is_executable_and_leaves_no_temp_file() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("S96iroh-tunnel-serve");
        write_registration(&path, "#!/bin/sh").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn an_absent_registration_reads_as_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_registration(&dir.path().join("none")), None);
    }
}
