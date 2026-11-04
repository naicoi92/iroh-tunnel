/// First 8 chars of a node id + `…`, for human-readable tables and log
/// lines.
///
/// One definition shared by every consumer (access's established/path-change
/// log lines, the `<role> status` tables) so a short id means the same
/// thing everywhere. The full id stays in the machine-readable fields
/// (`peer=`, status files) so both hosts can be correlated by grepping the
/// same string; short forms only need to be recognizable at a glance.
pub fn short_peer_id(peer: &str) -> String {
    let head: String = peer.chars().take(8).collect();
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_peer_id_keeps_eight_chars_and_an_ellipsis() {
        assert_eq!(short_peer_id("0123456789abcdef"), "01234567…");
        assert_eq!(short_peer_id("abc"), "abc…");
    }
}
