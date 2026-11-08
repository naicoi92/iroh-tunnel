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
    /// The principal the service was installed for: the only one, besides
    /// the system account, whose writes the service accepts.
    pub owner: PinOwner,
}

/// Who a registration is pinned to: a Unix uid or a Windows SID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinOwner {
    /// Unix user id.
    Uid(u32),
    /// Windows security identifier in `S-1-…` form.
    Sid(String),
}

impl std::fmt::Display for PinOwner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Uid(uid) => write!(f, "uid {uid}"),
            Self::Sid(sid) => f.write_str(sid),
        }
    }
}

impl Pin {
    /// The record text (no comment delimiters).
    pub fn record(&self) -> String {
        let owner = match &self.owner {
            PinOwner::Uid(uid) => format!("uid={uid}"),
            PinOwner::Sid(sid) => format!("sid={sid}"),
        };
        format!("{KEY} config={} {owner}", escape(&self.config))
    }

    /// The uid of a Unix registration; `None` for a SID pin.
    #[cfg(unix)]
    pub(super) fn unix_uid(&self) -> Option<u32> {
        match self.owner {
            PinOwner::Uid(uid) => Some(uid),
            PinOwner::Sid(_) => None,
        }
    }

    /// Whether `other` pins the same registration target: exact on Unix,
    /// ASCII-case- and separator-insensitive on Windows paths.
    pub(crate) fn same_target(&self, other: &Pin) -> bool {
        same_target_with(cfg!(windows), self, other)
    }

    /// Find and decode the record inside a registration body. `None` when the
    /// body has no record (a registration from before this policy) or the
    /// record is malformed: the caller must treat both as an unknown target.
    pub fn find(body: &str) -> Option<Self> {
        // Only a comment line counts: a path inside another directive (the
        // busybox `CONFIG=` line) could otherwise embed the key and forge it.
        let line = body.lines().find(|l| {
            let l = l.trim_start();
            ["# ", "<!-- "]
                .iter()
                .any(|marker| l.strip_prefix(marker).is_some_and(|r| r.starts_with(KEY)))
        })?;
        let rest = &line[line.find(KEY)? + KEY.len()..];
        let mut config = None;
        let mut uid = None;
        let mut sid = None;
        for field in rest.split_whitespace() {
            let field = field.trim_end_matches("-->");
            if let Some(value) = field.strip_prefix("config=") {
                config = unescape(value);
            } else if let Some(value) = field.strip_prefix("uid=") {
                uid = Some(value.parse().ok()?);
            } else if let Some(value) = field.strip_prefix("sid=") {
                sid = Some(value.to_owned());
            }
        }
        let owner = match (uid, sid) {
            (Some(uid), None) => PinOwner::Uid(uid),
            (None, Some(sid)) if super::trust_windows::model::parse_sid(&sid).is_some() => {
                PinOwner::Sid(sid)
            }
            _ => return None,
        };
        Some(Self {
            config: config?,
            owner,
        })
    }
}

/// Reject a SID pin handed to a Unix backend, before anything is written.
#[cfg(unix)]
pub(super) fn require_unix_pin(pin: Option<&Pin>) -> anyhow::Result<()> {
    if pin.is_some_and(|p| p.unix_uid().is_none()) {
        return Err(crate::error::CliError::Service(
            "internal: Unix registration with a SID pin".into(),
        )
        .into());
    }
    Ok(())
}

/// [`Pin::same_target`] with the platform made explicit, so the Windows rule
/// is testable everywhere.
#[cfg_attr(not(unix), allow(dead_code))]
fn same_target_with(windows: bool, a: &Pin, b: &Pin) -> bool {
    if a.owner != b.owner {
        return false;
    }
    if !windows {
        return a.config == b.config;
    }
    let norm = |p: &Path| {
        p.to_string_lossy()
            .chars()
            .map(|c| {
                if c == '/' {
                    '\\'
                } else {
                    c.to_ascii_lowercase()
                }
            })
            .collect::<String>()
    };
    norm(&a.config) == norm(&b.config)
}

