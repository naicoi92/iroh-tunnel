//! Run a command in a NON-elevated child from an elevated test process.
//!
//! The child gets a restricted copy of the current token: all privileges
//! removed, the Administrators group made deny-only, integrity lowered to
//! Medium. That is what an unelevated admin session looks like, so the
//! non-elevated paths can be driven end to end on the elevated CI runner.

use anyhow::{bail, ensure, Result};
use std::ffi::{OsStr, OsString};
use std::io::Read as _;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::process::{Command, ExitStatus, Output};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::SetHandleInformation;
use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_PRIVILEGE_NOT_HELD, HANDLE, HANDLE_FLAG_INHERIT, WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::{
    CreateRestrictedToken, CreateWellKnownSid, GetLengthSid, SetTokenInformation,
    TokenIntegrityLevel, WinBuiltinAdministratorsSid, WinMediumLabelSid, DISABLE_MAX_PRIVILEGE,
    SECURITY_ATTRIBUTES, SECURITY_MAX_SID_SIZE, SID_AND_ATTRIBUTES, TOKEN_ADJUST_DEFAULT,
    TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CreateProcessAsUserW, CreateProcessWithTokenW, GetCurrentProcess, GetExitCodeProcess,
    OpenProcessToken, WaitForSingleObject, CREATE_UNICODE_ENVIRONMENT, INFINITE,
    PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOW,
};

/// `SE_GROUP_INTEGRITY` (winnt.h); windows-sys only exposes it behind a feature we do not need.
const SE_GROUP_INTEGRITY: u32 = 0x20;

/// Which API launched the child (reported by the self-test).
pub static LAUNCH_API: std::sync::Mutex<&'static str> = std::sync::Mutex::new("none");

/// A SID in a stack buffer.
struct Sid([u8; SECURITY_MAX_SID_SIZE as usize]);

impl Sid {
    fn well_known(kind: i32) -> Result<Self> {
        let mut buf = [0u8; SECURITY_MAX_SID_SIZE as usize];
        let mut len = buf.len() as u32;
        // SAFETY: `buf` is SECURITY_MAX_SID_SIZE bytes and `len` says so.
        let ok = unsafe { CreateWellKnownSid(kind, null_mut(), buf.as_mut_ptr().cast(), &mut len) };
        ensure!(
            ok != 0,
            "CreateWellKnownSid({kind}) failed: {}",
            last_error()
        );
        Ok(Self(buf))
    }
    fn as_ptr(&mut self) -> *mut core::ffi::c_void {
        self.0.as_mut_ptr().cast()
    }
}

fn last_error() -> u32 {
    // SAFETY: plain thread-local error read.
    unsafe { GetLastError() }
}

fn owned(handle: HANDLE) -> OwnedHandle {
    // SAFETY: `handle` is a valid handle this function now owns exclusively.
    unsafe { OwnedHandle::from_raw_handle(handle as _) }
}

/// `(read, write)` of an anonymous pipe; the child inherits only `child_end`.
fn pipe(child_reads: bool) -> Result<(OwnedHandle, OwnedHandle)> {
    let sa = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: null_mut(),
        bInheritHandle: 1,
    };
    let (mut read, mut write): (HANDLE, HANDLE) = (null_mut(), null_mut());
    // SAFETY: out-pointers are valid; `sa` outlives the call.
    ensure!(
        unsafe { CreatePipe(&mut read, &mut write, &sa, 0) } != 0,
        "CreatePipe failed: {}",
        last_error()
    );
    let (read, write) = (owned(read), owned(write));
    let ours = if child_reads { &write } else { &read };
    // SAFETY: `ours` is a valid handle; clear its inherit flag.
    ensure!(
        unsafe { SetHandleInformation(ours.as_raw_handle() as _, HANDLE_FLAG_INHERIT, 0) } != 0,
        "SetHandleInformation failed: {}",
        last_error()
    );
    Ok((read, write))
}

