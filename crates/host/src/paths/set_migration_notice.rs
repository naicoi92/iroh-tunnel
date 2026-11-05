//! Frontend hook for the macOS legacy-configuration migration notice.

use std::path::Path;

/// Observer told about a completed macOS legacy-configuration copy.
///
/// A CLI or other frontend installs one with [`set_migration_notice`] to show
/// the operator `legacy` and `dest`; the host itself never prints.
pub type MigrationNotice = fn(legacy: &Path, dest: &Path);

static MIGRATION_NOTICE: std::sync::OnceLock<MigrationNotice> = std::sync::OnceLock::new();

/// Install the observer for completed legacy-configuration copies, once.
///
/// Write-once: a second call is ignored. Without an observer the host logs
/// the copy at `info` level (`migrated configuration (original kept)`).
pub fn set_migration_notice(notice: MigrationNotice) {
    let _ = MIGRATION_NOTICE.set(notice);
}

pub(super) fn notify_migrated(legacy: &Path, dest: &Path) {
    match MIGRATION_NOTICE.get() {
        Some(notice) => notice(legacy, dest),
        None => tracing::info!(
            legacy = %legacy.display(),
            dest = %dest.display(),
            "migrated configuration (original kept)"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;
    use std::sync::Mutex;

    static SEEN: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

    fn record(_legacy: &Path, dest: &Path) {
        SEEN.lock().unwrap().push(dest.to_path_buf());
    }

    #[test]
    fn an_installed_notice_receives_completed_migrations() {
        set_migration_notice(record);
        notify_migrated(
            Path::new("/legacy.toml"),
            Path::new("/dest-from-notice-test.toml"),
        );
        assert!(SEEN
            .lock()
            .unwrap()
            .contains(&PathBuf::from("/dest-from-notice-test.toml")));
    }
}
