//! Privileged configuration trust policy (SM-04, SM-05; issue #79 / #91).
//!
//! A system service runs as root but reads files in the installing user's
//! home. Root must therefore refuse a path that an *untrusted* principal can
//! change, or it would act on attacker-chosen content (or write through an
//! attacker-chosen link).
//!
//! - **Trusted principals:** root and the one UID recorded at install time.
//! - **Check:** descend from `/` one component at a time with no-follow,
//!   handle-relative `openat`, `fstat` every opened handle and refuse when an
//!   untrusted principal can write to the file or any directory above it. The
//!   path is canonical (the installer resolves root-owned system symlinks such
//!   as macOS `/var` once), so any symlink met here is refused.
//! - **A sticky, group/world-writable directory** (`/tmp`) is accepted only
//!   when the next component is owned by a trusted principal: the sticky bit
//!   stops everyone else from renaming or removing it.
//! - **Never chown/chmod.** A refusal names the path, owner and mode.
//! - The config is parsed only from the verified handle.

use std::ffi::{CString, OsStr};
use std::fs::File;
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::RoleDoc;
use crate::error::CliError;

/// Set by the root-owned registration of a system service: the UID whose
/// files the service may trust besides root. Its presence turns the checks on.
pub const ENV_TRUSTED_UID: &str = "IROH_TUNNEL_TRUSTED_UID";
const ENV_STATE_DIR: &str = "IROH_TUNNEL_STATE_DIR";

const S_IFMT: u32 = 0o170_000;
const S_IFDIR: u32 = 0o040_000;
const S_IFREG: u32 = 0o100_000;
const S_ISVTX: u32 = 0o1_000;
const WRITABLE_BY_OTHERS: u32 = 0o022;

fn refuse(msg: String) -> anyhow::Error {
    CliError::Permission(msg).into()
}

/// The principals whose ownership the trust walk accepts.
///
/// The service binary is checked with [`TrustedOwners::RootOnly`]: the uid
/// pinned for the config must never be able to change what root executes.
/// The user's config, state directory and status files use
/// [`TrustedOwners::RootAnd`] with that pinned uid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TrustedOwners {
    /// Only root (uid 0).
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "used by the managed service binary (unix-system-binary-trust task 3)"
        )
    )]
    RootOnly,
    /// Root and the given uid.
    RootAnd(u32),
}

impl TrustedOwners {
    /// Whether a file or directory owned by `owner` is trusted.
    pub(crate) fn trusts(self, owner: u32) -> bool {
        owner == 0 || matches!(self, Self::RootAnd(uid) if uid == owner)
    }

    fn describe(self) -> String {
        match self {
            Self::RootOnly => "root".to_owned(),
            Self::RootAnd(uid) => format!("root or the installing user (uid {uid})"),
        }
    }
}

/// The type a checked path must have.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EntryKind {
    File,
    Dir,
}

impl EntryKind {
    fn type_bits(self) -> u32 {
        match self {
            Self::File => S_IFREG,
            Self::Dir => S_IFDIR,
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Dir => "directory",
        }
    }
}

fn owner_and_mode(owner: u32, mode: u32) -> String {
    format!("(owner uid {owner}, mode {:04o})", mode & 0o7777)
}

/// The policy for the final entry of a checked path, from its metadata only:
/// it has the expected `kind`, a trusted owner and no group or other write
/// bit. The reason names the owner and mode; the caller adds the path.
///
/// # Errors
///
/// The refusal reason, naming the owner uid and mode.
pub(crate) fn check_entry(
    owner: u32,
    mode: u32,
    kind: EntryKind,
    owners: TrustedOwners,
) -> std::result::Result<(), String> {
    let described = owner_and_mode(owner, mode);
    if mode & S_IFMT != kind.type_bits() {
        return Err(format!("{described} is not a regular {}", kind.noun()));
    }
    if !owners.trusts(owner) || mode & WRITABLE_BY_OTHERS != 0 {
        return Err(format!(
            "{described} must be owned by {} and not writable by group or others; nothing was \
             changed. Fix it (e.g. `chmod go-w`) and re-run",
            owners.describe()
        ));
    }
    Ok(())
}

/// `open("/")` as a directory handle.
fn open_root() -> Result<File> {
    // SAFETY: the path is a valid NUL-terminated string; a returned fd is
    // owned exclusively by the `File` created from it.
    let fd = unsafe {
        libc::open(
            c"/".as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error()).context("cannot open /");
    }
    // SAFETY: `fd` is a freshly opened, valid descriptor nobody else owns.
    Ok(unsafe { File::from_raw_fd(fd) })
}