/// The current token, restricted and lowered to Medium integrity.
fn restricted_token() -> Result<OwnedHandle> {
    let mut token: HANDLE = null_mut();
    // SAFETY: pseudo-handle of this process; `token` is a valid out-pointer.
    let ok = unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_ASSIGN_PRIMARY | TOKEN_ADJUST_DEFAULT,
            &mut token,
        )
    };
    ensure!(ok != 0, "OpenProcessToken failed: {}", last_error());
    let token = owned(token);

    let mut admins = Sid::well_known(WinBuiltinAdministratorsSid)?;
    let deny = SID_AND_ATTRIBUTES {
        Sid: admins.as_ptr(),
        Attributes: 0,
    };
    let mut restricted: HANDLE = null_mut();
    // SAFETY: `deny` points at a live SID; the optional lists are empty.
    let ok = unsafe {
        CreateRestrictedToken(
            token.as_raw_handle() as _,
            DISABLE_MAX_PRIVILEGE,
            1,
            &deny,
            0,
            null(),
            0,
            null(),
            &mut restricted,
        )
    };
    ensure!(ok != 0, "CreateRestrictedToken failed: {}", last_error());
    let restricted = owned(restricted);

    let mut medium = Sid::well_known(WinMediumLabelSid)?;
    let label = TOKEN_MANDATORY_LABEL {
        Label: SID_AND_ATTRIBUTES {
            Sid: medium.as_ptr(),
            Attributes: SE_GROUP_INTEGRITY,
        },
    };
    // SAFETY: the SID is valid, so its length is readable.
    let sid_len = unsafe { GetLengthSid(medium.as_ptr()) };
    // SAFETY: `label` is a TOKEN_MANDATORY_LABEL whose SID is live; the size
    // is the struct plus the SID, as the API documents.
    let ok = unsafe {
        SetTokenInformation(
            restricted.as_raw_handle() as _,
            TokenIntegrityLevel,
            (&label as *const TOKEN_MANDATORY_LABEL).cast(),
            size_of::<TOKEN_MANDATORY_LABEL>() as u32 + sid_len,
        )
    };
    ensure!(
        ok != 0,
        "SetTokenInformation(integrity) failed: {}",
        last_error()
    );
    Ok(restricted)
}

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().collect()
}

/// Append one argument using the `CommandLineToArgvW` rules: quote when empty
/// or containing space/tab/quote; backslashes are literal except before a
/// quote (doubled, plus one to escape the quote) or the closing quote (doubled).
fn push_arg(line: &mut Vec<u16>, arg: &OsStr) {
    let chars = wide(arg);
    let needs_quotes = chars.is_empty()
        || chars
            .iter()
            .any(|&c| c == b' ' as u16 || c == b'\t' as u16 || c == b'"' as u16);
    if !needs_quotes {
        line.extend(chars);
        return;
    }
    line.push(b'"' as u16);
    let mut backslashes = 0usize;
    for c in chars {
        if c == b'\\' as u16 {
            backslashes += 1;
            continue;
        }
        if c == b'"' as u16 {
            line.extend(std::iter::repeat_n(b'\\' as u16, backslashes * 2 + 1));
        } else {
            line.extend(std::iter::repeat_n(b'\\' as u16, backslashes));
        }
        backslashes = 0;
        line.push(c);
    }
    line.extend(std::iter::repeat_n(b'\\' as u16, backslashes * 2));
    line.push(b'"' as u16);
}

fn command_line(cmd: &Command) -> Vec<u16> {
    let mut line = Vec::new();
    push_arg(&mut line, cmd.get_program());
    for arg in cmd.get_args() {
        line.push(b' ' as u16);
        push_arg(&mut line, arg);
    }
    line.push(0);
    line
}

/// The process environment with the command's changes applied, as a sorted
/// (case-insensitive, as Windows requires) UTF-16 `K=V\0...\0\0` block.
fn environment_block(cmd: &Command) -> Vec<u16> {
    let mut vars: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    for (key, value) in cmd.get_envs() {
        let same = |k: &OsString| {
            k.to_string_lossy()
                .eq_ignore_ascii_case(&key.to_string_lossy())
        };
        vars.retain(|(k, _)| !same(k));
        if let Some(value) = value {
            vars.push((key.to_owned(), value.to_owned()));
        }
    }
    vars.sort_by_key(|(k, _)| k.to_string_lossy().to_uppercase());
    let mut block = Vec::new();
    for (key, value) in vars {
        block.extend(wide(&key));
        block.push(b'=' as u16);
        block.extend(wide(&value));
        block.push(0);
    }
    block.push(0);
    if block.len() == 1 {
        block.push(0);
    }
    block
}

