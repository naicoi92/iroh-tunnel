use bytes::{BufMut, Bytes, BytesMut};

use super::types::{FlowId, HEADER_LEN};

/// Frame `payload` for `flow`.
pub fn encode(flow: FlowId, payload: &[u8]) -> Bytes {
    let mut out = BytesMut::with_capacity(HEADER_LEN + payload.len());
    out.put_u32(flow.0);
    out.put_slice(payload);
    out.freeze()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::udp::decode;

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
}