/// `openat(parent, name)` without following a symlink in `name`.
fn open_child(parent: &File, name: &OsStr, directory: bool) -> std::io::Result<File> {
    let name = CString::new(name.as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let mut flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK;
    if directory {
        flags |= libc::O_DIRECTORY;
    }
    // SAFETY: `parent` is an open descriptor and `name` is NUL-terminated;
    // a returned fd is owned exclusively by the `File` created from it.
    let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `fd` is a freshly opened, valid descriptor nobody else owns.
    Ok(unsafe { File::from_raw_fd(fd) })
}

struct Entry {
    file: File,
    path: PathBuf,
    owner: u32,
    mode: u32,
}

impl Entry {
    fn new(file: File, path: PathBuf) -> Result<Self> {
        let meta = file
            .metadata()
            .with_context(|| format!("cannot inspect {}", path.display()))?;
        Ok(Self {
            file,
            path,
            owner: meta.uid(),
            mode: meta.mode(),
        })
    }

    fn describe(&self) -> String {
        format!(
            "{} (owner uid {}, mode {:04o})",
            self.path.display(),
            self.owner,
            self.mode & 0o7777
        )
    }
}

/// The `Normal` components of an absolute, canonical `path`.
fn components(path: &Path) -> Result<Vec<&OsStr>> {
    if !path.is_absolute() {
        return Err(refuse(format!(
            "{} is not absolute; the service pins an absolute path",
            path.display()
        )));
    }
    // `Path::components` silently drops interior `.`, so check the raw bytes.
    let dotted = path
        .as_os_str()
        .as_bytes()
        .split(|byte| *byte == b'/')
        .any(|segment| segment == b"." || segment == b"..");
    if dotted {
        return Err(refuse(format!(
            "{} is not canonical (contains `.` or `..`); re-run the install to pin the \
             resolved path",
            path.display()
        )));
    }
    path.components()
        .filter(|component| !matches!(component, Component::RootDir))
        .map(|component| match component {
            Component::Normal(name) => Ok(name),
            _ => Err(refuse(format!(
                "{} is not canonical (contains `.` or `..`); re-run the install to pin the \
                 resolved path",
                path.display()
            ))),
        })
        .collect()
}

fn root_entry() -> Result<Entry> {
    Entry::new(open_root()?, PathBuf::from("/"))
}

/// Open `parent/name` without following a link and check that `parent` keeps
/// it safe.
fn descend(parent: &Entry, name: &OsStr, directory: bool, owners: TrustedOwners) -> Result<Entry> {
    let opened = open_child(&parent.file, name, directory);
    adopt_child(parent, name, directory, opened, owners)
}

/// Turn the result of opening `parent/name` into a verified child entry.
fn adopt_child(
    parent: &Entry,
    name: &OsStr,
    directory: bool,
    opened: std::io::Result<File>,
    owners: TrustedOwners,
) -> Result<Entry> {
    let child_path = parent.path.join(name);
    let file = opened.map_err(|e| {
        refuse(format!(
            "{} is not a plain {} reachable without following a symlink ({e}); refusing to \
             use it as root. Replace the link with the real path and re-run the install",
            child_path.display(),
            if directory { "directory" } else { "file" },
        ))
    })?;
    let child = Entry::new(file, child_path)?;
    check_parent(parent, &child, owners)?;
    Ok(child)
}

/// Descend from `/` to `path`, verifying every handle, and return the handle
/// of the final component.
fn walk(path: &Path, owners: TrustedOwners, kind: EntryKind) -> Result<Entry> {
    let names = components(path)?;
    let mut current = root_entry()?;
    let last = names.len().saturating_sub(1);
    for (index, name) in names.iter().enumerate() {
        let directory = index != last || kind == EntryKind::Dir;
        current = descend(&current, name, directory, owners)?;
    }
    check_final(&current, owners, kind)?;
    Ok(current)
}

/// The directory above `child` must not let an untrusted principal replace or
/// alter `child`.
fn check_parent(parent: &Entry, child: &Entry, owners: TrustedOwners) -> Result<()> {
    let bad_owner = !owners.trusts(parent.owner);
    let writable = parent.mode & WRITABLE_BY_OTHERS != 0;
    let sticky_protects = parent.mode & S_ISVTX != 0 && owners.trusts(child.owner);
    if bad_owner || (writable && !sticky_protects) {
        let why = if bad_owner {
            "is owned by an untrusted user"
        } else {
            "is writable by group or others"
        };
        return Err(refuse(format!(
            "unsafe directory {} {why}, so another user could replace {} before the service \
             (root) uses it. Make it owned by {} and not group- or world-writable (e.g. \
             `chmod go-w`); nothing was changed",
            parent.describe(),
            child.path.display(),
            owners.describe()
        )));
    }
    Ok(())
}

fn check_final(entry: &Entry, owners: TrustedOwners, kind: EntryKind) -> Result<()> {
    check_entry(entry.owner, entry.mode, kind, owners).map_err(|why| {
        refuse(format!(
            "unsafe {}: {} {why}",
            kind.noun(),
            entry.path.display()
        ))
    })
}

/// Verify that `path` is a regular file only `owners` can change: the file
/// and every directory above it, reached without following a link (the
/// sticky-directory rule applies). Production passes
/// [`TrustedOwners::RootOnly`] for the binary a root service runs.
///
/// # Errors
///
/// [`CliError::Permission`] naming the unsafe path, owner and mode.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "used by the managed service binary (unix-system-binary-trust task 3)"
    )
)]
pub(crate) fn verify_binary(path: &Path, owners: TrustedOwners) -> Result<()> {
    walk(path, owners, EntryKind::File).map(drop)
}

/// Make sure the directory `path` exists and only `owners` can change it.
///
/// Existing components are verified and never modified. Each missing one is
/// created with `mkdirat` through its verified parent handle, opened without
/// following a link and `fchmod`ed to `mode` through that handle, so the
/// umask cannot change it and no pathname `chmod` is issued.
///
/// # Errors
///
/// [`CliError::Permission`] when an existing component is unsafe (nothing
/// below it is created), or the creation error.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "used by the managed service binary (unix-system-binary-trust task 3)"
    )
)]
pub(crate) fn ensure_dir(path: &Path, owners: TrustedOwners, mode: u32) -> Result<()> {
    let mut current = root_entry()?;
    for name in components(path)? {
        current = match open_child(&current.file, name, true) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                create_child_dir(&current, name, owners, mode)?
            }
            opened => adopt_child(&current, name, true, opened, owners)?,
        };
    }
    check_final(&current, owners, EntryKind::Dir)
}

