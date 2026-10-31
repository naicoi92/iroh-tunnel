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
use std::os::unix::fs::MetadataExt;
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

fn trusted(owner: u32, trusted_uid: u32) -> bool {
    owner == 0 || owner == trusted_uid
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

/// Descend from `/` to `path`, verifying every handle, and return the handle
/// of the final component.
fn walk(path: &Path, trusted_uid: u32, want_dir: bool) -> Result<Entry> {
    if !path.is_absolute() {
        return Err(refuse(format!(
            "{} is not absolute; the service pins an absolute path",
            path.display()
        )));
    }
    let mut names = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => names.push(name),
            _ => {
                return Err(refuse(format!(
                    "{} is not canonical (contains `.` or `..`); re-run the install to pin the \
                     resolved path",
                    path.display()
                )))
            }
        }
    }
    let mut current = Entry::new(open_root()?, PathBuf::from("/"))?;
    let last = names.len().saturating_sub(1);
    for (index, name) in names.iter().enumerate() {
        let is_last = index == last;
        let directory = !is_last || want_dir;
        let child_path = current.path.join(name);
        let file = open_child(&current.file, name, directory).map_err(|e| {
            refuse(format!(
                "{} is not a plain {} reachable without following a symlink ({e}); refusing to \
                 use it as root. Replace the link with the real path and re-run the install",
                child_path.display(),
                if directory { "directory" } else { "file" },
            ))
        })?;
        let child = Entry::new(file, child_path)?;
        check_parent(&current, &child, trusted_uid)?;
        current = child;
    }
    check_final(&current, trusted_uid, want_dir)?;
    Ok(current)
}

/// The directory above `child` must not let an untrusted principal replace or
/// alter `child`.
fn check_parent(parent: &Entry, child: &Entry, trusted_uid: u32) -> Result<()> {
    let bad_owner = !trusted(parent.owner, trusted_uid);
    let writable = parent.mode & WRITABLE_BY_OTHERS != 0;
    let sticky_protects = parent.mode & S_ISVTX != 0 && trusted(child.owner, trusted_uid);
    if bad_owner || (writable && !sticky_protects) {
        let why = if bad_owner {
            "is owned by an untrusted user"
        } else {
            "is writable by group or others"
        };
        return Err(refuse(format!(
            "unsafe directory {} {why}, so another user could replace {} before the service \
             (root) reads it. Make it owned by root or the installing user and not group- or \
             world-writable (e.g. `chmod go-w`); nothing was changed",
            parent.describe(),
            child.path.display()
        )));
    }
    Ok(())
}

fn check_final(entry: &Entry, trusted_uid: u32, want_dir: bool) -> Result<()> {
    let kind = entry.mode & S_IFMT;
    if (want_dir && kind != S_IFDIR) || (!want_dir && kind != S_IFREG) {
        return Err(refuse(format!(
            "{} is not a regular {}",
            entry.describe(),
            if want_dir { "directory" } else { "file" }
        )));
    }
    if !trusted(entry.owner, trusted_uid) || entry.mode & WRITABLE_BY_OTHERS != 0 {
        return Err(refuse(format!(
            "unsafe {}: {} must be owned by root or the installing user (uid {trusted_uid}) and \
             not writable by group or others; nothing was changed. Fix it (e.g. `chmod go-w`) \
             and re-run",
            if want_dir { "directory" } else { "file" },
            entry.describe()
        )));
    }
    Ok(())
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
    walk(path, trusted_uid, true).map(drop)
}

