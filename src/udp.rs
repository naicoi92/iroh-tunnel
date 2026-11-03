//! UDP datagram framing shared by Access and Serve (#88).
//!
//! Wire contract: `docs/design/udp-integration-contract.md`. Each tunnel
//! datagram is `[flow id: u32 big-endian][client payload]`, carried as one
//! QUIC DATAGRAM. The payload is opaque and may be empty. There is no
//! fragmentation: a payload larger than the connection's datagram limit
//! (maximum datagram size minus the header) is never sent.

use bytes::{BufMut, Bytes, BytesMut};

/// Bytes of the flow identifier that prefix every tunnel datagram.
pub const HEADER_LEN: usize = 4;

/// Identifier of one flow, assigned by Access within a session incarnation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FlowId(pub u32);

/// Why a tunnel datagram could not be decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// Shorter than the 4-byte header.
    Malformed,
}

/// Frame `payload` for `flow`.
pub fn encode(flow: FlowId, payload: &[u8]) -> Bytes {
    let mut out = BytesMut::with_capacity(HEADER_LEN + payload.len());
    out.put_u32(flow.0);
    out.put_slice(payload);
    out.freeze()
}

/// Split a tunnel datagram into its flow and payload.
pub fn decode(datagram: &Bytes) -> Result<(FlowId, Bytes), DecodeError> {
    if datagram.len() < HEADER_LEN {
        return Err(DecodeError::Malformed);
    }
    let flow = u32::from_be_bytes([datagram[0], datagram[1], datagram[2], datagram[3]]);
    Ok((FlowId(flow), datagram.slice(HEADER_LEN..)))
}

/// Largest client payload a connection can carry now, or `None` when the
/// connection cannot carry datagrams at all (unsupported, disabled, or a
/// maximum no larger than the header, per the contract).
pub fn payload_limit(max_datagram_size: Option<usize>) -> Option<usize> {
    max_datagram_size
        .and_then(|max| max.checked_sub(HEADER_LEN))
        .filter(|limit| *limit > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_then_decode_returns_flow_and_unchanged_payload() {
        let framed = encode(FlowId(0x0102_0304), b"payload");
        assert_eq!(&framed[..HEADER_LEN], &[1, 2, 3, 4], "big-endian header");
        let (flow, payload) = decode(&framed).unwrap();
        assert_eq!(flow, FlowId(0x0102_0304));
        assert_eq!(payload.as_ref(), b"payload");
    }

    #[test]
    fn an_empty_payload_is_valid_and_stays_empty() {
        let (flow, payload) = decode(&encode(FlowId(7), b"")).unwrap();
        assert_eq!(flow, FlowId(7));
        assert!(payload.is_empty());
    }

    #[test]
    fn a_datagram_shorter_than_the_header_is_malformed() {
        for len in 0..HEADER_LEN {
            let short = Bytes::from(vec![0u8; len]);
            assert_eq!(decode(&short), Err(DecodeError::Malformed), "len {len}");
        }
    }

    #[test]
    fn payload_limit_subtracts_the_header() {
        assert_eq!(payload_limit(Some(1162)), Some(1158));
        assert_eq!(payload_limit(Some(HEADER_LEN + 1)), Some(1));
    }

    #[test]
    fn payload_limit_is_none_when_datagrams_are_unusable() {
        assert_eq!(payload_limit(None), None);
        assert_eq!(payload_limit(Some(HEADER_LEN - 1)), None);
        assert_eq!(payload_limit(Some(HEADER_LEN)), None, "contract: 4 or less");
    }
}