/// `mkdirat(parent, name, mode)` then `fchmod` the new handle to `mode`; an
/// entry that appeared meanwhile is only opened and verified.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "used by the managed service binary (unix-system-binary-trust task 3)"
    )
)]
fn create_child_dir(
    parent: &Entry,
    name: &OsStr,
    owners: TrustedOwners,
    mode: u32,
) -> Result<Entry> {
    // The new child does not exist yet, so `check_parent` cannot vouch for
    // `parent`: it must itself pass the policy before anything is created in
    // it (a sticky world-writable directory is not good enough here).
    check_final(parent, owners, EntryKind::Dir)?;
    let child_path = parent.path.join(name);
    let name_c = CString::new(name.as_bytes())
        .with_context(|| format!("{} contains a NUL byte", child_path.display()))?;
    // Created owner-only; the exact `mode` is applied below with `fchmod`, so
    // the directory is never wider than intended, whatever the umask.
    // SAFETY: `parent.file` is an open, verified directory descriptor and
    // `name_c` is NUL-terminated; mkdirat(2) does not retain either.
    let made = unsafe { libc::mkdirat(parent.file.as_raw_fd(), name_c.as_ptr(), 0o700) };
    if made != 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            return descend(parent, name, true, owners);
        }
        return Err(error).with_context(|| format!("cannot create {}", child_path.display()));
    }
    let child = descend(parent, name, true, owners)?;
    child
        .file
        .set_permissions(std::fs::Permissions::from_mode(mode))
        .with_context(|| format!("cannot set the mode of {}", child_path.display()))?;
    Entry::new(child.file, child.path)
}

/// Verify a directory (e.g. the state directory) the root service writes in.
///
/// # Errors
///
/// [`CliError::Permission`] naming the unsafe path, owner and mode, or when the
/// directory is missing (the service never creates it as root).
pub fn verify_dir(path: &Path, trusted_uid: u32) -> Result<()> {
    if !path.exists() {
        return Err(refuse(format!(
            "{} does not exist; create it as the installing user (`mkdir -p`) — the service \
             does not create directories in a user's home as root",
            path.display()
        )));
    }
    walk(path, TrustedOwners::RootAnd(trusted_uid), EntryKind::Dir).map(drop)
}

/// Verify `path` and return its text read from the verified handle.
///
/// # Errors
///
/// [`CliError::Permission`] when the path or any parent is unsafe.
pub fn read_verified(path: &Path, trusted_uid: u32) -> Result<String> {
    let mut entry = walk(path, TrustedOwners::RootAnd(trusted_uid), EntryKind::File)?;
    let mut text = String::new();
    entry
        .file
        .read_to_string(&mut text)
        .with_context(|| format!("failed to read config: {}", path.display()))?;
    Ok(text)
}

/// Resolve `path` once to its canonical form (root-owned system symlinks such
/// as macOS `/var`), for pinning into a registration.
///
/// # Errors
///
/// [`CliError::Config`] when the path does not exist.
pub fn canonical(path: &Path) -> Result<PathBuf> {
    std::fs::canonicalize(path).map_err(|e| {
        CliError::Config(format!(
            "cannot resolve {}: {e}. Create it first or pass the right path",
            path.display()
        ))
        .into()
    })
}

/// Atomically publish `dir/name` with the bytes of `source` and exactly
/// `mode`, through a freshly verified directory handle: the temp file is
/// created `O_EXCL|O_NOFOLLOW`, streamed, `fchmod`ed (so the umask cannot
/// change the mode), synced and renamed with `renameat`, so a link planted
/// in the directory is never followed by root. The directory is re-verified
/// on every call, which closes the swap-after-start race for the status
/// snapshot (SM-04). On any failure the temp file is unlinked and an
/// existing `dir/name` is left untouched.
///
/// # Errors
///
/// [`CliError::Permission`] when the directory is no longer safe, or the
/// I/O error of creating, writing or renaming the file.
pub(crate) fn publish(
    dir: &Path,
    owners: TrustedOwners,
    name: &str,
    temp: &str,
    source: &mut dyn Read,
    mode: u32,
) -> Result<()> {
    let dir_entry = walk(dir, owners, EntryKind::Dir)?;
    let fd = dir_entry.file.as_raw_fd();
    let temp_c = cstring(temp)?;
    let name_c = cstring(name)?;
    // SAFETY: `fd` is the verified directory handle, kept open by `dir_entry`;
    // the names are NUL-terminated; a returned fd is owned by the new `File`.
    let raw = unsafe {
        libc::openat(
            fd,
            temp_c.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600 as libc::c_uint,
        )
    };
    if raw < 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("cannot create {temp} in {}", dir.display()));
    }
    // SAFETY: `raw` is a freshly opened descriptor nobody else owns.
    let mut file = unsafe { File::from_raw_fd(raw) };
    let renamed = fill(&mut file, source, mode)
        .with_context(|| format!("failed to write {temp} in {}", dir.display()))
        .and_then(|()| {
            // SAFETY: `fd` is the verified directory handle kept open by
            // `dir_entry` for this whole function, and both names are
            // NUL-terminated `CString`s that outlive the call.
            if unsafe { libc::renameat(fd, temp_c.as_ptr(), fd, name_c.as_ptr()) } == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
                    .with_context(|| format!("failed to finalize {name} in {}", dir.display()))
            }
        });
    if renamed.is_err() {
        // SAFETY: `fd` is still the open verified directory handle and
        // `temp_c` a live NUL-terminated name; this removes only our own temp
        // file, and its failure is irrelevant to the error being returned.
        let _ = unsafe { libc::unlinkat(fd, temp_c.as_ptr(), 0) };
    }
    renamed
}

