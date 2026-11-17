//! Size-capped service log: one file per role plus a single `.1` rotation.
//!
//! The service has no console, so its tracing output goes here. Every error
//! is swallowed: logging must never stop the service.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How often a healthy handle is checked against the path (an unlinked file
/// still accepts writes on Unix).
const RECHECK: Duration = Duration::from_secs(1);

/// An append-only `<role>.log` that rotates to `<role>.log.1` before a write
/// would pass the cap.
#[derive(Debug)]
pub struct CappedLog {
    dir: PathBuf,
    path: PathBuf,
    cap: u64,
    written: u64,
    file: Option<File>,
    /// Last time the path was confirmed to still exist.
    checked: Instant,
}

impl CappedLog {
    /// Open (creating `dir` when missing) the log of `role`. A file that
    /// cannot be opened yet is retried by later writes, at most once per
    /// second, so a transient startup failure does not silence the whole run.
    #[must_use]
    pub fn open(dir: &Path, role: &str, cap: u64) -> CappedLog {
        let mut log = CappedLog {
            dir: dir.to_path_buf(),
            path: dir.join(format!("{role}.log")),
            cap,
            written: 0,
            file: None,
            checked: Instant::now(),
        };
        log.reopen();
        log
    }

    fn reopen(&mut self) {
        self.file = None;
        if std::fs::create_dir_all(&self.dir).is_err() {
            return;
        }
        if let Ok(file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            self.written = file.metadata().map_or(0, |m| m.len());
            self.file = Some(file);
        }
    }

    /// Start the active file over. Used when rotation failed: the file is
    /// emptied instead of growing past the cap. When even that fails the
    /// writer drops records until a later rotation succeeds.
    fn truncate(&mut self) {
        self.file = None;
        self.written = 0;
        if let Ok(file) = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.path)
        {
            self.file = Some(file);
        }
    }

    fn rotate(&mut self) {
        self.file = None;
        let mut rotated = self.path.clone().into_os_string();
        rotated.push(".1");
        // Windows `rename` replaces an existing file; remove first elsewhere
        // so the behaviour is the same on every platform.
        let _ = std::fs::remove_file(&rotated);
        let _ = std::fs::rename(&self.path, &rotated);
        self.written = 0;
        self.reopen();
        if self.written > 0 {
            // The rename failed (`.1` locked or a directory): bound the file.
            self.truncate();
        }
    }
}

impl Write for CappedLog {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // A missing handle or path is retried at most once per RECHECK, so
        // a log that stays unopenable does not cost an open per record.
        if self.checked.elapsed() >= RECHECK {
            if self.file.is_none() || !self.path.exists() {
                self.reopen();
            }
            self.checked = Instant::now();
        }
        if self.file.is_some() && self.written > 0 && self.written + buf.len() as u64 > self.cap {
            self.rotate();
        }
        let wrote = self
            .file
            .as_mut()
            .is_some_and(|file| file.write_all(buf).is_ok());
        if wrote {
            self.written += buf.len() as u64;
        } else if self.file.is_some() {
            // The handle went bad (directory removed, disk error): recreate
            // it on the next due check instead of statting the path every time.
            self.file = None;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(file) = &mut self.file {
            let _ = file.flush();
        }
        Ok(())
    }
}

/// The machine log directory, `<machine dir>/logs`. On Windows the machine
/// directory is created when missing; it is `None` when an existing one must
/// be trust-verified and does not pass, or cannot be created.
///
/// Log selection is best-effort: an unresolvable machine directory yields
/// `None` here, while ownership still fails closed when the role claims.
#[must_use]
pub fn machine_log_dir() -> Option<PathBuf> {
    let machine = crate::ownership::machine_dir().ok().flatten()?;
    #[cfg(windows)]
    crate::ownership::ensure_machine_dir(&machine).ok()?;
    Some(machine.path.join("logs"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_append_to_role_log() {
        let tmp = tempfile::tempdir().unwrap();
        let mut log = CappedLog::open(tmp.path(), "serve", 1000);
        log.write_all(b"one\n").unwrap();
        log.write_all(b"two\n").unwrap();
        drop(log);
        let mut again = CappedLog::open(tmp.path(), "serve", 1000);
        again.write_all(b"three\n").unwrap();
        let text = std::fs::read_to_string(tmp.path().join("serve.log")).unwrap();
        assert_eq!(text, "one\ntwo\nthree\n");
    }

    #[test]
    fn rotates_at_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let mut log = CappedLog::open(tmp.path(), "serve", 100);
        for _ in 0..15 {
            log.write_all(b"0123456789").unwrap();
        }
        assert!(tmp.path().join("serve.log.1").exists());
        let len = std::fs::metadata(tmp.path().join("serve.log"))
            .unwrap()
            .len();
        assert!(len < 100, "{len}");
    }

    #[test]
    fn failed_rotation_keeps_the_log_within_the_cap() {
        let tmp = tempfile::tempdir().unwrap();
        // A directory at `.1` makes every rename fail.
        std::fs::create_dir(tmp.path().join("serve.log.1")).unwrap();
        let mut log = CappedLog::open(tmp.path(), "serve", 100);
        for _ in 0..50 {
            log.write_all(b"0123456789").unwrap();
            let len = std::fs::metadata(tmp.path().join("serve.log"))
                .unwrap()
                .len();
            assert!(len <= 100 + 10, "{len}");
        }
    }

    #[test]
    fn log_writer_recreates_missing_logs_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let logs = tmp.path().join("logs");
        let mut log = CappedLog::open(&logs, "serve", 100);
        std::fs::remove_dir_all(&logs).unwrap();
        log.checked -= RECHECK; // the periodic path check is due
        log.write_all(b"after\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(logs.join("serve.log")).unwrap(),
            "after\n"
        );
    }

    #[test]
    fn log_writer_retries_a_failed_startup_open() {
        let tmp = tempfile::tempdir().unwrap();
        let logs = tmp.path().join("logs");
        std::fs::write(&logs, "not a directory").unwrap();
        let mut log = CappedLog::open(&logs, "serve", 100);
        assert_eq!(log.write(b"dropped\n").unwrap(), 8);
        // Retries are throttled: not every record pays for a failing open.
        std::fs::remove_file(&logs).unwrap();
        log.write_all(b"throttled\n").unwrap();
        assert!(!logs.exists(), "reopened before the retry was due");
        // Once the retry is due the next record lands in the log.
        log.checked -= RECHECK;
        log.write_all(b"after\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(logs.join("serve.log")).unwrap(),
            "after\n"
        );
    }

    #[test]
    fn log_writer_survives_a_dir_replaced_by_a_file() {
        let tmp = tempfile::tempdir().unwrap();
        // Writing after the directory became a file must still succeed.
        let sub = tmp.path().join("sub");
        let mut log = CappedLog::open(&sub, "serve", 100);
        std::fs::remove_dir_all(&sub).unwrap();
        std::fs::write(&sub, "x").unwrap();
        assert_eq!(log.write(b"dropped").unwrap(), 7);
    }
}
