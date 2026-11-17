use bytes::Bytes;

use super::types::{DecodeError, FlowId, HEADER_LEN};

/// Split a tunnel datagram into its flow and payload.
pub fn decode(datagram: &Bytes) -> Result<(FlowId, Bytes), DecodeError> {
    let Some(header) = datagram.first_chunk::<HEADER_LEN>() else {
        return Err(DecodeError::Malformed);
    };
    let flow = u32::from_be_bytes(*header);
    Ok((FlowId(flow), datagram.slice(HEADER_LEN..)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_datagram_shorter_than_the_header_is_malformed() {
        for len in 0..HEADER_LEN {
            let short = Bytes::from(vec![0u8; len]);
            assert_eq!(decode(&short), Err(DecodeError::Malformed), "len {len}");
        }
    }
}
