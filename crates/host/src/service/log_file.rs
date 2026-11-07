//! Size-capped service log: one file per role plus a single `.1` rotation.
//!
//! The service has no console, so its tracing output goes here. Every error
//! is swallowed: logging must never stop the service.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// An append-only `<role>.log` that rotates to `<role>.log.1` before a write
/// would pass the cap.
#[derive(Debug)]
pub struct CappedLog {
    dir: PathBuf,
    path: PathBuf,
    cap: u64,
    written: u64,
    file: Option<File>,
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
    }
}

impl Write for CappedLog {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.file.is_none() || !self.path.exists() {
            self.reopen();
        }
        if self.file.is_some() && self.written > 0 && self.written + buf.len() as u64 > self.cap {
            self.rotate();
        }
        if let Some(file) = &mut self.file {
            if file.write_all(buf).is_ok() {
                self.written += buf.len() as u64;
            }
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

/// The machine log directory, `<machine dir>/logs`. On Windows it is `None`
/// when the machine directory does not pass the machine trust check.
#[must_use]
pub fn machine_log_dir() -> Option<PathBuf> {
    let dir = crate::ownership::machine_dir()?;
    #[cfg(windows)]
    super::verify_machine_dir(&dir).ok()?;
    Some(dir.join("logs"))
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
    fn log_writer_recreates_missing_logs_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let logs = tmp.path().join("logs");
        let mut log = CappedLog::open(&logs, "serve", 100).unwrap();
        std::fs::remove_dir_all(&logs).unwrap();
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
