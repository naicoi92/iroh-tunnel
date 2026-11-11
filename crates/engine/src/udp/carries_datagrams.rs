//! Datagram usability predicate and the close that reports it.
//!
//! Invariant: a session incarnation exists only on a connection that can
//! carry datagrams. See `docs/udp-integration-contract.md`.

use super::payload_limit::payload_limit;

/// Application close code either role sends when its peer cannot carry datagrams.
pub(crate) const DATAGRAMS_UNSUPPORTED_CODE: u32 = 0x1d47;

/// Application close reason paired with [`DATAGRAMS_UNSUPPORTED_CODE`].
pub(crate) const DATAGRAMS_UNSUPPORTED_REASON: &[u8] = b"udp datagrams unsupported";

/// True when the maximum QUIC datagram size leaves room for a payload.
///
/// Unusable means absent or no larger than the tunnel header; this defers
/// to [`payload_limit`] so there is a single implementation.
pub(crate) fn carries_datagrams(max_datagram_size: Option<usize>) -> bool {
    payload_limit(max_datagram_size).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::udp::HEADER_LEN;

    #[test]
    fn carries_datagrams_at_the_boundary() {
        assert!(!carries_datagrams(None));
        assert!(!carries_datagrams(Some(HEADER_LEN)));
        assert!(carries_datagrams(Some(HEADER_LEN + 1)));
    }
}