/// Stream `source` into `file`, set its exact `mode` and sync it.
fn fill(file: &mut File, source: &mut dyn Read, mode: u32) -> std::io::Result<()> {
    std::io::copy(source, file)?;
    file.set_permissions(std::fs::Permissions::from_mode(mode))?;
    file.sync_all()
}

/// Read `dir/name` through a verified directory handle without following a
/// link; `None` when absent or unreadable.
pub(crate) fn read_in(dir: &Path, owners: TrustedOwners, name: &str) -> Option<String> {
    let dir_entry = walk(dir, owners, EntryKind::Dir)
        .inspect_err(|e| tracing::debug!("not reading {name}: {e:#}"))
        .ok()?;
    let mut file = open_child(&dir_entry.file, OsStr::new(name), false).ok()?;
    let mut text = String::new();
    file.read_to_string(&mut text).ok()?;
    Some(text)
}

/// Remove `dir/name` through a verified directory handle (never follows a
/// link); an absent name is fine.
///
/// # Errors
///
/// [`CliError::Permission`] when the directory is unsafe, or the unlink error.
pub(crate) fn remove_in(dir: &Path, owners: TrustedOwners, name: &str) -> Result<()> {
    let dir_entry = walk(dir, owners, EntryKind::Dir)?;
    unlink_in(&dir_entry, name).map(drop)
}

/// Remove `dir/name` (a link is unlinked itself, never followed), then
/// remove `dir` when it is left empty. Absence is checked without following
/// a link: a missing `dir` or `name` is reported as `false`.
///
/// # Errors
///
/// [`CliError::Permission`] when `dir` or a directory above it is unsafe, or
/// the unlink error.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "used by the managed service binary (unix-system-binary-trust task 3)"
    )
)]
pub(crate) fn remove_entry(dir: &Path, owners: TrustedOwners, name: &str) -> Result<bool> {
    match std::fs::symlink_metadata(dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e).with_context(|| format!("cannot inspect {}", dir.display())),
        Ok(_) => {}
    }
    let (Some(parent), Some(dir_name)) = (dir.parent(), dir.file_name()) else {
        return Err(refuse(format!(
            "{} has no parent directory; refusing to remove from it",
            dir.display()
        )));
    };
    let parent_entry = walk(parent, owners, EntryKind::Dir)?;
    let dir_entry = descend(&parent_entry, dir_name, true, owners)?;
    check_final(&dir_entry, owners, EntryKind::Dir)?;
    let removed = unlink_in(&dir_entry, name)?;
    remove_dir_if_empty(&parent_entry, dir_name)
        .with_context(|| format!("failed to remove {}", dir.display()))?;
    Ok(removed)
}

/// `unlinkat(dir, name, 0)`; `false` when `name` is absent.
fn unlink_in(dir: &Entry, name: &str) -> Result<bool> {
    let name_c = cstring(name)?;
    // SAFETY: `dir` keeps the verified directory open; `name_c` is
    // NUL-terminated and outlives the call.
    if unsafe { libc::unlinkat(dir.file.as_raw_fd(), name_c.as_ptr(), 0) } == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::NotFound {
        Ok(false)
    } else {
        Err(error).with_context(|| format!("failed to remove {name} in {}", dir.path.display()))
    }
}

/// `unlinkat(parent, name, AT_REMOVEDIR)`, ignoring a directory that is not
/// empty or already gone.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "used by the managed service binary (unix-system-binary-trust task 3)"
    )
)]
fn remove_dir_if_empty(parent: &Entry, name: &OsStr) -> Result<()> {
    let name_c = CString::new(name.as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    // SAFETY: `parent` keeps the verified directory open; `name_c` is
    // NUL-terminated and outlives the call.
    if unsafe { libc::unlinkat(parent.file.as_raw_fd(), name_c.as_ptr(), libc::AT_REMOVEDIR) } == 0
    {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ENOTEMPTY | libc::EEXIST | libc::ENOENT) => Ok(()),
        _ => Err(error.into()),
    }
}

fn cstring(text: &str) -> Result<CString> {
    CString::new(text).with_context(|| format!("{text:?} contains a NUL byte"))
}

/// The trust policy requested by the registration, if any.
pub(crate) fn trusted_uid_from_env() -> Result<Option<u32>> {
    match std::env::var(ENV_TRUSTED_UID) {
        Ok(value) => value.trim().parse().map(Some).map_err(|_| {
            CliError::Config(format!(
                "{ENV_TRUSTED_UID} must be a numeric uid; re-run the service install"
            ))
            .into()
        }),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(e) => Err(CliError::Config(format!("{ENV_TRUSTED_UID}: {e}")).into()),
    }
}

/// Whether this is root run by a service manager with no trust pin: the
/// registration predates the policy. systemd sets `INVOCATION_ID` and launchd
/// `XPC_SERVICE_NAME`; a foreground `sudo` run sets neither and is unaffected.
/// BusyBox init sets nothing, so that backend cannot be told apart (it is
/// refused only after re-install, which pins it).
fn is_unpinned_service(root: bool, env_set: impl Fn(&str) -> bool) -> bool {
    root && ["INVOCATION_ID", "XPC_SERVICE_NAME"]
        .iter()
        .any(|name| env_set(name))
}

fn unpinned_refusal() -> String {
    "this system service was installed before configuration trust checks existed, so it would \
     read a user's file as root without verifying it. Nothing was changed. Re-install it with \
     `sudo iroh-tunnel <role> service install --system`"
        .to_owned()
}