/// Verify `path` and return its text read from the verified handle.
///
/// # Errors
///
/// [`CliError::Permission`] when the path or any parent is unsafe.
pub fn read_verified(path: &Path, trusted_uid: u32) -> Result<String> {
    let mut entry = walk(path, trusted_uid, false)?;
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

/// Atomically publish `contents` as `dir/name` through a freshly verified
/// directory handle: the temp file is created `O_EXCL|O_NOFOLLOW` and renamed
/// with `renameat`, so a link planted by the directory's owner is never
/// followed by root. The directory is re-verified on every call, which closes
/// the swap-after-start race for the status snapshot (SM-04).
///
/// # Errors
///
/// [`CliError::Permission`] when the directory is no longer safe.
pub(crate) fn publish(
    dir: &Path,
    trusted_uid: u32,
    name: &str,
    temp: &str,
    contents: &[u8],
) -> Result<()> {
    use std::io::Write;
    let dir_entry = walk(dir, trusted_uid, true)?;
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
            0o644 as libc::c_uint,
        )
    };
    if raw < 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("cannot create {temp} in {}", dir.display()));
    }
    // SAFETY: `raw` is a freshly opened descriptor nobody else owns.
    let mut file = unsafe { File::from_raw_fd(raw) };
    let written = file
        .write_all(contents)
        .and_then(|()| file.sync_all())
        .with_context(|| format!("failed to write {temp} in {}", dir.display()));
    let renamed = written.and_then(|()| {
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

/// Read `dir/name` through a verified directory handle without following a
/// link; `None` when absent or unreadable.
pub(crate) fn read_in(dir: &Path, trusted_uid: u32, name: &str) -> Option<String> {
    let dir_entry = walk(dir, trusted_uid, true)
        .inspect_err(|e| tracing::debug!("not reading {name}: {e:#}"))
        .ok()?;
    let mut file = open_child(&dir_entry.file, OsStr::new(name), false).ok()?;
    let mut text = String::new();
    file.read_to_string(&mut text).ok()?;
    Some(text)
}

/// Remove `dir/name` through a verified directory handle (never follows a link).
pub(crate) fn remove_in(dir: &Path, trusted_uid: u32, name: &str) -> Result<()> {
    let dir_entry = walk(dir, trusted_uid, true)?;
    let name_c = cstring(name)?;
    // SAFETY: `dir_entry` keeps the verified directory open; `name_c` is
    // NUL-terminated.
    if unsafe { libc::unlinkat(dir_entry.file.as_raw_fd(), name_c.as_ptr(), 0) } == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::NotFound {
        Ok(())
    } else {
        Err(error).with_context(|| format!("failed to remove {name} in {}", dir.display()))
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
        return C::load(path);
    };
    if let Some(state) = std::env::var_os(ENV_STATE_DIR).filter(|d| !d.is_empty()) {
        verify_dir(Path::new(&state), uid)?;
    }
    let text = read_verified(path, uid)?;
    let mut cfg = C::from_toml(&text, path)?;
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
    use std::os::unix::fs::{symlink, PermissionsExt};

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
        let msg = refusal(read_verified(&file, other));
        assert!(
            msg.contains("unsafe") && msg.contains(&format!("uid {}", me())),
            "{msg}"
        );
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
        publish(
            dir,
            me(),
            "serve-status.json",
            "serve-status.json.tmp.1",
            b"one",
        )
        .unwrap();
        publish(
            dir,
            me(),
            "serve-status.json",
            "serve-status.json.tmp.2",
            b"two",
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("serve-status.json")).unwrap(),
            "two"
        );
        assert_eq!(
            read_in(dir, me(), "serve-status.json").as_deref(),
            Some("two")
        );

        // A link planted at the final name is replaced, not written through.
        let victim = tmp.path().canonicalize().unwrap().join("victim");
        std::fs::write(&victim, "keep").unwrap();
        std::fs::remove_file(dir.join("serve-status.json")).unwrap();
        symlink(&victim, dir.join("serve-status.json")).unwrap();
        assert!(read_in(dir, me(), "serve-status.json").is_none());
        publish(
            dir,
            me(),
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
        assert!(publish(dir, me(), "serve-status.json", "t", b"x").is_err());
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");

        // Removal unlinks a link itself, never its target; absent is fine.
        remove_in(dir, me(), "t").unwrap();
        remove_in(dir, me(), "t").unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
    }

    #[test]
    fn publish_refuses_a_directory_that_became_unsafe() {
        let (_tmp, file) = fixture();
        let dir = file.parent().unwrap();
        mode(dir, 0o777);
        assert!(publish(dir, me(), "s", "s.tmp", b"x").is_err());
        assert!(!dir.join("s").exists());
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
