//! Recognition of a system instance started by an earlier version.
//!
//! Before the claim/detection lock split, a system process held
//! `<role>.lock` exclusively and had no `<role>.claim.lock`. Such a process is
//! invisible to the claim lock, so this module recognises it from facts only
//! an admin-owned process can produce: `<role>.lock` is held exclusively AND
//! `<role>.owner.json` in the same directory is owned by an admin principal
//! (root / SYSTEM / Administrators) and names a live pid. A non-admin account
//! can hold the world-readable lock but cannot forge an admin-owned record, so
//! it can never block a system start.

use std::path::Path;

use anyhow::Result;

use super::{held_exclusively, lock_name, owner_name, OwnerRecord};
use crate::role::Role;

/// The most an owner record may hold; anything larger is not ours.
#[cfg(unix)]
const MAX_RECORD_BYTES: u64 = 1 << 20;

/// The record of an earlier-version system instance of `role` running in
/// `dir`, if there is one.
///
/// # Errors
///
/// A malformed test owner, or an I/O error testing the lock.
pub(super) fn earlier_version_holder(dir: &Path, role: Role) -> Result<Option<OwnerRecord>> {
    #[cfg(any(unix, windows))]
    {
        let trust = crate::service::admin_trust(&crate::service::process_env)?;
        earlier_version_holder_with(dir, role, &trust)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (dir, role);
        Ok(None)
    }
}

/// [`earlier_version_holder`] with the admin trust policy injected.
#[cfg(any(unix, windows))]
pub(super) fn earlier_version_holder_with(
    dir: &Path,
    role: Role,
    trust: &crate::service::AdminTrust,
) -> Result<Option<OwnerRecord>> {
    if !held_exclusively(&dir.join(lock_name(role)))? {
        return Ok(None);
    }
    let Some(body) = read_admin_owned(&dir.join(owner_name(role)), trust) else {
        return Ok(None);
    };
    Ok(serde_json::from_str::<OwnerRecord>(&body)
        .ok()
        .filter(|record| record.role == role.name() && pid_alive(record.pid)))
}

/// Read `path` without following a link, only when the opened handle is a
/// regular file owned by an admin principal.
#[cfg(unix)]
fn read_admin_owned(path: &Path, trust: &crate::service::AdminTrust) -> Option<String> {
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || !trust.trusts(meta.uid()) {
        return None;
    }
    let mut body = String::new();
    file.take(MAX_RECORD_BYTES).read_to_string(&mut body).ok()?;
    Some(body)
}

/// Windows: the owner SID is read from the opened handle.
#[cfg(windows)]
fn read_admin_owned(path: &Path, trust: &crate::service::AdminTrust) -> Option<String> {
    crate::service::trust_windows::read_owned_by(path, trust)
        .inspect_err(|e| tracing::debug!("not reading {}: {e:#}", path.display()))
        .ok()
        .flatten()
}

/// Whether process `pid` exists.
#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    if pid == 0 {
        return false;
    }
    // SAFETY: signal 0 only checks that the process exists.
    unsafe {
        libc::kill(pid, 0) == 0
            || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

/// Whether process `pid` exists and has not exited.
#[cfg(windows)]
fn pid_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    const STILL_ACTIVE: u32 = 259;
    if pid == 0 {
        return false;
    }
    // SAFETY: plain query; the handle is closed before returning.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return std::io::Error::last_os_error().raw_os_error()
                == Some(ERROR_ACCESS_DENIED.cast_signed());
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(handle, &mut code) != 0;
        CloseHandle(handle);
        ok && code == STILL_ACTIVE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_process_is_alive_and_pid_zero_is_not() {
        assert!(pid_alive(std::process::id()));
        assert!(!pid_alive(0));
    }
}
