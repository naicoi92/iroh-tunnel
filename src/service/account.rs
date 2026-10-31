//! OS account lookup for choosing whose files a system service uses (SM-02).
//!
//! `SUDO_USER` is only an *input* to the choice (SM-04); the account is always
//! resolved through the user database, never trusted as a path.

use std::path::PathBuf;

/// A user account from the user database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub name: String,
    pub uid: u32,
    pub home: PathBuf,
}

/// Look up an account by name. `None` when it does not exist.
#[cfg(unix)]
pub fn by_name(name: &str) -> Option<Account> {
    let name = std::ffi::CString::new(name).ok()?;
    lookup(|pwd, buf, len, out| {
        // SAFETY: `name` is NUL-terminated; `pwd`, `buf` and `out` come from
        // `lookup` and are valid for the stated sizes for the whole call.
        unsafe { libc::getpwnam_r(name.as_ptr(), pwd, buf, len, out) }
    })
}

/// Look up an account by UID. `None` when it does not exist.
#[cfg(unix)]
pub fn by_uid(uid: u32) -> Option<Account> {
    lookup(|pwd, buf, len, out| {
        // SAFETY: `pwd`, `buf` and `out` come from `lookup` and are valid for
        // the stated sizes for the whole call.
        unsafe { libc::getpwuid_r(uid, pwd, buf, len, out) }
    })
}

#[cfg(unix)]
fn lookup(
    call: impl Fn(
        *mut libc::passwd,
        *mut libc::c_char,
        libc::size_t,
        *mut *mut libc::passwd,
    ) -> libc::c_int,
) -> Option<Account> {
    use std::ffi::CStr;
    use std::os::unix::ffi::OsStrExt;

    let mut buf = vec![0 as libc::c_char; 1024];
    loop {
        // SAFETY: an all-zero `passwd` is a valid value for the C struct; the
        // call overwrites it.
        let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
        let mut found: *mut libc::passwd = std::ptr::null_mut();
        let rc = call(&mut pwd, buf.as_mut_ptr(), buf.len(), &mut found);
        if rc == libc::ERANGE && buf.len() < 1 << 20 {
            buf.resize(buf.len() * 2, 0);
            continue;
        }
        if rc != 0 || found.is_null() {
            return None;
        }
        // SAFETY: on success the string fields point into `buf`, which is
        // alive and unmodified until this function returns.
        let (name, home) = unsafe {
            (
                CStr::from_ptr(pwd.pw_name).to_string_lossy().into_owned(),
                CStr::from_ptr(pwd.pw_dir),
            )
        };
        return Some(Account {
            name,
            uid: pwd.pw_uid,
            home: PathBuf::from(std::ffi::OsStr::from_bytes(home.to_bytes())),
        });
    }
}

#[cfg(not(unix))]
pub fn by_name(_name: &str) -> Option<Account> {
    None
}

#[cfg(not(unix))]
pub fn by_uid(_uid: u32) -> Option<Account> {
    None
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn root_resolves_by_name_and_uid() {
        let by_name = by_name("root").expect("root account");
        assert_eq!(by_name.uid, 0);
        assert_eq!(by_uid(0).map(|a| a.name), Some(by_name.name));
    }

    #[test]
    fn unknown_account_is_none() {
        assert_eq!(by_name("no-such-user-iroh-tunnel"), None);
        assert_eq!(by_name("bad\0name"), None);
    }
}
