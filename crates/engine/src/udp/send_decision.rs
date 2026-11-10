use super::payload_limit::payload_limit;
use super::types::DropReason;

/// Outcome of checking one client payload against the datagram limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SendDecision {
    /// The payload fits and may be sent.
    Send,
    /// The payload must be dropped for the given reason.
    Drop(DropReason),
}

/// Decides whether a payload of `payload_len` bytes may be sent now.
///
/// An unusable limit drops as [`DropReason::LimitShrink`]; a payload over a
/// usable limit drops as [`DropReason::Oversize`].
pub(crate) fn send_decision(max_datagram_size: Option<usize>, payload_len: usize) -> SendDecision {
    match payload_limit(max_datagram_size) {
        None => SendDecision::Drop(DropReason::LimitShrink),
        Some(limit) if payload_len > limit => SendDecision::Drop(DropReason::Oversize),
        Some(_) => SendDecision::Send,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::udp::{DropReason, HEADER_LEN};

    #[test]
    fn send_decision_drops_as_limit_shrink_when_unusable() {
        assert_eq!(
            send_decision(None, 10),
            SendDecision::Drop(DropReason::LimitShrink)
        );
        assert_eq!(
            send_decision(Some(HEADER_LEN), 0),
            SendDecision::Drop(DropReason::LimitShrink)
        );
    }

    #[test]
    fn send_decision_drops_as_oversize_over_a_usable_limit() {
        assert_eq!(
            send_decision(Some(104), 101),
            SendDecision::Drop(DropReason::Oversize)
        );
    }

    #[test]
    fn send_decision_sends_within_the_limit() {
        assert_eq!(send_decision(Some(104), 100), SendDecision::Send);
        assert_eq!(send_decision(Some(104), 0), SendDecision::Send);
    }
}