fn drain(handle: OwnedHandle) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = std::fs::File::from(handle).read_to_end(&mut bytes);
        bytes
    })
}

/// Run `cmd` (program, args, env changes, working directory) under a
/// restricted, Medium-integrity copy of this process's token and capture its
/// output. stdin is empty. Failure to launch is an error, never a skip.
pub fn run_non_elevated(cmd: &mut Command) -> Result<Output> {
    let token = restricted_token()?;
    let mut line = command_line(cmd);
    let mut env = environment_block(cmd);
    let cwd: Option<Vec<u16>> = cmd.get_current_dir().map(|dir| {
        let mut w = wide(dir.as_os_str());
        w.push(0);
        w
    });
    let cwd_ptr = cwd.as_ref().map_or(null(), |w| w.as_ptr());

    let (stdin_read, stdin_write) = pipe(true)?;
    let (out_read, out_write) = pipe(false)?;
    let (err_read, err_write) = pipe(false)?;
    drop(stdin_write); // the child sees EOF on stdin

    // SAFETY: all-zero is a valid STARTUPINFOW before the fields are set.
    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = size_of::<STARTUPINFOW>() as u32;
    startup.dwFlags = STARTF_USESTDHANDLES;
    startup.hStdInput = stdin_read.as_raw_handle() as _;
    startup.hStdOutput = out_write.as_raw_handle() as _;
    startup.hStdError = err_write.as_raw_handle() as _;
    // SAFETY: all-zero is a valid PROCESS_INFORMATION out-parameter.
    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };

    // SAFETY: `token` is a primary token; `line`, `env` and `cwd` are
    // NUL-terminated UTF-16 buffers alive for the call; `startup` names
    // inheritable pipe handles; `info` is a valid out-pointer.
    let ok = unsafe {
        CreateProcessAsUserW(
            token.as_raw_handle() as _,
            null(),
            line.as_mut_ptr(),
            null(),
            null(),
            1,
            CREATE_UNICODE_ENVIRONMENT,
            env.as_mut_ptr().cast(),
            cwd_ptr,
            &startup,
            &mut info,
        )
    };
    let api = if ok != 0 {
        "CreateProcessAsUserW"
    } else {
        let error = last_error();
        if error != ERROR_PRIVILEGE_NOT_HELD {
            bail!("CreateProcessAsUserW failed: {error}");
        }
        // SAFETY: as above; this API takes no inherit flag and inherits the
        // standard handles named by STARTF_USESTDHANDLES.
        let ok = unsafe {
            CreateProcessWithTokenW(
                token.as_raw_handle() as _,
                0,
                null(),
                line.as_mut_ptr(),
                CREATE_UNICODE_ENVIRONMENT,
                env.as_mut_ptr().cast(),
                cwd_ptr,
                &startup,
                &mut info,
            )
        };
        ensure!(
            ok != 0,
            "CreateProcessAsUserW (1314) and CreateProcessWithTokenW both failed: {}",
            last_error()
        );
        "CreateProcessWithTokenW"
    };
    *LAUNCH_API.lock().unwrap() = api;
    let process = owned(info.hProcess);
    drop(owned(info.hThread));
    // Close our copies of the child's ends so the readers see EOF at exit.
    drop((stdin_read, out_write, err_write));
    let (stdout, stderr) = (drain(out_read), drain(err_read));

    // SAFETY: `process` is a valid process handle.
    let wait = unsafe { WaitForSingleObject(process.as_raw_handle() as _, INFINITE) };
    ensure!(wait == WAIT_OBJECT_0, "WaitForSingleObject returned {wait}");
    let mut code = 0u32;
    // SAFETY: valid process handle and out-pointer.
    ensure!(
        unsafe { GetExitCodeProcess(process.as_raw_handle() as _, &mut code) } != 0,
        "GetExitCodeProcess failed: {}",
        last_error()
    );
    use std::os::windows::process::ExitStatusExt as _;
    Ok(Output {
        status: ExitStatus::from_raw(code),
        stdout: stdout
            .join()
            .map_err(|_| anyhow::anyhow!("stdout reader panicked"))?,
        stderr: stderr
            .join()
            .map_err(|_| anyhow::anyhow!("stderr reader panicked"))?,
    })
}
