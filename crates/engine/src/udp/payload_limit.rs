use super::types::HEADER_LEN;

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