/// Load a role config for `run`.
///
/// A plain read normally. Under a system service (`IROH_TUNNEL_TRUSTED_UID`
/// set by the root-owned registration) the state directory and the config are
/// verified first and the config is parsed only from the verified handle. The
/// service never writes a generated key into the user's file (it would leave a
/// root-owned file there), so a missing key is an error.
///
/// # Errors
///
/// [`CliError::Permission`] for an unsafe path, [`CliError::Config`] for an
/// invalid or keyless config.
pub(crate) fn load_config<C: RoleDoc>(path: &Path) -> Result<C> {
    let Some(uid) = trusted_uid_from_env()? else {
        // SM-04: fail closed. A root process started by a service manager
        // without a pin is a registration from before the policy.
        // SAFETY: geteuid(2) has no preconditions and cannot fail.
        let root = unsafe { libc::geteuid() } == 0;
        if is_unpinned_service(root, |name| std::env::var_os(name).is_some()) {
            return Err(CliError::Permission(unpinned_refusal()).into());
        }
        return crate::config::load_role_config::<C>(path);
    };
    if let Some(state) = std::env::var_os(ENV_STATE_DIR).filter(|d| !d.is_empty()) {
        verify_dir(Path::new(&state), uid)?;
    }
    let text = read_verified(path, uid)?;
    let mut cfg = crate::config::parse_role_config::<C>(&text, path)?;
    if cfg.node_mut().secret_key.is_empty() {
        return Err(CliError::Config(format!(
            "{} has no secret_key, and a system service does not write into a user's file as \
             root. Run `iroh-tunnel <role> config keygen --config {}` as the user, then restart",
            path.display(),
            path.display()
        ))
        .into());
    }
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_under_a_service_manager_without_a_pin_is_a_legacy_registration() {
        let manager = |name: &str| name == "INVOCATION_ID";
        let launchd = |name: &str| name == "XPC_SERVICE_NAME";
        let none = |_: &str| false;
        assert!(is_unpinned_service(true, manager));
        assert!(is_unpinned_service(true, launchd));
        // A foreground root run (no manager) and any non-root run are not.
        assert!(!is_unpinned_service(true, none));
        assert!(!is_unpinned_service(false, manager));
    }

    #[test]
    fn legacy_refusal_tells_the_operator_to_reinstall() {
        let msg = unpinned_refusal();
        assert!(msg.contains("service install --system"), "{msg}");
    }
    use std::os::unix::fs::symlink;

    fn me() -> u32 {
        // SAFETY: geteuid(2) has no preconditions and cannot fail.
        unsafe { libc::geteuid() }
    }

    /// `<tmp>/a/b/serve.toml` with sane permissions.
    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let dir = root.join("a/b");
        std::fs::create_dir_all(&dir).unwrap();
        for d in [root.join("a"), dir.clone()] {
            std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let file = dir.join("serve.toml");
        std::fs::write(&file, "x = 1\n").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        (tmp, file)
    }

    fn mode(path: &Path, mode: u32) {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    fn refusal(result: Result<String>) -> String {
        let err = result.expect_err("must be refused");
        match err.downcast_ref::<CliError>() {
            Some(CliError::Permission(msg)) => msg.clone(),
            other => panic!("expected Permission, got {other:?}"),
        }
    }

    #[test]
    fn safe_path_is_read_from_the_handle() {
        let (_tmp, file) = fixture();
        assert_eq!(read_verified(&file, me()).unwrap(), "x = 1\n");
    }

    #[test]
    fn an_owner_that_is_neither_root_nor_trusted_is_refused() {
        let (_tmp, file) = fixture();
        let other = if me() == 4242 { 4243 } else { 4242 };
        // The refusal may come from the file or, when the fixture lives under
        // a sticky `/tmp`, from the directory above it; either way a path that
        // a different principal owns must be named and refused.
        let msg = refusal(read_verified(&file, other));
        assert!(msg.contains("unsafe") && msg.contains("/"), "{msg}");
        // And the same path is accepted when its owner is the trusted uid.
        assert!(read_verified(&file, me()).is_ok());
    }

    #[test]
    fn world_or_group_writable_file_is_refused_naming_path_and_mode() {
        let (_tmp, file) = fixture();
        for bad in [0o666, 0o620, 0o602] {
            mode(&file, bad);
            let msg = refusal(read_verified(&file, me()));
            assert!(msg.contains(file.to_str().unwrap()), "{msg}");
            assert!(msg.contains(&format!("{bad:04o}")), "{msg}");
        }
    }

    #[test]
    fn group_or_world_writable_parent_is_refused_even_if_the_file_is_private() {
        let (_tmp, file) = fixture();
        for dir in [
            file.parent().unwrap(),
            file.parent().unwrap().parent().unwrap(),
        ] {
            for bad in [0o775, 0o757] {
                mode(dir, bad);
                let msg = refusal(read_verified(&file, me()));
                assert!(msg.contains(dir.to_str().unwrap()), "{msg}");
            }
            mode(dir, 0o755);
        }
    }

    #[test]
    fn symlinked_file_or_parent_is_refused() {
        let (tmp, file) = fixture();
        let root = tmp.path().canonicalize().unwrap();
        let link = root.join("link.toml");
        symlink(&file, &link).unwrap();
        let msg = refusal(read_verified(&link, me()));
        assert!(msg.contains("symlink"), "{msg}");

        let dir_link = root.join("dlink");
        symlink(root.join("a"), &dir_link).unwrap();
        let msg = refusal(read_verified(&dir_link.join("b/serve.toml"), me()));
        assert!(msg.contains("symlink"), "{msg}");
    }

    #[test]
    fn dot_dot_and_relative_paths_are_refused() {
        let (_tmp, file) = fixture();
        let dotted = file.parent().unwrap().join("../b/serve.toml");
        assert!(refusal(read_verified(&dotted, me())).contains("canonical"));
        assert!(refusal(read_verified(Path::new("serve.toml"), me())).contains("absolute"));
    }

    #[test]
    fn sticky_world_writable_parent_is_accepted_only_for_a_trusted_child() {
        let (tmp, _file) = fixture();
        let root = tmp.path().canonicalize().unwrap();
        let sticky = root.join("sticky");
        std::fs::create_dir(&sticky).unwrap();
        mode(&sticky, 0o1777);
        let inner = sticky.join("dir");
        std::fs::create_dir(&inner).unwrap();
        mode(&inner, 0o700);
        let file = inner.join("serve.toml");
        std::fs::write(&file, "y").unwrap();
        mode(&file, 0o600);
        assert_eq!(read_verified(&file, me()).unwrap(), "y");

        // Without the sticky bit the same directory is refused.
        mode(&sticky, 0o777);
        assert!(refusal(read_verified(&file, me())).contains("writable"));
    }

    #[test]
    fn directory_check_requires_an_existing_safe_directory() {
        let (_tmp, file) = fixture();
        let dir = file.parent().unwrap();
        verify_dir(dir, me()).unwrap();
        let missing = dir.join("nope");
        assert!(verify_dir(&missing, me())
            .unwrap_err()
            .to_string()
            .contains("does not exist"));
        // A file is not a directory.
        assert!(verify_dir(&file, me()).is_err());
        mode(dir, 0o777);
        assert!(verify_dir(dir, me()).is_err());
    }

    #[test]
    fn publish_replaces_atomically_and_never_follows_a_planted_link() {
        let (tmp, file) = fixture();
        let dir = file.parent().unwrap();
        put(dir, "serve-status.json", "serve-status.json.tmp.1", b"one").unwrap();
        put(dir, "serve-status.json", "serve-status.json.tmp.2", b"two").unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("serve-status.json")).unwrap(),
            "two"
        );
        assert_eq!(
            read_in(dir, mine(), "serve-status.json").as_deref(),
            Some("two")
        );

        // A link planted at the final name is replaced, not written through.
        let victim = tmp.path().canonicalize().unwrap().join("victim");
        std::fs::write(&victim, "keep").unwrap();
        std::fs::remove_file(dir.join("serve-status.json")).unwrap();
        symlink(&victim, dir.join("serve-status.json")).unwrap();
        assert!(read_in(dir, mine(), "serve-status.json").is_none());
        put(
            dir,
            "serve-status.json",
            "serve-status.json.tmp.3",
            b"three",
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
        assert_eq!(
            std::fs::read_to_string(dir.join("serve-status.json")).unwrap(),
            "three"
        );

        // A link planted at the temp name makes the create fail (O_EXCL).
        symlink(&victim, dir.join("t")).unwrap();
        assert!(put(dir, "serve-status.json", "t", b"x").is_err());
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");

        // Removal unlinks a link itself, never its target; absent is fine.
        remove_in(dir, mine(), "t").unwrap();
        remove_in(dir, mine(), "t").unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
    }

    #[test]
    fn publish_refuses_a_directory_that_became_unsafe() {
        let (_tmp, file) = fixture();
        let dir = file.parent().unwrap();
        mode(dir, 0o777);
        assert!(put(dir, "s", "s.tmp", b"x").is_err());
        assert!(!dir.join("s").exists());
    }

    // ---- Root-only trust mode (unix-system-binary-trust) ----

    fn mine() -> TrustedOwners {
        TrustedOwners::RootAnd(me())
    }

    fn other() -> u32 {
        me() + 1
    }

    /// The status-writer shape of `publish`: bytes, mode 0644.
    fn put(dir: &Path, name: &str, temp: &str, bytes: &[u8]) -> Result<()> {
        publish(dir, mine(), name, temp, &mut &bytes[..], 0o644)
    }

    fn perm_message(result: Result<()>) -> String {
        refusal(result.map(|()| String::new()))
    }

    fn mode_of(path: &Path) -> u32 {
        std::fs::symlink_metadata(path).unwrap().mode() & 0o7777
    }

    /// `<tmp>/bin/<file>` with 0755 directories and a 0755 file.
    fn binary_fixture() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let dir = root.join("bin");
        std::fs::create_dir(&dir).unwrap();
        mode(&dir, 0o755);
        let file = dir.join("iroh-tunnel");
        std::fs::write(&file, "#!/bin/sh\n").unwrap();
        mode(&file, 0o755);
        (tmp, file)
    }

    #[test]
    fn root_only_accepts_a_system_binary() {
        let sh = std::fs::canonicalize("/bin/sh").unwrap();
        verify_binary(&sh, TrustedOwners::RootOnly).unwrap();
    }

    #[test]
    fn root_only_refuses_a_file_owned_by_the_test_user() {
        if me() == 0 {
            return;
        }
        let (_tmp, file) = binary_fixture();
        let msg = perm_message(verify_binary(&file, TrustedOwners::RootOnly));
        // The walk refuses the first user-owned component (the tempdir or
        // the fixture itself); either way the binary's path prefix is named.
        assert!(msg.contains("owner uid"), "{msg}");
        assert!(
            file.ancestors()
                .any(|p| p != Path::new("/") && msg.contains(p.to_str().unwrap())),
            "{msg}"
        );
    }

    #[test]
    fn binary_refuses_untrusted_parent_naming_it() {
        let (tmp, file) = binary_fixture();
        let msg = perm_message(verify_binary(&file, TrustedOwners::RootAnd(other())));
        let root = tmp.path().canonicalize().unwrap();
        let named = [root.clone(), root.join("bin")]
            .into_iter()
            .chain(root.ancestors().map(Path::to_path_buf))
            .filter(|p| p != Path::new("/"))
            .find(|p| msg.contains(&format!("{} (owner uid", p.display())));
        let dir = named.unwrap_or_else(|| panic!("no fixture directory named: {msg}"));
        assert!(
            msg.contains(&format!("mode {:04o}", mode_of(&dir))),
            "{msg}"
        );
    }

    #[test]
    fn binary_refuses_0775_naming_the_mode() {
        let (_tmp, file) = binary_fixture();
        mode(&file, 0o775);
        let msg = perm_message(verify_binary(&file, mine()));
        assert!(msg.contains(file.to_str().unwrap()), "{msg}");
        assert!(msg.contains("0775"), "{msg}");
    }

    #[test]
    fn binary_refuses_a_symlink_and_a_directory() {
        let (tmp, file) = binary_fixture();
        let link = tmp.path().canonicalize().unwrap().join("bin/link");
        symlink(&file, &link).unwrap();
        let msg = perm_message(verify_binary(&link, mine()));
        assert!(msg.contains("symlink"), "{msg}");

        let dir = file.parent().unwrap();
        let msg = perm_message(verify_binary(dir, mine()));
        assert!(msg.contains(dir.to_str().unwrap()), "{msg}");
    }

    /// `<tmp>/sticky(1777)/inner(0755)/iroh-tunnel(0755)`.
    fn sticky_binary() -> (tempfile::TempDir, PathBuf) {
        let (tmp, _file) = binary_fixture();
        let sticky = tmp.path().canonicalize().unwrap().join("sticky");
        std::fs::create_dir(&sticky).unwrap();
        mode(&sticky, 0o1777);
        let inner = sticky.join("inner");
        std::fs::create_dir(&inner).unwrap();
        mode(&inner, 0o755);
        let file = inner.join("iroh-tunnel");
        std::fs::write(&file, "x").unwrap();
        mode(&file, 0o755);
        (tmp, file)
    }

    #[test]
    fn binary_sticky_parent_trusted_child_passes() {
        let (_tmp, file) = sticky_binary();
        verify_binary(&file, mine()).unwrap();
    }

    #[test]
    fn binary_sticky_parent_untrusted_child_fails() {
        let (_tmp, file) = sticky_binary();
        assert!(verify_binary(&file, TrustedOwners::RootAnd(other())).is_err());
    }

    #[test]
    fn binary_refuses_interior_dot_segments() {
        let (tmp, _file) = binary_fixture();
        let root = tmp.path().canonicalize().unwrap();
        for text in [
            format!("{}/bin/./iroh-tunnel", root.display()),
            format!("{}/./bin/iroh-tunnel", root.display()),
            format!("{}/bin/iroh-tunnel/.", root.display()),
        ] {
            let msg = perm_message(verify_binary(Path::new(&text), mine()));
            assert!(msg.contains("not canonical"), "{msg}");
            assert!(msg.contains(&text), "{msg}");
        }
    }

    fn entry_of(path: &Path, owner: u32, mode: u32) -> Entry {
        let mut entry = Entry::new(File::open(path).unwrap(), path.to_path_buf()).unwrap();
        entry.owner = owner;
        entry.mode = mode;
        entry
    }

    #[test]
    fn check_parent_sticky_rule_depends_only_on_the_child_owner() {
        let (tmp, file) = binary_fixture();
        let dir = file.parent().unwrap();
        let _ = tmp;
        let parent = entry_of(dir, me(), 0o040_000 | 0o1777);
        let trusted_child = entry_of(&file, me(), 0o100_755);
        check_parent(&parent, &trusted_child, mine()).unwrap();

        let untrusted_child = entry_of(&file, other(), 0o100_755);
        let msg = check_parent(&parent, &untrusted_child, mine())
            .unwrap_err()
            .to_string();
        assert!(msg.contains(dir.to_str().unwrap()), "{msg}");
        assert!(msg.contains("writable by group or others"), "{msg}");
    }

    #[test]
    fn ensure_dir_creates_nothing_under_a_sticky_world_writable_parent() {
        let (tmp, _file) = binary_fixture();
        let sticky = tmp.path().canonicalize().unwrap().join("sticky");
        std::fs::create_dir(&sticky).unwrap();
        mode(&sticky, 0o1777);
        let msg = perm_message(ensure_dir(&sticky.join("new/inner"), mine(), 0o755));
        assert!(msg.contains(sticky.to_str().unwrap()), "{msg}");
        assert_eq!(std::fs::read_dir(&sticky).unwrap().count(), 0);
    }
    #[test]
    fn check_entry_refuses_other_owner() {
        let msg = check_entry(other(), 0o040_755, EntryKind::Dir, mine()).unwrap_err();
        assert!(msg.contains(&other().to_string()), "{msg}");
        assert!(msg.contains("0755"), "{msg}");
        check_entry(0, 0o040_755, EntryKind::Dir, TrustedOwners::RootOnly).unwrap();
        // The test user is not root, so RootOnly never trusts it.
        if me() != 0 {
            assert!(
                check_entry(me(), 0o100_755, EntryKind::File, TrustedOwners::RootOnly).is_err()
            );
        }
        // Group write and the wrong kind are refused for a trusted owner.
        assert!(check_entry(0, 0o100_775, EntryKind::File, TrustedOwners::RootOnly).is_err());
        assert!(check_entry(0, 0o040_755, EntryKind::File, TrustedOwners::RootOnly).is_err());
    }

    #[test]
    fn ensure_dir_creates_missing_components_0755() {
        let (tmp, _file) = binary_fixture();
        let base = tmp.path().canonicalize().unwrap().join("bin");
        let target = base.join("managed/bin/serve");
        ensure_dir(&target, mine(), 0o755).unwrap();
        for dir in [
            base.join("managed"),
            base.join("managed/bin"),
            target.clone(),
        ] {
            assert!(dir.is_dir(), "{}", dir.display());
            assert_eq!(mode_of(&dir), 0o755, "{}", dir.display());
        }
        // Idempotent over existing, safe directories.
        ensure_dir(&target, mine(), 0o755).unwrap();
    }

    #[test]
    fn ensure_dir_refuses_existing_untrusted_and_leaves_mode() {
        let (tmp, _file) = binary_fixture();
        let open = tmp.path().canonicalize().unwrap().join("bin/open");
        std::fs::create_dir(&open).unwrap();
        mode(&open, 0o777);
        let msg = perm_message(ensure_dir(&open.join("serve"), mine(), 0o755));
        assert!(msg.contains(open.to_str().unwrap()), "{msg}");
        assert!(msg.contains("0777"), "{msg}");
        assert_eq!(mode_of(&open), 0o777);
        assert!(!open.join("serve").exists());
    }

    #[test]
    fn ensure_dir_never_follows_a_planted_link() {
        let (tmp, _file) = binary_fixture();
        let root = tmp.path().canonicalize().unwrap();
        let target = root.join("elsewhere");
        std::fs::create_dir(&target).unwrap();
        mode(&target, 0o700);
        let link = root.join("bin/managed");
        symlink(&target, &link).unwrap();
        let msg = perm_message(ensure_dir(&link.join("serve"), mine(), 0o755));
        assert!(msg.contains("symlink"), "{msg}");
        assert_eq!(mode_of(&target), 0o700);
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);
        assert_eq!(std::fs::read_link(&link).unwrap(), target);
    }

    #[test]
    fn publish_and_ensure_dir_ignore_umask() {
        const NAME: &str = "service::trust::tests::publish_and_ensure_dir_ignore_umask";
        super::super::fixtures::in_isolated_home(NAME, |home| {
            // SAFETY: umask(2) only changes this child process's file mode
            // creation mask; it has no memory-safety preconditions.
            unsafe { libc::umask(0o077) };
            let base = home.canonicalize().unwrap();
            mode(&base, 0o755);
            let dir = base.join("managed/serve");
            ensure_dir(&dir, mine(), 0o755).unwrap();
            assert_eq!(mode_of(&base.join("managed")), 0o755);
            assert_eq!(mode_of(&dir), 0o755);
            publish(&dir, mine(), "iroh-tunnel", "t", &mut &b"bin"[..], 0o755).unwrap();
            assert_eq!(mode_of(&dir.join("iroh-tunnel")), 0o755);
        });
    }

    /// A source that yields some bytes, then fails.
    struct FailingRead(bool);

    impl Read for FailingRead {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if std::mem::replace(&mut self.0, true) {
                return Err(std::io::Error::other("source vanished"));
            }
            buf[..4].copy_from_slice(b"half");
            Ok(4)
        }
    }

    #[test]
    fn publish_failure_before_rename_keeps_old_file_and_no_temp() {
        let (_tmp, file) = binary_fixture();
        let dir = file.parent().unwrap();
        let err = publish(
            dir,
            mine(),
            "iroh-tunnel",
            "copy.tmp",
            &mut FailingRead(false),
            0o755,
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("source vanished"), "{err:#}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "#!/bin/sh\n");
        let names: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("iroh-tunnel")]);
    }

    #[test]
    fn remove_entry_absent_and_dangling_link() {
        let (tmp, _file) = binary_fixture();
        let root = tmp.path().canonicalize().unwrap();
        assert!(!remove_entry(&root.join("missing"), mine(), "iroh-tunnel").unwrap());

        // A dangling link at the name is unlinked and the empty dir removed.
        let role = root.join("role");
        std::fs::create_dir(&role).unwrap();
        mode(&role, 0o755);
        symlink(root.join("nowhere"), role.join("iroh-tunnel")).unwrap();
        assert!(remove_entry(&role, mine(), "iroh-tunnel").unwrap());
        assert!(std::fs::symlink_metadata(&role).is_err());

        // A live link is removed itself; its target is intact.
        std::fs::create_dir(&role).unwrap();
        mode(&role, 0o755);
        let victim = root.join("victim");
        std::fs::write(&victim, "keep").unwrap();
        symlink(&victim, role.join("iroh-tunnel")).unwrap();
        std::fs::write(role.join("other"), "x").unwrap();
        assert!(remove_entry(&role, mine(), "iroh-tunnel").unwrap());
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
        // Not empty: the directory stays.
        assert!(role.is_dir());
        // The name is now absent.
        assert!(!remove_entry(&role, mine(), "iroh-tunnel").unwrap());
    }

    #[test]
    fn canonical_resolves_symlinks_once_and_rejects_missing_paths() {
        let (tmp, file) = fixture();
        let link = tmp.path().canonicalize().unwrap().join("l.toml");
        symlink(&file, &link).unwrap();
        assert_eq!(canonical(&link).unwrap(), file);
        assert!(canonical(&file.with_extension("nope")).is_err());
    }
}
