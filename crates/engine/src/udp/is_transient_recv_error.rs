/// Whether a receive error on a UDP socket only reports a lost datagram.
///
/// A connected socket reports the target's ICMP answer (port unreachable) on a
/// later receive. Windows reports it on unconnected sockets too, as
/// `WSAECONNRESET` (os error 10054), after a send to a port nobody listens on.
/// Neither means the socket is dead.
pub fn is_transient_recv_error(error: &std::io::Error) -> bool {
    use std::io::ErrorKind::{
        ConnectionAborted, ConnectionRefused, ConnectionReset, Interrupted, WouldBlock,
    };
    matches!(
        error.kind(),
        ConnectionRefused | ConnectionReset | ConnectionAborted | Interrupted | WouldBlock
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icmp_style_receive_errors_are_transient() {
        use std::io::{Error, ErrorKind};
        for kind in [
            ErrorKind::ConnectionRefused,
            ErrorKind::ConnectionReset,
            ErrorKind::ConnectionAborted,
        ] {
            assert!(is_transient_recv_error(&Error::from(kind)), "{kind:?}");
        }
        assert!(!is_transient_recv_error(&Error::from(
            ErrorKind::PermissionDenied
        )));
        assert!(!is_transient_recv_error(&Error::from(
            ErrorKind::InvalidInput
        )));
    }
}
