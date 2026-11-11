use iroh::endpoint::{ConnectionError, VarInt};

use super::carries_datagrams::DATAGRAMS_UNSUPPORTED_CODE;
#[cfg(test)]
use super::carries_datagrams::DATAGRAMS_UNSUPPORTED_REASON;

/// True only when the peer closed with the dedicated
/// "datagrams unsupported" application code.
///
/// Local close, reset, timeout and other application codes are not it.
pub(crate) fn closed_for_unsupported_datagrams(error: &ConnectionError) -> bool {
    matches!(
        error,
        ConnectionError::ApplicationClosed(close)
            if close.error_code == VarInt::from_u32(DATAGRAMS_UNSUPPORTED_CODE)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_close(code: u32) -> iroh::endpoint::ConnectionError {
        iroh::endpoint::ConnectionError::ApplicationClosed(iroh::endpoint::ApplicationClose {
            error_code: iroh::endpoint::VarInt::from_u32(code),
            reason: bytes::Bytes::from_static(DATAGRAMS_UNSUPPORTED_REASON),
        })
    }

    #[test]
    fn close_predicate_matches_the_dedicated_code() {
        assert!(closed_for_unsupported_datagrams(&app_close(
            DATAGRAMS_UNSUPPORTED_CODE
        )));
    }

    #[test]
    fn close_predicate_ignores_other_application_codes() {
        assert!(!closed_for_unsupported_datagrams(&app_close(0)));
    }

    #[test]
    fn close_predicate_ignores_local_close_reset_and_timeout() {
        use iroh::endpoint::ConnectionError::*;
        for e in [LocallyClosed, Reset, TimedOut] {
            assert!(!closed_for_unsupported_datagrams(&e));
        }
    }
}
