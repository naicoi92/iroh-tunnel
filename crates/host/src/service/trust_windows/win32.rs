//! Win32 adapter for the trust model: open every path element without
//! following a reparse point, read its owner and DACL, and do the verified
//! I/O (read, publish, remove) through the handles that were checked.
//!
//! Every element of the chain stays open, without `FILE_SHARE_DELETE`, for
//! as long as the [`Chain`] lives. No element can be renamed or deleted in
//! that time, so the path that later I/O resolves is the one that was
//! verified.

use std::ffi::c_void;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use windows_sys::Win32::Foundation::{
    GetLastError, LocalFree, ERROR_FILE_NOT_FOUND, ERROR_INSUFFICIENT_BUFFER,
    ERROR_INVALID_PARAMETER, ERROR_NOT_SUPPORTED, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS,
    GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSidToSidW, GetSecurityInfo, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    GetAce, GetTokenInformation, IsValidSid, LookupAccountSidW, TokenUser, ACE_HEADER, ACL,
    DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
    SID_NAME_USE, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FileDispositionInfo, FileRenameInfo, FileRenameInfoEx, FlushFileBuffers,
    GetFileInformationByHandle, ReadFile, SetFileInformationByHandle, WriteFile,
    BY_HANDLE_FILE_INFORMATION, CREATE_NEW, DELETE, FILE_ADD_FILE, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_INFO_BY_HANDLE_CLASS, FILE_LIST_DIRECTORY,
    FILE_READ_ATTRIBUTES, FILE_READ_DATA, FILE_RENAME_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING, READ_CONTROL, SYNCHRONIZE,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows_sys::Win32::System::WindowsProgramming::{
    FILE_RENAME_FLAG_POSIX_SEMANTICS, FILE_RENAME_FLAG_REPLACE_IF_EXISTS,
};

use super::ace::ace_layout;
use super::model::{self, Ace, Descriptor, Refusal, Sid, TrustedSet};
use crate::error::CliError;

/// What the last element of a chain is opened for; ancestors always get
/// `READ_CONTROL | FILE_READ_ATTRIBUTES` only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LastAccess {
    /// Owner, DACL and attributes only.
    Inspect,
    /// Also `FILE_READ_DATA`, for [`read_verified`].
    ReadFile,
    /// Also `FILE_ADD_FILE | FILE_LIST_DIRECTORY`, for [`publish`].
    WriteDir,
}

/// The open handles of a path (volume root to target) and their descriptors,
/// index for index. The walk stops early at a reparse point, which then is
/// the last element: `model::evaluate` refuses it wherever it sits.
pub(crate) struct Chain {
    pub(crate) handles: Vec<OwnedHandle>,
    pub(crate) descriptors: Vec<Descriptor>,
}

impl Chain {
    /// The target's handle and descriptor. Callers evaluate first, which
    /// refuses a chain that stopped before the target.
    fn target(&self) -> Result<(&OwnedHandle, &Descriptor)> {
        match (self.handles.last(), self.descriptors.last()) {
            (Some(h), Some(d)) => Ok((h, d)),
            _ => Err(refusal(Refusal::EmptyChain)),
        }
    }
}

fn refusal(r: Refusal) -> anyhow::Error {
    CliError::from(r).into()
}

fn permission(msg: String) -> anyhow::Error {
    CliError::Permission(msg).into()
}

/// NUL-terminated UTF-16.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// `\\?\` + `component`, so Win32 path normalisation cannot change what is
/// opened (`C:\` → `\\?\C:\`).
fn verbatim(component: &str) -> Vec<u16> {
    wide(&format!("\\\\?\\{component}"))
}

/// A `LocalAlloc`ed block (security descriptor, string SID, SID) freed with
/// `LocalFree` on drop.
struct LocalBox(*mut c_void);

impl Drop for LocalBox {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the pointer came from an API documented to allocate
            // with LocalAlloc, is freed exactly once here and never used after.
            unsafe { LocalFree(self.0) };
        }
    }
}

