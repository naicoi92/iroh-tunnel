//! Filesystem primitives shared by the host's security-sensitive writers.

use std::path::{Path, PathBuf};

/// Drop a leading `\\?\` (verbatim) prefix that `canonicalize` adds on
/// Windows, so a pinned or displayed path matches what the user would type.
/// A verbatim UNC path (`\\?\UNC\…`) is left alone: the trust walk refuses it.
#[cfg_attr(
    all(not(test), not(windows)),
    expect(dead_code, reason = "only Windows paths carry a verbatim prefix")
)]
pub(crate) fn strip_verbatim_prefix(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with(r"UNC\") => PathBuf::from(rest),
        _ => path,
    }
}

/// Create `path` exclusively (`O_EXCL`: a pre-planted file or symlink is
/// never followed, truncated or chmodded) and open it for writing.
///
/// On unix the file is created `0600`, so a permissive umask never opens a
/// window for another user to grab a write fd before the caller sets the
/// final mode on the returned handle. Elsewhere the file inherits the
/// parent directory's permissions.
///
/// # Errors
///
/// Returns the OS error unchanged, including
/// [`AlreadyExists`](std::io::ErrorKind::AlreadyExists) when `path` exists.
pub(crate) fn create_private_new(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue #232: the shared staging open is owner-only from the moment the
    /// file exists, whatever the process umask.
    #[cfg(unix)]
    #[test]
    fn creates_owner_only_file_under_a_permissive_umask() {
        const NAME: &str = "fs_util::tests::creates_owner_only_file_under_a_permissive_umask";
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

            let file = create_private_new(&home.join("staged")).unwrap();

            let mode = file.metadata().unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "mode {:o}", mode & 0o777);
        });
    }

    #[test]
    fn refuses_an_existing_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("planted");
        std::fs::write(&path, b"keep").unwrap();

        let err = create_private_new(&path).unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&path).unwrap(), b"keep");
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_planted_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim");
        std::fs::write(&victim, b"keep").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&victim, &link).unwrap();

        let err = create_private_new(&link).unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&victim).unwrap(), b"keep");
    }
}
