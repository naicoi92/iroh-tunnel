//! Render a `host:port` address for display.

/// Format `host:port`, bracketing IPv6 literals (`[::1]:3000`).
///
/// Host copy of the engine's private `format_local_addr`.
// Consumed by `set_service` (next task); remove this attribute there.
#[cfg_attr(not(test), expect(dead_code))]
pub(crate) fn display_addr(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_addr_brackets_ipv6() {
        assert_eq!(display_addr("::1", 3000), "[::1]:3000");
        assert_eq!(display_addr("db.internal", 5432), "db.internal:5432");
        assert_eq!(display_addr("127.0.0.1", 80), "127.0.0.1:80");
    }
}