fn open_raw(
    path: &[u16],
    access: u32,
    share: u32,
    disposition: u32,
    flags: u32,
) -> io::Result<OwnedHandle> {
    // SAFETY: `path` is a NUL-terminated UTF-16 string that outlives the
    // call; no security attributes or template handle are passed.
    let raw = unsafe {
        CreateFileW(
            path.as_ptr(),
            access,
            share,
            std::ptr::null(),
            disposition,
            flags,
            std::ptr::null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreateFileW succeeded, so `raw` is a valid handle owned by no
    // one else; OwnedHandle closes it on drop.
    Ok(unsafe { OwnedHandle::from_raw_handle(raw) })
}

/// Open one chain element without following a reparse point and without
/// letting anyone rename or delete it while the handle is held.
fn open_element(component: &str, access: u32) -> io::Result<OwnedHandle> {
    open_raw(
        &verbatim(component),
        access,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        OPEN_EXISTING,
        FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
    )
}

fn attributes(handle: &OwnedHandle) -> io::Result<u32> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: `handle` is open with FILE_READ_ATTRIBUTES; `info` is a
    // writable BY_HANDLE_FILE_INFORMATION.
    if unsafe { GetFileInformationByHandle(handle.as_raw_handle(), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(info.dwFileAttributes)
}

/// The string form of a SID the OS handed out.
///
/// # Safety
///
/// `sid` must point to a SID that stays valid for the duration of the call.
unsafe fn sid_string(sid: PSID) -> io::Result<Sid> {
    let mut text: *mut u16 = std::ptr::null_mut();
    // SAFETY: the caller guarantees `sid` is valid; `text` is a valid out
    // pointer that receives a LocalAlloc'ed string.
    if unsafe { IsValidSid(sid) } == 0 || unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let guard = LocalBox(text.cast());
    let mut len = 0;
    // SAFETY: `text` is the NUL-terminated string just returned, kept alive
    // by `guard` until the end of this function.
    let units = unsafe {
        while *text.add(len) != 0 {
            len += 1;
        }
        std::slice::from_raw_parts(text, len)
    };
    let s = String::from_utf16(units).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    drop(guard);
    Ok(Sid(s))
}

/// Decode a DACL into model ACEs. `None` when any ACE has no SID that can
/// be read (an unknown or compound layout): the element is then reported as
/// having no usable access list, so an unreadable grant is never ignored.
/// Object, callback and audit ACEs count as allow ACEs (see `ace.rs`).
///
/// # Safety
///
/// `acl` must point to a valid ACL that stays alive for the call.
unsafe fn read_dacl(acl: *const ACL) -> Option<Vec<Ace>> {
    // SAFETY: the caller guarantees `acl` is a valid ACL.
    let count = unsafe { (*acl).AceCount };
    let mut aces = Vec::with_capacity(usize::from(count));
    for index in 0..u32::from(count) {
        let mut raw: *mut c_void = std::ptr::null_mut();
        // SAFETY: `acl` is valid and `index` < AceCount; `raw` is a valid
        // out pointer that receives a pointer into the ACL.
        if unsafe { GetAce(acl, index, &mut raw) } == 0 {
            return None;
        }
        // SAFETY: GetAce returned a pointer to an ACE inside the ACL, which
        // starts with an ACE_HEADER whose AceSize bytes lie inside the ACL.
        let bytes = unsafe {
            let header = std::ptr::read_unaligned(raw.cast::<ACE_HEADER>());
            std::slice::from_raw_parts(raw.cast::<u8>(), usize::from(header.AceSize))
        };
        let layout = ace_layout(bytes)?;
        let sid_bytes = &bytes[layout.sid_offset..layout.sid_offset + layout.sid_len];
        // SAFETY: `sid_bytes` lies inside the live ACE and holds a whole SID
        // of the length its sub-authority count announces.
        let sid = unsafe { sid_string(sid_bytes.as_ptr().cast_mut().cast()) }.ok()?;
        aces.push(Ace {
            sid,
            mask: layout.mask,
            inherit_only: layout.inherit_only,
            allow: layout.allow,
        });
    }
    Some(aces)
}

/// Owner and DACL of an open handle (READ_CONTROL).
fn owner_and_dacl(handle: &OwnedHandle) -> io::Result<(Sid, Option<Vec<Ace>>)> {
    let mut owner: PSID = std::ptr::null_mut();
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: `handle` is open with READ_CONTROL; the out pointers are valid
    // and receive pointers into `sd`, which is LocalAlloc'ed.
    let status = unsafe {
        GetSecurityInfo(
            handle.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut sd,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let _sd = LocalBox(sd);
    if owner.is_null() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "no owner"));
    }
    // SAFETY: `owner` points into `sd`, alive until `_sd` drops.
    let owner = unsafe { sid_string(owner) }?;
    // A null DACL pointer is a NULL DACL (or none at all): everyone has
    // access, which the model refuses.
    let aces = if dacl.is_null() {
        None
    } else {
        // SAFETY: `dacl` points into `sd`, alive until `_sd` drops.
        unsafe { read_dacl(dacl) }
    };
    Ok((owner, aces))
}

fn components_of(path: &Path) -> Result<Vec<String>> {
    let text = path.to_str().ok_or_else(|| {
        refusal(Refusal::Malformed {
            path: path.to_path_buf(),
        })
    })?;
    model::components(text).map_err(refusal)
}

/// Open `path` element by element from the volume root, recording each
/// element's owner, DACL and attributes.
///
/// # Errors
///
/// [`CliError::Permission`] for a path that cannot be verified (not local
/// and absolute, unsafe segment, an element that cannot be opened).
pub(crate) fn open_chain(path: &Path, last: LastAccess) -> Result<Chain> {
    let components = components_of(path)?;
    let final_index = components.len() - 1;
    let mut chain = Chain {
        handles: Vec::with_capacity(components.len()),
        descriptors: Vec::with_capacity(components.len()),
    };
    for (index, component) in components.iter().enumerate() {
        let is_last = index == final_index;
        let mut access = READ_CONTROL | FILE_READ_ATTRIBUTES;
        if is_last {
            access |= match last {
                LastAccess::Inspect => 0,
                LastAccess::ReadFile => FILE_READ_DATA,
                LastAccess::WriteDir => FILE_ADD_FILE | FILE_LIST_DIRECTORY,
            };
        }
        let handle = open_element(component, access).map_err(|e| {
            permission(format!(
                "cannot open {component} to verify {} ({e}); nothing was changed",
                path.display()
            ))
        })?;
        let attrs = attributes(&handle)
            .and_then(|attrs| owner_and_dacl(&handle).map(|od| (attrs, od)))
            .map_err(|e| permission(format!("cannot read the security of {component}: {e}")));
        let (attrs, (owner, dacl)) = attrs?;
        let reparse = attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0;
        let is_dir = attrs & FILE_ATTRIBUTE_DIRECTORY != 0;
        chain.handles.push(handle);
        chain.descriptors.push(Descriptor {
            path: PathBuf::from(component),
            owner,
            dacl,
            reparse,
            is_dir,
        });
        // A file above the target: refuse here, or the shortened chain would
        // make that file look like the target.
        if !is_last && !is_dir && !reparse {
            return Err(refusal(Refusal::Malformed {
                path: PathBuf::from(component),
            }));
        }
        // Going further would traverse the reparse point; the model refuses
        // the chain collected so far, whose last element it is.
        if reparse {
            break;
        }
    }
    Ok(chain)
}

fn verified(path: &Path, last: LastAccess, trusted: &TrustedSet) -> Result<Chain> {
    let chain = open_chain(path, last)?;
    model::evaluate(&chain.descriptors, trusted).map_err(refusal)?;
    Ok(chain)
}

fn not_a(kind: &str, d: &Descriptor) -> anyhow::Error {
    permission(format!("{} is not a regular {kind}", d.path.display()))
}

/// Verify a directory (e.g. the state directory) the service writes in.
///
/// # Errors
///
/// [`CliError::Permission`] naming the unsafe element, or when the
/// directory is missing (the service never creates it).
#[cfg_attr(not(test), allow(dead_code))] // consumed by the service runtime later
pub(crate) fn verify_dir(path: &Path, trusted: &TrustedSet) -> Result<()> {
    if !path.exists() {
        return Err(permission(format!(
            "{} does not exist; create it as the installing user, the service does not \
             create directories in a user's profile",
            path.display()
        )));
    }
    let chain = verified(path, LastAccess::Inspect, trusted)?;
    let (_, d) = chain.target()?;
    if !d.is_dir {
        return Err(not_a("directory", d));
    }
    Ok(())
}

/// Verify `path` and return its text, read from the verified handle.
///
/// # Errors
///
/// [`CliError::Permission`] when the file or any element above it is unsafe.
#[cfg_attr(not(test), allow(dead_code))] // consumed by the service runtime later
pub(crate) fn read_verified(path: &Path, trusted: &TrustedSet) -> Result<String> {
    let chain = verified(path, LastAccess::ReadFile, trusted)?;
    let (handle, d) = chain.target()?;
    if d.is_dir {
        return Err(not_a("file", d));
    }
    let bytes = read_all(handle).with_context(|| format!("failed to read {}", path.display()))?;
    String::from_utf8(bytes).with_context(|| format!("{} is not UTF-8", path.display()))
}

fn read_all(handle: &OwnedHandle) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let mut read = 0u32;
        // SAFETY: `handle` is open with FILE_READ_DATA for synchronous I/O;
        // `buf` is writable for the length passed; `read` is a valid out
        // pointer; no OVERLAPPED is used.
        let ok = unsafe {
            ReadFile(
                handle.as_raw_handle(),
                buf.as_mut_ptr(),
                buf.len() as u32,
                &mut read,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        if read == 0 {
            return Ok(out);
        }
        out.extend_from_slice(&buf[..read as usize]);
    }
}

/// Verify the service binary against the system principals only.
///
/// # Errors
///
/// [`CliError::Permission`] when the binary or any directory above it can be
/// changed by a non-system principal.
#[cfg_attr(not(test), allow(dead_code))] // consumed by the SCM install later
pub(crate) fn verify_binary(path: &Path) -> Result<()> {
    let chain = verified(path, LastAccess::Inspect, &TrustedSet::for_binary())?;
    let (_, d) = chain.target()?;
    if d.is_dir {
        return Err(not_a("file", d));
    }
    Ok(())
}

/// `name` as a single plain path segment (no separator, stream, wildcard,
/// `.`/`..` or trailing dot/space), reusing the model's segment rules.
fn plain_name(name: &str) -> Result<&str> {
    match model::components(&format!("C:\\{name}")) {
        Ok(c) if c.len() == 2 => Ok(name),
        _ => Err(anyhow!("internal: {name:?} is not a plain file name")),
    }
}

/// `\\?\<dir>\<name>` for a dir taken from a verified chain.
fn child_path(dir: &Descriptor, name: &str) -> Vec<u16> {
    let dir = dir.path.to_string_lossy();
    let sep = if dir.ends_with('\\') { "" } else { "\\" };
    verbatim(&format!("{dir}{sep}{name}"))
}

/// A `FILE_RENAME_INFO` with its trailing name, aligned for the HANDLE it
/// contains. Valid by construction for `SetFileInformationByHandle`.
struct RenameInfo {
    buf: Vec<u64>,
    len: u32,
}

impl RenameInfo {
    /// `flags` is `FILE_RENAME_FLAG_*` for `FileRenameInfoEx`; `None` builds
    /// the legacy `FileRenameInfo` with `ReplaceIfExists = TRUE`.
    fn new(dir: &OwnedHandle, name: &[u16], flags: Option<u32>) -> Self {
        let name_offset = std::mem::offset_of!(FILE_RENAME_INFO, FileName);
        let name_bytes = std::mem::size_of_val(name);
        let len = (name_offset + name_bytes + 2).max(std::mem::size_of::<FILE_RENAME_INFO>());
        let mut buf = vec![0u64; len.div_ceil(8)];
        let info = buf.as_mut_ptr().cast::<FILE_RENAME_INFO>();
        // SAFETY: `buf` is zeroed, 8-byte aligned and at least `len` bytes,
        // which covers the fixed FILE_RENAME_INFO fields and `name` plus a
        // NUL at FileName; each write stays inside it.
        unsafe {
            match flags {
                Some(f) => (*info).Anonymous.Flags = f,
                None => (*info).Anonymous.ReplaceIfExists = true,
            }
            (*info).RootDirectory = dir.as_raw_handle();
            (*info).FileNameLength = name_bytes as u32;
            std::ptr::copy_nonoverlapping(
                name.as_ptr(),
                buf.as_mut_ptr().cast::<u8>().add(name_offset).cast::<u16>(),
                name.len(),
            );
        }
        Self {
            buf,
            len: len as u32,
        }
    }
}

/// `SetFileInformationByHandle` for a rename; the Win32 error on failure.
fn set_rename_info(
    file: &OwnedHandle,
    class: FILE_INFO_BY_HANDLE_CLASS,
    info: &RenameInfo,
) -> Result<(), u32> {
    // SAFETY: `file` is open with DELETE; `info` holds a valid
    // FILE_RENAME_INFO of `info.len` bytes that outlives the call.
    let ok = unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            class,
            info.buf.as_ptr().cast(),
            info.len,
        )
    };
    if ok == 0 {
        // SAFETY: GetLastError has no preconditions.
        Err(unsafe { GetLastError() })
    } else {
        Ok(())
    }
}

