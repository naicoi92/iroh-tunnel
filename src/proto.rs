//! ALPN convention for iroh-tunnel streams.
//!
//! Each service is addressed by an ALPN byte string of the form
//! `iroh-tunnel/{name}`. The fixed prefix namespaces our streams so they
//! can't collide with other protocols multiplexed on the same QUIC
//! connection.
//!
//! Since 0.2.0 there is a second, opt-in variant per service,
//! [`multiplex_alpn_for`]: `iroh-tunnel/{name}/multi`. Serve nodes register
//! both variants for every service; the variant is the version negotiation:
//! an access node that wants to multiplex many streams over one connection
//! dials the `/multi` ALPN, and a serve node that does not know it refuses
//! the connection at the TLS handshake (fail-fast) so the access side can
//! fall back to the legacy one-connection-per-channel behavior.
//!
//! Implements T-03 (Page 05 v3 §5.1).
//
// Consumed by the serve/access handlers (T-06/T-07); flagged dead code until
// then by the binary crate's single-crate layout.
#![allow(dead_code)]

/// Fixed prefix for every service ALPN.
pub const ALPN_PREFIX: &str = "iroh-tunnel/";

/// Suffix marking the multi-stream (multiplexed) ALPN variant.
///
/// Service names are validated to `[a-z0-9-]` (no `/`), so the suffix is
/// unambiguous: `iroh-tunnel/{name}/multi` can only parse as one name.
pub const ALPN_MULTIPLEX_SUFFIX: &str = "/multi";

/// Build the ALPN byte string for a service name.
///
/// `name` is expected to already be validated (lowercase, `[a-z0-9-]`, ≤ 63
/// bytes — see `config`). No validation is done here on purpose: this is the
/// hot path for stream setup and should stay allocation-cheap.
pub fn alpn_for(name: &str) -> Vec<u8> {
    format!("{ALPN_PREFIX}{name}").into_bytes()
}

/// Build the multi-stream ALPN variant for a service name:
/// `iroh-tunnel/{name}/multi`.
///
/// Serve nodes register this alongside [`alpn_for`] so an access node can
/// negotiate multiplexing by dialing it. See the module docs.
pub fn multiplex_alpn_for(name: &str) -> Vec<u8> {
    format!("{ALPN_PREFIX}{name}{ALPN_MULTIPLEX_SUFFIX}").into_bytes()
}

/// Inverse of [`alpn_for`]/[`multiplex_alpn_for`]: strip the prefix (and the
/// multiplex suffix, if present) from an ALPN byte string and return the
/// service name. Returns `None` if the bytes are not valid UTF-8 or do not
/// start with our prefix.
pub fn name_from_alpn(alpn: &[u8]) -> Option<&str> {
    let s = std::str::from_utf8(alpn).ok()?;
    let name = s.strip_prefix(ALPN_PREFIX)?;
    // The multiplex suffix is optional: legacy ALPNs (`iroh-tunnel/{name}`)
    // must keep parsing to the bare name.
    Some(name.strip_suffix(ALPN_MULTIPLEX_SUFFIX).unwrap_or(name))
}

/// Whether `alpn` is the multi-stream variant of a service ALPN.
///
/// Reliable where a plain `ends_with` is not: a *legacy* ALPN for a service
/// literally named "multi" (`iroh-tunnel/multi`) must not classify as the
/// multiplex variant. Since service names cannot contain `/`, any slash
/// after the prefix can only come from the multiplex suffix.
pub fn is_multiplex_alpn(alpn: &[u8]) -> bool {
    match std::str::from_utf8(alpn).ok().and_then(|s| s.strip_prefix(ALPN_PREFIX)) {
        Some(name) => name.contains('/'),
        None => false,
    }
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
        assert_eq!(name_from_alpn(b"iroh-tunnel/postgres"), Some("postgres"));
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

    #[test]
    fn multiplex_alpn_for_builds_suffix_form() {
        assert_eq!(
            multiplex_alpn_for("postgres"),
            b"iroh-tunnel/postgres/multi"
        );
        assert_eq!(multiplex_alpn_for("dns"), b"iroh-tunnel/dns/multi");
    }

    #[test]
    fn name_from_alpn_strips_multiplex_suffix() {
        assert_eq!(
            name_from_alpn(b"iroh-tunnel/postgres/multi"),
            Some("postgres")
        );
        assert_eq!(name_from_alpn(b"iroh-tunnel/dns/multi"), Some("dns"));
        // A service literally named "multi" is not ambiguous with the suffix.
        assert_eq!(name_from_alpn(b"iroh-tunnel/multi"), Some("multi"));
    }

    #[test]
    fn is_multiplex_alpn_classifies_variants() {
        assert!(is_multiplex_alpn(b"iroh-tunnel/web-1/multi"));
        assert!(!is_multiplex_alpn(b"iroh-tunnel/web-1"));
        assert!(!is_multiplex_alpn(b"iroh-tunnel/multi"));
    }

    #[test]
    fn multiplex_alpn_stays_under_quic_limit() {
        // 13-byte prefix + 63-byte name + 6-byte suffix = 82 bytes, well
        // under the 255-byte QUIC ALPN limit.
        let max_name = "a".repeat(63);
        let alpn = multiplex_alpn_for(&max_name);
        assert!(alpn.len() <= 255);
    }

}