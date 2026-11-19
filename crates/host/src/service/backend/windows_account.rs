//! Win32 probes for [`super::super::always_elevated`]: the account's SID and
//! group membership, and the UAC policy.

use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Security::{
    CreateWellKnownSid, EqualSid, GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation,
    TokenGroups, TokenUser, WinBuiltinAdministratorsSid, SECURITY_MAX_SID_SIZE, TOKEN_GROUPS,
    TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// Relative id of the built-in Administrator account.
const BUILTIN_ADMIN_RID: u32 = 500;

fn open_token() -> Option<OwnedHandle> {
    let mut raw: HANDLE = std::ptr::null_mut();
    // SAFETY: the pseudo-handle needs no closing; `raw` is a valid out-pointer.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) } == 0 {
        return None;
    }
    // SAFETY: success means `raw` is a valid handle owned only here.
    Some(unsafe { OwnedHandle::from_raw_handle(raw) })
}

/// The token information `class` as an 8-byte-aligned buffer.
fn token_info(token: &OwnedHandle, class: i32) -> Option<Vec<u64>> {
    let mut needed = 0u32;
    // SAFETY: a null buffer of length 0 only asks for the required size.
    unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            class,
            std::ptr::null_mut(),
            0,
            &mut needed,
        )
    };
    if needed == 0 {
        return None;
    }
    let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
    // SAFETY: `buf` holds at least `needed` writable bytes.
    let ok = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            class,
            buf.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    };
    (ok != 0).then_some(buf)
}

/// Whether the account is the built-in Administrator (user SID ends in RID 500).
pub(crate) fn is_builtin_admin() -> bool {
    let Some(token) = open_token() else {
        return false;
    };
    let Some(buf) = token_info(&token, TokenUser) else {
        return false;
    };
    // SAFETY: the buffer holds a TOKEN_USER written by the OS, whose SID
    // points into the same buffer.
    unsafe {
        let sid = (*buf.as_ptr().cast::<TOKEN_USER>()).User.Sid;
        let count = *GetSidSubAuthorityCount(sid);
        count > 0 && *GetSidSubAuthority(sid, u32::from(count) - 1) == BUILTIN_ADMIN_RID
    }
}

/// Whether the account belongs to `BUILTIN\Administrators`. A filtered or
/// restricted token lists the group as deny-only; it is still listed, so the
/// account's membership is read from the group list, not from access checks.
pub(crate) fn is_admin_member() -> bool {
    let mut admins = [0u8; SECURITY_MAX_SID_SIZE as usize];
    let mut size = SECURITY_MAX_SID_SIZE;
    // SAFETY: `admins` is writable for `size` bytes; a null domain is valid
    // for a well-known local SID.
    let made = unsafe {
        CreateWellKnownSid(
            WinBuiltinAdministratorsSid,
            std::ptr::null_mut(),
            admins.as_mut_ptr().cast(),
            &mut size,
        )
    };
    let Some(token) = open_token() else {
        return false;
    };
    let Some(buf) = token_info(&token, TokenGroups) else {
        return false;
    };
    if made == 0 {
        return false;
    }
    // SAFETY: the buffer holds a TOKEN_GROUPS written by the OS with
    // `GroupCount` entries whose SIDs point into the same buffer.
    unsafe {
        let groups = buf.as_ptr().cast::<TOKEN_GROUPS>();
        let list =
            std::slice::from_raw_parts((*groups).Groups.as_ptr(), (*groups).GroupCount as usize);
        list.iter()
            .any(|g| EqualSid(g.Sid, admins.as_mut_ptr().cast()) != 0)
    }
}

/// `EnableLUA` (UAC on/off); `None` when the value is absent or unreadable.
pub(crate) fn enable_lua() -> Option<bool> {
    let subkey: Vec<u16> = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Policies\\System\0"
        .encode_utf16()
        .collect();
    let value: Vec<u16> = "EnableLUA\0".encode_utf16().collect();
    let mut data = 0u32;
    let mut len = 4u32;
    // SAFETY: both names are NUL-terminated; `data` is a writable DWORD of
    // `len` bytes.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            std::ptr::addr_of_mut!(data).cast(),
            &mut len,
        )
    };
    (status == 0).then_some(data != 0)
}
