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
    /// Open (creating `dir` when missing) the log of `role`. `None` when the
    /// file cannot be opened.
    #[must_use]
    pub fn open(dir: &Path, role: &str, cap: u64) -> Option<CappedLog> {
        let mut log = CappedLog {
            dir: dir.to_path_buf(),
            path: dir.join(format!("{role}.log")),
            cap,
            written: 0,
            file: None,
            checked: Instant::now(),
        };
        log.reopen();
        log.file.is_some().then_some(log)
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
        if self.file.is_none() || (self.checked.elapsed() >= RECHECK && !self.path.exists()) {
            self.reopen();
        }
        if self.checked.elapsed() >= RECHECK {
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
            // on the next write instead of statting the path every time.
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
#[must_use]
pub fn machine_log_dir() -> Option<PathBuf> {
    let machine = crate::ownership::machine_dir()?;
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
        let mut log = CappedLog::open(tmp.path(), "serve", 1000).unwrap();
        log.write_all(b"one\n").unwrap();
        log.write_all(b"two\n").unwrap();
        drop(log);
        let mut again = CappedLog::open(tmp.path(), "serve", 1000).unwrap();
        again.write_all(b"three\n").unwrap();
        let text = std::fs::read_to_string(tmp.path().join("serve.log")).unwrap();
        assert_eq!(text, "one\ntwo\nthree\n");
    }

    #[test]
    fn rotates_at_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let mut log = CappedLog::open(tmp.path(), "serve", 100).unwrap();
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
        let mut log = CappedLog::open(tmp.path(), "serve", 100).unwrap();
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
        let mut log = CappedLog::open(&logs, "serve", 100).unwrap();
        std::fs::remove_dir_all(&logs).unwrap();
        log.checked -= RECHECK; // the periodic path check is due
        log.write_all(b"after\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(logs.join("serve.log")).unwrap(),
            "after\n"
        );
    }

    #[test]
    fn log_writer_open_failure_is_not_fatal() {
        let tmp = tempfile::tempdir().unwrap();
        let logs = tmp.path().join("logs");
        std::fs::write(&logs, "not a directory").unwrap();
        assert!(CappedLog::open(&logs, "serve", 100).is_none());
        // Writing after the directory became a file must still succeed.
        let sub = tmp.path().join("sub");
        let mut log = CappedLog::open(&sub, "serve", 100).unwrap();
        std::fs::remove_dir_all(&sub).unwrap();
        std::fs::write(&sub, "x").unwrap();
        assert_eq!(log.write(b"dropped").unwrap(), 7);
    }
}
