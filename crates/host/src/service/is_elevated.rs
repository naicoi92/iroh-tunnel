//! Whether the process runs with root/Administrator privileges.

/// Whether the process runs with root privileges. Always `false` on targets
/// without a service backend (where every operation is refused anyway).
#[cfg(unix)]
pub fn is_elevated() -> bool {
    // SAFETY: geteuid(2) has no preconditions and no failure mode.
    unsafe { libc::geteuid() == 0 }
}

/// See the Unix variant; there is no root account to compare against here.
#[cfg(not(unix))]
pub fn is_elevated() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elevation_matches_the_effective_user() {
        #[cfg(unix)]
        assert_eq!(is_elevated(), unsafe { libc::geteuid() } == 0);
        #[cfg(not(unix))]
        assert!(!is_elevated());
    }
}
