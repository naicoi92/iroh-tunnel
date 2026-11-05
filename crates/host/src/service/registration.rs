//! Registration-file helpers shared by the backends (private).

use std::path::Path;

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
    let temp = path.with_extension(format!("tmp{}", std::process::id()));
    let written = (|| {
        std::fs::write(&temp, contents)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = if path.extension().is_none() {
                0o755
            } else {
                0o644
            };
            std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(mode))?;
        }
        std::fs::rename(&temp, path)
    })();
    written.map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        denied(e, "writing", path)
    })
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

    #[test]
    fn an_absent_registration_reads_as_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_registration(&dir.path().join("none")), None);
    }
}
