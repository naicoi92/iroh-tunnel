//! The pin recorded in a service registration (SM-02, SM-04; issue #91).
//!
//! A system service runs as root but reads the installing user's
//! configuration. The registration is root-owned, so it is the one place that
//! can safely record *whose* files the service may trust: the installer writes
//! the absolute config path and the trusted owner UID into it, and a later
//! install reads them back to detect that it would retarget the service.
//!
//! The record is a single comment line so every backend (systemd unit, launchd
//! plist, BusyBox script) can carry it without a format of its own:
//! `iroh-tunnel-pin config=<escaped path> uid=<n>`. The path is escaped to a
//! conservative alphabet, which also keeps it legal inside an XML comment.

use std::path::{Path, PathBuf};

const KEY: &str = "iroh-tunnel-pin";

/// What a registration pins: the config file and the user it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    /// Absolute, canonical path of the config file the service reads.
    pub config: PathBuf,
    /// Owner UID of the config directory at install time: the only principal,
    /// besides root, whose writes the service accepts.
    pub uid: u32,
}

impl Pin {
    /// The record text (no comment delimiters).
    pub fn record(&self) -> String {
        format!("{KEY} config={} uid={}", escape(&self.config), self.uid)
    }

    /// Find and decode the record inside a registration body. `None` when the
    /// body has no record (a registration from before this policy) or the
    /// record is malformed: the caller must treat both as an unknown target.
    pub fn find(body: &str) -> Option<Self> {
        let line = body.lines().find(|l| l.contains(KEY))?;
        let rest = &line[line.find(KEY)? + KEY.len()..];
        let mut config = None;
        let mut uid = None;
        for field in rest.split_whitespace() {
            let field = field.trim_end_matches("-->");
            if let Some(value) = field.strip_prefix("config=") {
                config = unescape(value);
            } else if let Some(value) = field.strip_prefix("uid=") {
                uid = value.parse().ok();
            }
        }
        Some(Self {
            config: config?,
            uid: uid?,
        })
    }
}

/// Escape to `[A-Za-z0-9/._]`; everything else, including `-` (illegal as `--`
/// in an XML comment) and whitespace, becomes `%XX` per byte.
fn escape(path: &Path) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for &byte in path.as_os_str().as_encoded_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

#[cfg(unix)]
fn unescape(text: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
}

#[cfg(not(unix))]
fn unescape(text: &str) -> Option<PathBuf> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            out.push(u8::from_str_radix(text.get(i + 1..i + 3)?, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok().map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin(path: &str) -> Pin {
        Pin {
            config: PathBuf::from(path),
            uid: 1000,
        }
    }

    #[test]
    fn record_round_trips_awkward_paths() {
        for path in [
            "/home/a/.config/iroh-tunnel/serve.toml",
            "/a b/it's -- 100%.toml",
        ] {
            let record = format!("# {}\n", pin(path).record());
            assert_eq!(Pin::find(&record), Some(pin(path)), "{path}");
        }
    }

    #[test]
    fn record_is_legal_inside_an_xml_comment() {
        let record = pin("/a b/--x.toml").record();
        assert!(!record.contains("--"), "{record}");
        let plist = format!("<!-- {record} -->\n<dict/>");
        assert_eq!(Pin::find(&plist), Some(pin("/a b/--x.toml")));
    }

    #[test]
    fn missing_or_malformed_record_is_unknown_not_a_match() {
        assert_eq!(Pin::find("[Service]\nExecStart=x\n"), None);
        assert_eq!(Pin::find("# iroh-tunnel-pin config=/a uid=x"), None);
        assert_eq!(Pin::find("# iroh-tunnel-pin config=%ZZ uid=1"), None);
    }
}