/// The rename step, injectable so the fallback is testable.
type Rename<'a> =
    &'a mut dyn FnMut(&OwnedHandle, FILE_INFO_BY_HANDLE_CLASS, &RenameInfo) -> Result<(), u32>;

/// Rename `temp` to `name` under `dir`, replacing whatever entry (including
/// a planted link) is there: POSIX semantics first, the legacy class when
/// the file system does not support them.
fn rename_into(
    temp: &OwnedHandle,
    dir: &OwnedHandle,
    name: &str,
    rename: Rename,
) -> io::Result<()> {
    let name: Vec<u16> = name.encode_utf16().collect();
    let flags = FILE_RENAME_FLAG_REPLACE_IF_EXISTS | FILE_RENAME_FLAG_POSIX_SEMANTICS;
    let result = match rename(
        temp,
        FileRenameInfoEx,
        &RenameInfo::new(dir, &name, Some(flags)),
    ) {
        Err(ERROR_INVALID_PARAMETER | ERROR_NOT_SUPPORTED) => {
            rename(temp, FileRenameInfo, &RenameInfo::new(dir, &name, None))
        }
        other => other,
    };
    result.map_err(|code| io::Error::from_raw_os_error(code as i32))
}

fn delete_on_close(file: &OwnedHandle) -> io::Result<()> {
    let info = FILE_DISPOSITION_INFO { DeleteFile: true };
    // SAFETY: `file` is open with DELETE; `info` is a valid
    // FILE_DISPOSITION_INFO of the size passed.
    let ok = unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileDispositionInfo,
            (&info as *const FILE_DISPOSITION_INFO).cast(),
            std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn write_all(file: &OwnedHandle, mut bytes: &[u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        let chunk = bytes.len().min(u32::MAX as usize) as u32;
        let mut written = 0u32;
        // SAFETY: `file` is open with GENERIC_WRITE for synchronous I/O;
        // `bytes` is readable for `chunk` bytes; `written` is a valid out
        // pointer; no OVERLAPPED is used.
        let ok = unsafe {
            WriteFile(
                file.as_raw_handle(),
                bytes.as_ptr(),
                chunk,
                &mut written,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        if written == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        bytes = &bytes[written as usize..];
    }
    // SAFETY: `file` is an open, writable handle.
    if unsafe { FlushFileBuffers(file.as_raw_handle()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Atomically publish `bytes` as `dir\name` through a freshly verified
/// directory chain. The temp file is created `CREATE_NEW` without following
/// a reparse point and renamed over `name` relative to the held directory
/// handle, so a link planted at either name is never written through. The
/// temp file is deleted on any error.
///
/// # Errors
///
/// [`CliError::Permission`] when the directory is no longer safe.
#[cfg_attr(not(test), allow(dead_code))] // consumed by the status writer later
pub(crate) fn publish(
    dir: &Path,
    trusted: &TrustedSet,
    name: &str,
    temp: &str,
    bytes: &[u8],
) -> Result<()> {
    publish_with(dir, trusted, name, temp, bytes, &mut set_rename_info)
}

fn publish_with(
    dir: &Path,
    trusted: &TrustedSet,
    name: &str,
    temp: &str,
    bytes: &[u8],
    rename: Rename,
) -> Result<()> {
    let (name, temp) = (plain_name(name)?, plain_name(temp)?);
    let chain = verified(dir, LastAccess::WriteDir, trusted)?;
    let (dir_handle, d) = chain.target()?;
    if !d.is_dir {
        return Err(not_a("directory", d));
    }
    let file = open_raw(
        &child_path(d, temp),
        GENERIC_WRITE | DELETE | SYNCHRONIZE,
        0,
        CREATE_NEW,
        FILE_FLAG_OPEN_REPARSE_POINT,
    )
    .with_context(|| format!("cannot create {temp} in {}", dir.display()))?;
    let result = write_all(&file, bytes)
        .with_context(|| format!("failed to write {temp} in {}", dir.display()))
        .and_then(|()| {
            rename_into(&file, dir_handle, name, rename)
                .with_context(|| format!("failed to finalize {name} in {}", dir.display()))
        });
    if result.is_err() {
        // Only our own temp file; its failure does not change the error.
        let _ = delete_on_close(&file);
    }
    result
}

/// Remove `dir\name` (a file or a link, never its target) under a verified
/// directory chain; absent is fine.
///
/// # Errors
///
/// [`CliError::Permission`] when the directory is no longer safe.
#[cfg_attr(not(test), allow(dead_code))] // consumed by the status writer later
pub(crate) fn remove_in(dir: &Path, trusted: &TrustedSet, name: &str) -> Result<()> {
    let name = plain_name(name)?;
    let chain = verified(dir, LastAccess::WriteDir, trusted)?;
    let (_, d) = chain.target()?;
    if !d.is_dir {
        return Err(not_a("directory", d));
    }
    let file = match open_raw(
        &child_path(d, name),
        DELETE,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        OPEN_EXISTING,
        FILE_FLAG_OPEN_REPARSE_POINT,
    ) {
        Ok(file) => file,
        Err(e)
            if [ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND]
                .iter()
                .any(|c| e.raw_os_error() == Some(*c as i32)) =>
        {
            return Ok(());
        }
        Err(e) => {
            return Err(e).with_context(|| format!("cannot remove {name} in {}", dir.display()))
        }
    };
    delete_on_close(&file).with_context(|| format!("failed to remove {name} in {}", dir.display()))
}

/// The SID of the process token's user.
///
/// # Errors
///
/// When the token cannot be queried.
#[cfg_attr(not(test), allow(dead_code))] // consumed by the SCM install later
pub(crate) fn token_user() -> Result<Sid> {
    let mut raw: HANDLE = std::ptr::null_mut();
    // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no
    // closing; `raw` is a valid out pointer.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) } == 0 {
        return Err(io::Error::last_os_error()).context("cannot open the process token");
    }
    // SAFETY: OpenProcessToken succeeded; `raw` is a token handle this
    // function exclusively owns.
    let token = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut needed = 0u32;
    // SAFETY: a size query: no buffer, length 0, `needed` a valid out pointer.
    unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            std::ptr::null_mut(),
            0,
            &mut needed,
        )
    };
    // u64 storage keeps TOKEN_USER (pointer-aligned) aligned.
    let mut buf = vec![0u64; (needed as usize).div_ceil(8).max(1)];
    // SAFETY: `buf` is writable for `needed` bytes and suitably aligned;
    // `needed` is a valid out pointer.
    let ok = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            buf.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error()).context("cannot read the token user");
    }
    // SAFETY: GetTokenInformation filled `buf` with a TOKEN_USER whose SID
    // points inside `buf`, which outlives this read.
    let sid = unsafe { (*buf.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    // SAFETY: `sid` points inside `buf`, alive for the call.
    unsafe { sid_string(sid) }.context("cannot convert the token user SID")
}

/// `DOMAIN\name` for `sid`, or `None` when it does not resolve.
pub(crate) fn account_of(sid: &Sid) -> Option<String> {
    let text = wide(&sid.0);
    let mut psid: PSID = std::ptr::null_mut();
    // SAFETY: `text` is NUL-terminated; `psid` receives a LocalAlloc'ed SID.
    if unsafe { ConvertStringSidToSidW(text.as_ptr(), &mut psid) } == 0 {
        return None;
    }
    let _psid = LocalBox(psid);
    let (mut name_len, mut domain_len) = (0u32, 0u32);
    let mut kind: SID_NAME_USE = 0;
    // SAFETY: a size query: null buffers with zero lengths; the length and
    // `kind` out pointers are valid; `psid` is alive.
    let ok = unsafe {
        LookupAccountSidW(
            std::ptr::null(),
            psid,
            std::ptr::null_mut(),
            &mut name_len,
            std::ptr::null_mut(),
            &mut domain_len,
            &mut kind,
        )
    };
    // SAFETY: GetLastError has no preconditions.
    if ok != 0 || unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER {
        return None;
    }
    let mut name = vec![0u16; name_len as usize];
    let mut domain = vec![0u16; domain_len as usize];
    // SAFETY: the buffers are writable for the lengths passed; `psid` alive.
    let ok = unsafe {
        LookupAccountSidW(
            std::ptr::null(),
            psid,
            name.as_mut_ptr(),
            &mut name_len,
            domain.as_mut_ptr(),
            &mut domain_len,
            &mut kind,
        )
    };
    if ok == 0 {
        return None;
    }
    let name = String::from_utf16(&name[..name_len as usize]).ok()?;
    let domain = String::from_utf16(&domain[..domain_len as usize]).ok()?;
    Some(if domain.is_empty() {
        name
    } else {
        format!("{domain}\\{name}")
    })
}

/// The owner of `path` itself (a link is not followed).
///
/// # Errors
///
/// [`CliError::Permission`] when the path cannot be opened or an element
/// above it is a reparse point or not a directory.
#[cfg_attr(not(test), allow(dead_code))] // consumed by the SCM install later
pub(crate) fn owner_of(path: &Path) -> Result<Sid> {
    let expected = components_of(path)?.len();
    let chain = open_chain(path, LastAccess::Inspect)?;
    // Shorter only when the walk stopped at a reparse ancestor.
    if chain.descriptors.len() != expected {
        return Err(refusal(Refusal::Reparse {
            path: chain.target()?.1.path.clone(),
        }));
    }
    Ok(chain.target()?.1.owner.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::fs::OpenOptionsExt;
    use std::process::Command;
    use windows_sys::Win32::Security::Authorization::SetSecurityInfo;
    use windows_sys::Win32::Security::PROTECTED_DACL_SECURITY_INFORMATION;
    use windows_sys::Win32::Storage::FileSystem::WRITE_DAC;

    fn me() -> TrustedSet {
        TrustedSet::for_user_files(&token_user().expect("token user"))
    }

    fn run(program: &str, args: &[&str]) -> bool {
        Command::new(program)
            .args(args)
            .output()
            .is_ok_and(|o| o.status.success())
    }

    fn permission_text(err: anyhow::Error) -> String {
        match err.downcast_ref::<CliError>() {
            Some(CliError::Permission(m)) => m.clone(),
            other => panic!("expected Permission, got {other:?}: {err:#}"),
        }
    }

    /// The display form the model reports for `path` (drive letter upper).
    fn reported(path: &Path) -> String {
        model::components(path.to_str().unwrap())
            .unwrap()
            .pop()
            .unwrap()
    }

    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("serve.toml");
        std::fs::write(&file, "x = 1\n").unwrap();
        (tmp, file)
    }

    #[test]
    fn profile_temp_dir_passes_for_current_user() {
        let (tmp, file) = fixture();
        verify_dir(tmp.path(), &me()).unwrap();
        let chain = open_chain(&file, LastAccess::Inspect).unwrap();
        assert_eq!(chain.handles.len(), chain.descriptors.len());
        assert_eq!(
            chain.descriptors.len(),
            model::components(file.to_str().unwrap()).unwrap().len()
        );
        assert!(chain.descriptors.iter().all(|d| !d.reparse));
    }

    #[test]
    fn everyone_modify_on_config_is_refused() {
        let (_tmp, file) = fixture();
        assert!(run(
            "icacls",
            &[file.to_str().unwrap(), "/grant", "*S-1-1-0:M"]
        ));
        let text = permission_text(read_verified(&file, &me()).unwrap_err());
        assert!(text.contains("S-1-1-0"), "{text}");
        assert!(text.contains(&reported(&file)), "{text}");
    }

    #[test]
    fn junction_in_chain_is_refused() {
        let (tmp, _file) = fixture();
        let real = tmp.path().join("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::write(real.join("serve.toml"), "x = 1\n").unwrap();
        let link = tmp.path().join("link");
        assert!(run(
            "cmd",
            &[
                "/c",
                "mklink",
                "/J",
                link.to_str().unwrap(),
                real.to_str().unwrap()
            ]
        ));
        let text = permission_text(read_verified(&link.join("serve.toml"), &me()).unwrap_err());
        assert!(text.contains("reparse point"), "{text}");
        assert!(text.contains(&reported(&link)), "{text}");
    }

    #[test]
    fn null_dacl_is_refused() {
        let (_tmp, file) = fixture();
        let handle = std::fs::OpenOptions::new()
            .access_mode(READ_CONTROL | WRITE_DAC)
            .open(&file)
            .unwrap();
        // SAFETY: `handle` is open with WRITE_DAC; a null DACL pointer with
        // DACL_SECURITY_INFORMATION sets a NULL DACL, protected so that no
        // inherited ACE replaces it.
        let status = unsafe {
            SetSecurityInfo(
                handle.as_raw_handle(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        assert_eq!(status, ERROR_SUCCESS);
        drop(handle);
        let text = permission_text(read_verified(&file, &me()).unwrap_err());
        assert!(text.contains("access list"), "{text}");
        assert!(text.contains(&reported(&file)), "{text}");
    }

    #[test]
    fn read_verified_returns_content_of_the_verified_handle() {
        let (_tmp, file) = fixture();
        assert_eq!(read_verified(&file, &me()).unwrap(), "x = 1\n");
        // A directory is not a config file.
        let dir = file.parent().unwrap();
        assert!(read_verified(dir, &me()).is_err());
        // Non-local paths never reach CreateFileW.
        assert!(read_verified(Path::new("\\\\server\\share\\x"), &me()).is_err());
    }

    #[test]
    fn publish_replaces_a_planted_symlink() {
        let (tmp, _file) = fixture();
        let dir = tmp.path();
        let target = dir.join("target");
        std::fs::write(&target, "keep").unwrap();
        let status = dir.join("serve-status.json");
        if !run(
            "cmd",
            &[
                "/c",
                "mklink",
                status.to_str().unwrap(),
                target.to_str().unwrap(),
            ],
        ) {
            println!("skipped: cannot create a symlink (needs privilege or developer mode)");
            return;
        }
        assert!(std::fs::symlink_metadata(&status)
            .unwrap()
            .file_type()
            .is_symlink());
        publish(
            dir,
            &me(),
            "serve-status.json",
            "serve-status.json.tmp.1",
            b"new",
        )
        .unwrap();
        let meta = std::fs::symlink_metadata(&status).unwrap();
        assert!(meta.file_type().is_file() && !meta.file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(&status).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep");
        assert!(!dir.join("serve-status.json.tmp.1").exists());
    }

    #[test]
    fn publish_rename_falls_back_without_posix_semantics() {
        let (tmp, _file) = fixture();
        let dir = tmp.path();
        std::fs::write(dir.join("s.json"), "one").unwrap();
        let mut classes = Vec::new();
        let mut rename = |f: &OwnedHandle,
                          class: FILE_INFO_BY_HANDLE_CLASS,
                          info: &RenameInfo|
         -> Result<(), u32> {
            classes.push(class);
            if class == FileRenameInfoEx {
                Err(ERROR_INVALID_PARAMETER)
            } else {
                set_rename_info(f, class, info)
            }
        };
        publish_with(dir, &me(), "s.json", "s.json.tmp", b"two", &mut rename).unwrap();
        assert_eq!(classes, vec![FileRenameInfoEx, FileRenameInfo]);
        assert_eq!(std::fs::read_to_string(dir.join("s.json")).unwrap(), "two");

        // A rename failure that is not a capability error removes the temp.
        let mut failing = |_: &OwnedHandle,
                           _: FILE_INFO_BY_HANDLE_CLASS,
                           _: &RenameInfo|
         -> Result<(), u32> { Err(5) };
        assert!(publish_with(dir, &me(), "s.json", "s.json.tmp", b"x", &mut failing).is_err());
        assert!(!dir.join("s.json.tmp").exists());
        assert_eq!(std::fs::read_to_string(dir.join("s.json")).unwrap(), "two");
    }

    #[test]
    fn publish_and_remove_refuse_names_that_are_not_plain() {
        let (tmp, _file) = fixture();
        for bad in ["", ".", "..", "a\\b", "a/b", "a:s", "a."] {
            assert!(
                publish(tmp.path(), &me(), bad, "t", b"x").is_err(),
                "{bad:?}"
            );
            assert!(remove_in(tmp.path(), &me(), bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn remove_in_deletes_the_name_and_accepts_absent() {
        let (tmp, file) = fixture();
        let dir = tmp.path();
        remove_in(dir, &me(), "serve.toml").unwrap();
        assert!(!file.exists());
        remove_in(dir, &me(), "serve.toml").unwrap();
    }

    #[test]
    fn verify_binary_accepts_a_system_binary_and_refuses_a_directory() {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        let system32 = Path::new(&root).join("System32");
        verify_binary(&system32.join("cmd.exe")).unwrap();
        assert!(verify_binary(&system32).is_err());
    }

    #[test]
    fn owner_of_temp_file_is_current_user() {
        let (_tmp, file) = fixture();
        let owner = owner_of(&file).unwrap();
        // An elevated administrator's token makes Administrators the default
        // owner of new files; any other token makes the user the owner.
        let user = token_user().unwrap();
        assert!(
            owner == user || owner.0 == model::ADMINISTRATORS,
            "{owner:?} vs {user:?}"
        );
        assert!(account_of(&user).is_some_and(|name| name.contains('\\')));
        assert_eq!(account_of(&Sid("S-1-5-21-9-9-9-1".into())), None);
        assert_eq!(
            account_of(&Sid(model::SYSTEM.into())).map(|n| n.ends_with("\\SYSTEM")),
            Some(true)
        );
    }
}
