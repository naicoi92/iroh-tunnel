//! Standalone executable replacement (DIST-02, issue #80 policy).
//!
//! The verified file is staged beside the installed executable and renamed
//! over it. Any failure reports the OS error and leaves the installed
//! executable untouched; a running process keeps its already-open image.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::Context as _;

/// Staging file beside the target; removed on drop unless [`disarm`](Self::disarm)ed
/// after a successful rename.
struct StagedFile {
    path: PathBuf,
    armed: std::cell::Cell<bool>,
}

impl StagedFile {
    fn create(dir: &Path, target: &Path) -> anyhow::Result<Self> {
        let name = format!(".iroh-tunnel-update-{}.tmp", std::process::id());
        let path = dir.join(name);
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .with_context(|| format!("cannot stage update beside {}", target.display()))?;
        Ok(Self {
            path,
            armed: std::cell::Cell::new(true),
        })
    }

    fn write(
        &self,
        contents: &[u8],
        permissions: &std::fs::Permissions,
        target: &Path,
    ) -> anyhow::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(&self.path)
            .with_context(|| format!("cannot write staged update for {}", target.display()))?;
        file.write_all(contents)
            .and_then(|()| file.sync_all())
            .with_context(|| format!("cannot write staged update for {}", target.display()))?;
        drop(file);
        std::fs::set_permissions(&self.path, permissions.clone()).with_context(|| {
            format!(
                "cannot set permissions on staged update for {}",
                target.display()
            )
        })
    }

    fn disarm(&self) {
        self.armed.set(false);
    }
}

impl Drop for StagedFile {
    fn drop(&mut self) {
        if self.armed.get() {
            // Best effort: the original error is what the caller reports.
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Replace the installed executable at `target` with `contents`.
///
/// The bytes are written to a temporary file in the same directory (so the
/// final rename stays on one filesystem), given the permissions of the file
/// being replaced, then renamed over it.
///
/// # Errors
///
/// Returns the OS error, with context, when `target` does not exist, the
/// staging file cannot be created, written or given permissions, or the
/// rename fails (for example a file the OS keeps locked on Windows). In every
/// case the installed executable and its directory are left as they were.
pub fn replace_executable(target: &Path, contents: &[u8]) -> anyhow::Result<()> {
    let installed = std::fs::metadata(target)
        .with_context(|| format!("cannot read installed executable {}", target.display()))?;
    let dir = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));

    let staged = StagedFile::create(dir, target)?;
    staged.write(contents, &installed.permissions(), target)?;
    std::fs::rename(&staged.path, target)
        .with_context(|| format!("cannot replace {}", target.display()))?;
    staged.disarm();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_executable_content_and_keeps_no_staging_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("iroh-tunnel-serve");
        std::fs::write(&target, b"old").unwrap();

        replace_executable(&target, b"new").unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(entries.len(), 1, "staging file must not remain");
    }

    #[cfg(unix)]
    #[test]
    fn preserves_executable_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("it");
        std::fs::write(&target, b"old").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();

        replace_executable(&target, b"new").unwrap();

        let mode = std::fs::metadata(&target).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }

    #[cfg(unix)]
    #[test]
    fn failure_to_stage_reports_error_and_keeps_old_binary() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("it");
        std::fs::write(&target, b"old").unwrap();
        // Directory not writable: staging beside the target must fail.
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
        if std::fs::File::create(dir.path().join("probe")).is_ok() {
            return; // running as root: permission cannot be denied
        }

        let err = replace_executable(&target, b"new").unwrap_err();

        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        assert!(format!("{err:#}").contains("stage"), "{err:#}");
    }

    #[test]
    fn missing_target_is_an_error_and_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("absent");

        assert!(replace_executable(&target, b"new").is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
