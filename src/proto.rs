//! ALPN convention for iroh-tunnel streams.
//!
//! Each service is addressed by an ALPN byte string of the form
//! `iroh-tunnel/{name}`. The fixed prefix namespaces our streams so they
//! can't collide with other protocols multiplexed on the same QUIC
//! connection.
//!
//! Implements T-03 (Page 05 v3 §5.1).
//
// Consumed by the serve/access handlers (T-06/T-07); flagged dead code until
// then by the binary crate's single-crate layout.
#![allow(dead_code)]

/// Fixed prefix for every service ALPN.
pub const ALPN_PREFIX: &str = "iroh-tunnel/";

/// Build the ALPN byte string for a service name.
///
/// `name` is expected to already be validated (lowercase, `[a-z0-9-]`, ≤ 63
/// bytes — see `config`). No validation is done here on purpose: this is the
/// hot path for stream setup and should stay allocation-cheap.
pub fn alpn_for(name: &str) -> Vec<u8> {
    format!("{ALPN_PREFIX}{name}").into_bytes()
}

/// Inverse of [`alpn_for`]: strip the prefix from an ALPN byte string and
/// return the service name. Returns `None` if the bytes are not valid UTF-8
/// or do not start with our prefix.
pub fn name_from_alpn(alpn: &[u8]) -> Option<&str> {
    let s = std::str::from_utf8(alpn).ok()?;
    s.strip_prefix(ALPN_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpn_for_builds_prefixed_string() {
        assert_eq!(alpn_for("postgres"), b"iroh-tunnel/postgres");
        assert_eq!(alpn_for("dns"), b"iroh-tunnel/dns");
        assert_eq!(alpn_for("web-1"), b"iroh-tunnel/web-1");
    }

    #[test]
    fn name_from_alpn_strips_prefix() {
        assert_eq!(
            name_from_alpn(b"iroh-tunnel/postgres"),
            Some("postgres")
        );
        assert_eq!(name_from_alpn(b"iroh-tunnel/dns"), Some("dns"));
    }

    #[test]
    fn name_from_alpn_rejects_other_protocols() {
        assert_eq!(name_from_alpn(b"other/protocol"), None);
        assert_eq!(name_from_alpn(b"http/1.1"), None);
        assert_eq!(name_from_alpn(b"iroh-tunnel"), None); // no trailing slash/name
    }

    #[test]
    fn name_from_alpn_rejects_invalid_utf8() {
        assert_eq!(name_from_alpn(b"iroh-tunnel/\xff invalid"), None);
        assert_eq!(name_from_alpn(b"\xff\xfe"), None);
    }

    #[test]
    fn alpn_roundtrips() {
        for name in ["postgres", "dns", "a", "web-1", "service-123"] {
            let alpn = alpn_for(name);
            assert_eq!(name_from_alpn(&alpn), Some(name));
        }
    }

    #[test]
    fn alpn_stays_under_quic_limit_for_valid_names() {
        // QUIC ALPN max is 255 bytes. With a 63-byte validated name the ALPN
        // is well under the limit (prefix is 13 bytes).
        let max_name = "a".repeat(63);
        let alpn = alpn_for(&max_name);
        assert!(alpn.len() <= 255);
    }
}
