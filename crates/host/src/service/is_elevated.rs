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

/// Whether the process effectively holds administrator rights: the token is
/// elevated (`TokenElevation`) AND the effective token is a member of
/// `BUILTIN\Administrators`. `TokenElevation` alone is not enough: a token
/// restricted from an elevated one (`CreateRestrictedToken` with
/// Administrators set deny-only) can still report it, while
/// `CheckTokenMembership` respects deny-only SIDs. Any failure reads as "not
/// elevated".
#[cfg(windows)]
pub fn is_elevated() -> bool {
    token_is_elevated() && is_administrators_member()
}

/// Whether the effective token has `BUILTIN\Administrators` enabled (not
/// deny-only). A NULL token handle checks the calling thread's effective
/// token.
#[cfg(windows)]
fn is_administrators_member() -> bool {
    use windows_sys::Win32::Security::{
        CheckTokenMembership, CreateWellKnownSid, WinBuiltinAdministratorsSid,
        SECURITY_MAX_SID_SIZE,
    };

    let mut sid = [0u8; SECURITY_MAX_SID_SIZE as usize];
    let mut size = SECURITY_MAX_SID_SIZE;
    // SAFETY: `sid` is a writable buffer of `size` bytes; a NULL domain SID is
    // valid for a well-known local SID.
    let created = unsafe {
        CreateWellKnownSid(
            WinBuiltinAdministratorsSid,
            std::ptr::null_mut(),
            sid.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if created == 0 {
        return false;
    }
    let mut is_member = 0i32;
    // SAFETY: `sid` holds a valid SID written above; a NULL token means the
    // effective token; `is_member` is a valid out-pointer.
    let ok = unsafe {
        CheckTokenMembership(
            std::ptr::null_mut(),
            sid.as_mut_ptr().cast(),
            &mut is_member,
        )
    };
    ok != 0 && is_member != 0
}

/// Whether the process token reports `TokenElevation`.
#[cfg(windows)]
fn token_is_elevated() -> bool {
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
