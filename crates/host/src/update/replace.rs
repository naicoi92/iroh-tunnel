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
/// an elevated `service stop --system` releases it. Standalone binaries name
/// their own command; the combined binary names both possible service roles.
pub(super) fn windows_locked_hint(exe_name: &str) -> String {
    let commands = match exe_name {
        "iroh-tunnel-serve" | "iroh-tunnel-access" => {
            format!("{exe_name} service stop --system")
        }
        _ => "iroh-tunnel serve service stop --system or \
              iroh-tunnel access service stop --system"
            .to_owned(),
    };
    format!(
        "; if a system service runs this executable, stop it first from an elevated terminal: \
         {commands}"
    )
}

#[cfg(not(windows))]
const REPLACE_DENIED_HINT: &str = "rerun with sufficient privileges (for example sudo)";

/// A permission failure while replacing is reported as [`CliError::Permission`]
/// (exit 3, RS-06); everything else keeps its unclassified exit 1.
pub(super) fn classify_replace_error(err: anyhow::Error, exe_name: &str) -> anyhow::Error {
    let denied = err.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|e| e.kind() == std::io::ErrorKind::PermissionDenied)
    });
    if denied {
        let extra = if cfg!(windows) {
            windows_locked_hint(exe_name)
        } else {
            String::new()
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
        let mut file = open_staging(&path)
            .with_context(|| format!("cannot stage update beside {}", target.display()))?;
        let staged = Self {
            path,
            armed: std::cell::Cell::new(true),
        };
        // Test-only: observe the mode before it is changed below (#230).
        #[cfg(all(test, unix))]
        tests::record_mode_at_open(&file);
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

/// Open `path` exclusively (`O_EXCL`) for writing, owner-only until the
/// caller sets the final mode on the handle, so a permissive umask never
/// opens a window for another user to grab a write fd.
fn open_staging(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
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
        let err = classify_replace_error(
            anyhow::Error::new(io).context("cannot stage"),
            "iroh-tunnel",
        );
        assert!(matches!(
            err.downcast_ref::<CliError>(),
            Some(CliError::Permission(_))
        ));
        let other = classify_replace_error(anyhow::anyhow!("boom"), "iroh-tunnel");
        assert!(other.downcast_ref::<CliError>().is_none());
    }

    #[test]
    fn windows_locked_hint_mentions_service_stop_system() {
        let hint = windows_locked_hint("iroh-tunnel");
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
    fn windows_locked_hint_for_serve_needs_no_combined_executable() {
        let hint = windows_locked_hint("iroh-tunnel-serve");
        assert!(
            hint.contains("iroh-tunnel-serve service stop --system"),
            "{hint}"
        );
        assert!(!hint.contains("iroh-tunnel serve"), "{hint}");
        assert!(!hint.contains("access"), "{hint}");
        assert!(hint.contains("elevated"), "{hint}");
    }

    #[test]
    fn windows_locked_hint_for_access_needs_no_combined_executable() {
        let hint = windows_locked_hint("iroh-tunnel-access");
        assert!(
            hint.contains("iroh-tunnel-access service stop --system"),
            "{hint}"
        );
        assert!(!hint.contains("iroh-tunnel access"), "{hint}");
        assert!(!hint.contains("serve"), "{hint}");
        assert!(hint.contains("elevated"), "{hint}");
    }

    #[cfg(unix)]
    thread_local! {
        /// Mode of the staging file right after `StagedFile::create` opened
        /// it, before the final permissions are applied.
        static MODE_AT_OPEN: std::cell::Cell<Option<u32>> =
            const { std::cell::Cell::new(None) };
    }

    #[cfg(unix)]
    pub(super) fn record_mode_at_open(file: &std::fs::File) {
        use std::os::unix::fs::PermissionsExt;
        let mode = file.metadata().ok().map(|m| m.permissions().mode());
        MODE_AT_OPEN.with(|cell| cell.set(mode));
    }

    /// Issue #230: the staging file is owner-only from the moment it exists,
    /// so a permissive umask never lets another user grab a write fd before
    /// the final permissions are set.
    #[cfg(unix)]
    #[test]
    fn staging_file_is_created_owner_only_under_a_permissive_umask() {
        const NAME: &str =
            "update::replace::tests::staging_file_is_created_owner_only_under_a_permissive_umask";
        // The umask is process-wide, so it is only changed in a child
        // process that runs this test alone.
        crate::fixtures::in_isolated_home(NAME, |home| {
            use std::os::unix::fs::PermissionsExt;
            // Explicit, so the test does not depend on the runner's umask (a
            // 077 umask would hide the bug); 022 exposes group/other bits on
            // a default-mode open.
            // SAFETY: umask(2) only changes this child process's file mode
            // creation mask; it has no memory-safety preconditions.
            unsafe { libc::umask(0o022) };
            let target = home.join("t");
            std::fs::write(&target, b"o").unwrap();
            let perms = std::fs::Permissions::from_mode(0o755);

            let staged = StagedFile::create(home, &target, b"new", &perms).unwrap();

            let at_open = MODE_AT_OPEN.with(std::cell::Cell::take).unwrap();
            assert_eq!(at_open & 0o077, 0, "mode at open {:o}", at_open & 0o777);
            let final_mode = std::fs::metadata(&staged.path)
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(final_mode & 0o777, 0o755);
        });
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
