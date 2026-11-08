//! Replacing the installed executable on disk (private infrastructure of
//! `update`).

use std::path::{Path, PathBuf};

use std::io::Write as _;

use anyhow::Context as _;

use crate::error::CliError;

/// What to do when the OS denies replacing the executable. On Windows "access
/// denied" is also what a running image reports, and `sudo` does not exist.
#[cfg(windows)]
const REPLACE_DENIED_HINT: &str =
    "if this executable is running (a tunnel, or this very command), \
     stop it and run the update again from another copy; otherwise check the file permissions";

/// Appended on Windows: a system service holds its executable open, and only
/// an elevated `service stop --system` releases it. Both role commands are
/// named because `update` does not know which role the service runs, and a
/// `<role>` placeholder would break copy-paste (`<` redirects in shells).
#[cfg_attr(not(windows), allow(dead_code))]
pub(super) fn windows_locked_hint() -> &'static str {
    "; if a system service runs this executable, stop it first from an elevated terminal: \
     iroh-tunnel serve service stop --system or iroh-tunnel access service stop --system"
}

#[cfg(not(windows))]
const REPLACE_DENIED_HINT: &str = "rerun with sufficient privileges (for example sudo)";

/// A permission failure while replacing is reported as [`CliError::Permission`]
/// (exit 3, RS-06); everything else keeps its unclassified exit 1.
pub(super) fn classify_replace_error(err: anyhow::Error) -> anyhow::Error {
    let denied = err.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|e| e.kind() == std::io::ErrorKind::PermissionDenied)
    });
    if denied {
        let extra = if cfg!(windows) {
            windows_locked_hint()
        } else {
            ""
        };
        CliError::Permission(format!("{err:#}; {REPLACE_DENIED_HINT}{extra}")).into()
    } else {
        err
    }
}

/// Staging file beside the target; removed on drop unless
/// [`disarm`](Self::disarm)ed after a successful rename.
struct StagedFile {
    path: PathBuf,
    armed: std::cell::Cell<bool>,
}

impl StagedFile {
    /// Create the file exclusively (`O_EXCL`, so a pre-planted file or symlink
    /// is never followed) and write `contents` through that single handle.
    fn create(
        dir: &Path,
        target: &Path,
        contents: &[u8],
        permissions: &std::fs::Permissions,
    ) -> anyhow::Result<Self> {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let seq = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = dir.join(format!(
            ".iroh-tunnel-update-{}-{nonce:08x}-{seq}.tmp",
            std::process::id()
        ));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .with_context(|| format!("cannot stage update beside {}", target.display()))?;
        let staged = Self {
            path,
            armed: std::cell::Cell::new(true),
        };
        file.set_permissions(permissions.clone()).with_context(|| {
            format!(
                "cannot set permissions on staged update for {}",
                target.display()
            )
        })?;
        file.write_all(contents)
            .and_then(|()| file.sync_all())
            .with_context(|| format!("cannot write staged update for {}", target.display()))?;
        Ok(staged)
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
/// rename fails (for example a file the OS keeps locked on Windows, where
/// replacing the executable that is currently running is expected to fail with
/// the OS error; policy #80, covered by Windows runner evidence in #97). In every
/// case the installed executable and its directory are left as they were.
pub fn replace_executable(target: &Path, contents: &[u8]) -> anyhow::Result<()> {
    let installed = std::fs::metadata(target)
        .with_context(|| format!("cannot read installed executable {}", target.display()))?;
    let dir = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));

    let staged = StagedFile::create(dir, target, contents, &installed.permissions())?;
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

    #[test]
    fn permission_denied_becomes_exit_class_permission() {
        let io = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let err = classify_replace_error(anyhow::Error::new(io).context("cannot stage"));
        assert!(matches!(
            err.downcast_ref::<CliError>(),
            Some(CliError::Permission(_))
        ));
        let other = classify_replace_error(anyhow::anyhow!("boom"));
        assert!(other.downcast_ref::<CliError>().is_none());
    }

    #[test]
    fn windows_locked_hint_mentions_service_stop_system() {
        let hint = windows_locked_hint();
        assert!(
            hint.contains("iroh-tunnel serve service stop --system"),
            "{hint}"
        );
        assert!(
            hint.contains("iroh-tunnel access service stop --system"),
            "{hint}"
        );
        assert!(!hint.contains("<role>"), "{hint}");
        assert!(hint.contains("elevated"), "{hint}");
    }

    #[test]
    fn staging_names_are_unique() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("t");
        std::fs::write(&target, b"o").unwrap();
        let perms = std::fs::metadata(&target).unwrap().permissions();
        let a = StagedFile::create(dir.path(), &target, b"a", &perms).unwrap();
        let b = StagedFile::create(dir.path(), &target, b"b", &perms).unwrap();
        assert_ne!(a.path, b.path);
    }
}
