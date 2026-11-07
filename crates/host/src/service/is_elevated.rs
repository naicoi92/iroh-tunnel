//! Whether the process runs with root/Administrator privileges.

/// Whether the process runs with root privileges. Always `false` on targets
/// without a service backend (where every operation is refused anyway).
#[cfg(unix)]
pub fn is_elevated() -> bool {
    // SAFETY: geteuid(2) has no preconditions and no failure mode.
    unsafe { libc::geteuid() == 0 }
}

/// See the Unix variant; there is no root account to compare against here.
#[cfg(all(not(unix), not(windows)))]
pub fn is_elevated() -> bool {
    false
}

/// Whether the process token is elevated (`TokenElevation`). Any failure
/// reads as "not elevated".
#[cfg(windows)]
pub fn is_elevated() -> bool {
    use std::mem::size_of;
    use std::os::windows::io::{FromRawHandle, OwnedHandle};

    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut raw: HANDLE = std::ptr::null_mut();
    // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no closing;
    // `raw` is a valid out-pointer for the duration of the call.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) } == 0 {
        return false;
    }
    // SAFETY: OpenProcessToken succeeded, so `raw` is a valid handle that this
    // function exclusively owns; OwnedHandle closes it on drop.
    let token = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut returned = 0u32;
    // SAFETY: `token` is a live token handle with TOKEN_QUERY access;
    // `elevation` is a writable TOKEN_ELEVATION and the length passed equals
    // its size; `returned` is a valid out-pointer.
    let ok = unsafe {
        use std::os::windows::io::AsRawHandle;
        GetTokenInformation(
            token.as_raw_handle(),
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    };
    ok != 0 && elevation.TokenIsElevated != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elevation_matches_the_effective_user() {
        #[cfg(unix)]
        assert_eq!(is_elevated(), unsafe { libc::geteuid() } == 0);
        #[cfg(all(not(unix), not(windows)))]
        assert!(!is_elevated());
    }

    /// The Windows CI runner is elevated; a developer shell may not be.
    #[cfg(windows)]
    #[test]
    fn elevated_on_the_ci_runner() {
        if std::env::var("IROH_TUNNEL_E2E_SERVICE").as_deref() != Ok("1") {
            return;
        }
        assert!(is_elevated());
    }
}