/// Consent to retarget, valid only against the registration it was given for.
///
/// `previous` is `None` when the registration being replaced is a legacy
/// unpinned one, so consent can still be given for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetargetConsent {
    /// The pin of the registration being replaced; `None` for a legacy unpinned one.
    pub previous: Option<Pin>,
    /// The pin the new registration would carry.
    pub proposed: Pin,
}

/// Whether an install may replace the existing registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetargetDecision {
    Proceed,
    Needs {
        previous: Option<Pin>,
        proposed: Pin,
    },
}

/// Pure retarget policy: nothing installed or the same owner proceeds; another
/// owner (or an unpinned registration) proceeds only with consent that matches
/// the current registration and the proposed pin.
pub(crate) fn retarget_decision_for(
    existing: Option<&str>,
    proposed: Pin,
    consent: Option<&RetargetConsent>,
) -> RetargetDecision {
    let Some(body) = existing else {
        return RetargetDecision::Proceed;
    };
    let previous = Pin::find(body);
    // Same owner proceeds whatever the config path. A SID pin never equals a
    // uid, so across platforms it counts as an unknown owner.
    if previous.as_ref().is_some_and(|p| p.owner == proposed.owner) {
        return RetargetDecision::Proceed;
    }
    let same_previous = |c: &RetargetConsent| match (&c.previous, &previous) {
        (None, None) => true,
        (Some(a), Some(b)) => a.same_target(b),
        _ => false,
    };
    if consent.is_some_and(|c| same_previous(c) && c.proposed.same_target(&proposed)) {
        return RetargetDecision::Proceed;
    }
    RetargetDecision::Needs { previous, proposed }
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
            owner: PinOwner::Uid(1000),
        }
    }

    fn sid_pin(path: &str) -> Pin {
        Pin {
            config: PathBuf::from(path),
            owner: PinOwner::Sid("S-1-5-21-1-2-3-1001".into()),
        }
    }

    fn consent(previous: Option<Pin>, proposed: Pin) -> RetargetConsent {
        RetargetConsent { previous, proposed }
    }

    fn sid(n: u32) -> PinOwner {
        PinOwner::Sid(format!("S-1-5-21-1-2-3-{n}"))
    }

    #[test]
    fn sid_mismatch_needs_consent_naming_the_previous_owner() {
        let alice = Pin {
            owner: sid(1001),
            ..sid_pin("C:\\a.toml")
        };
        let bob = Pin {
            owner: sid(1002),
            ..sid_pin("C:\\b.toml")
        };
        let body = format!("# {}\n", alice.record());
        assert_eq!(
            retarget_decision_for(Some(&body), bob.clone(), None),
            RetargetDecision::Needs {
                previous: Some(alice),
                proposed: bob
            }
        );
    }

    #[test]
    fn same_sid_proceeds_whatever_the_path() {
        let old = sid_pin("C:\\old.toml");
        let body = format!("# {}\n", old.record());
        assert_eq!(
            retarget_decision_for(Some(&body), sid_pin("C:\\new.toml"), None),
            RetargetDecision::Proceed
        );
    }

    #[test]
    fn consent_matches_a_case_differing_path_on_windows_only() {
        let previous = Pin {
            owner: sid(1001),
            ..sid_pin("C:\\Users\\A\\Serve.toml")
        };
        let given_for = Pin {
            owner: sid(1002),
            ..sid_pin("C:\\Users\\B\\Serve.toml")
        };
        let proposed = Pin {
            owner: sid(1002),
            ..sid_pin("c:/users/b/serve.toml")
        };
        assert!(same_target_with(true, &given_for, &proposed));
        let body = format!("# {}\n", previous.record());
        let given = consent(Some(previous), given_for);
        let decision = retarget_decision_for(Some(&body), proposed, Some(&given));
        assert_eq!(
            decision == RetargetDecision::Proceed,
            cfg!(windows),
            "{decision:?}"
        );
    }

    #[test]
    fn empty_body_is_an_unknown_owner() {
        let proposed = sid_pin("C:\\a.toml");
        assert_eq!(
            retarget_decision_for(Some(""), proposed.clone(), None),
            RetargetDecision::Needs {
                previous: None,
                proposed
            }
        );
    }

    #[test]
    fn sid_record_round_trips() {
        let p = sid_pin("C:\\Users\\a\\serve.toml");
        assert!(p.record().ends_with(" sid=S-1-5-21-1-2-3-1001"));
        assert_eq!(Pin::find(&format!("# {}\n", p.record())), Some(p));
    }

    #[test]
    fn legacy_uid_record_still_parses() {
        assert_eq!(
            Pin::find("# iroh-tunnel-pin config=/home/a/c.toml uid=1000"),
            Some(pin("/home/a/c.toml"))
        );
        assert_eq!(
            pin("/home/a/c.toml").record(),
            "iroh-tunnel-pin config=/home/a/c.toml uid=1000"
        );
    }

    #[test]
    fn record_with_both_uid_and_sid_is_malformed() {
        assert_eq!(
            Pin::find("# iroh-tunnel-pin config=/a uid=1 sid=S-1-5-18"),
            None
        );
    }

    #[test]
    fn pin_compares_paths_case_insensitively_on_windows() {
        let a = sid_pin("C:\\Users\\A\\Serve.toml");
        let b = sid_pin("c:/users/a/serve.toml");
        assert!(same_target_with(true, &a, &b));
        assert!(!same_target_with(false, &a, &b));
        let other = Pin {
            owner: PinOwner::Sid("S-1-5-18".into()),
            ..b.clone()
        };
        assert!(!same_target_with(true, &a, &other));
    }

    #[test]
    fn pin_round_trips_unicode_path() {
        let p = sid_pin("C:\\Users\\Nguyễn Văn\\AppData\\Local\\iroh-tunnel\\config\\serve.toml");
        assert_eq!(Pin::find(&format!("# {}\n", p.record())), Some(p));
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
    fn a_path_embedding_the_key_cannot_forge_the_pin() {
        let forged = pin("/x/iroh-tunnel-pin config=/evil uid=0/serve.toml");
        let body = format!(
            "CONFIG=\"{}\"\n# {}\n",
            forged.config.display(),
            forged.record()
        );
        assert_eq!(Pin::find(&body), Some(forged));
        assert_eq!(
            Pin::find("CONFIG=\"/x/iroh-tunnel-pin config=/evil uid=0\"\n"),
            None
        );
    }

    #[test]
    fn empty_or_malformed_sid_is_unknown_not_a_pin() {
        assert_eq!(Pin::find("# iroh-tunnel-pin config=C%3A%5Ca sid="), None);
        assert_eq!(
            Pin::find("# iroh-tunnel-pin config=C%3A%5Ca sid=garbage"),
            None
        );
        assert_eq!(
            Pin::find("# iroh-tunnel-pin config=C%3A%5Ca sid=S-1-5"),
            None
        );
        assert!(Pin::find("# iroh-tunnel-pin config=C%3A%5Ca sid=S-1-5-18").is_some());
    }

    #[cfg(unix)]
    #[test]
    fn require_unix_pin_rejects_only_a_sid_pin() {
        assert!(require_unix_pin(None).is_ok());
        assert!(require_unix_pin(Some(&pin("/a"))).is_ok());
        let err = require_unix_pin(Some(&sid_pin("/a"))).unwrap_err();
        assert!(matches!(
            err.downcast_ref::<crate::error::CliError>(),
            Some(crate::error::CliError::Service(_))
        ));
    }

    #[test]
    fn windows_target_match_is_exact_apart_from_ascii_case_and_separators() {
        assert!(!same_target_with(
            true,
            &sid_pin("C:\\a"),
            &sid_pin("C:\\a\\")
        ));
        assert!(!same_target_with(
            true,
            &sid_pin("C:\\Ä\\s.toml"),
            &sid_pin("C:\\ä\\s.toml")
        ));
    }

    #[test]
    fn missing_or_malformed_record_is_unknown_not_a_match() {
        assert_eq!(Pin::find("[Service]\nExecStart=x\n"), None);
        assert_eq!(Pin::find("# iroh-tunnel-pin config=/a uid=x"), None);
        assert_eq!(Pin::find("# iroh-tunnel-pin config=%ZZ uid=1"), None);
    }
}
