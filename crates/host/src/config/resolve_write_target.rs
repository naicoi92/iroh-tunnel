//! The file a config write would actually replace.

use std::path::{Path, PathBuf};

use super::store::resolve_target;

/// Follow symlinks from `path` to the file a write would actually replace
/// (a dangling link resolves to its target). Pure path arithmetic: nothing is
/// created or locked.
pub fn resolve_write_target(path: &Path) -> PathBuf {
    resolve_target(path)
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::*;

    #[cfg(unix)]
    #[test]
    fn a_symlinked_config_resolves_to_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.toml");
        std::fs::write(&real, "").unwrap();
        let link = dir.path().join("link.toml");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert_eq!(
            resolve_write_target(&link).canonicalize().unwrap(),
            real.canonicalize().unwrap()
        );
    }
}
