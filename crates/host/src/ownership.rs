//! Role ownership: at most one running process per role (RO-01..RO-03,
//! issue #87).
//!
//! A role (`serve` or `access`) holds kernel advisory locks for its whole
//! lifetime. The kernel drops them when the process dies, so a crash, a
//! `kill -9` or a leftover file can never block the next start, and a reused
//! PID can never inherit ownership (AC-RO-05, AC-RO-06). Lock files are never
//! unlinked: removing a locked inode would let a third process lock a fresh
//! file at the same path.
//!
//! ## Two tiers
//!
//! - **User tier** — `<default state dir>/<role>.lock`. The location comes
//!   from the user's home, never from `--config` or `IROH_TUNNEL_STATE_DIR`,
//!   so foreground and user-service runs of one user share it however they
//!   were started and whichever binary started them.
//! - **Machine tier** — `/run/iroh-tunnel/<role>.lock` (Linux),
//!   `/var/run/iroh-tunnel/<role>.lock` (macOS); not supported on Windows,
//!   which has no service backend. A process that can create and write the
//!   file (the root-owned system service) holds it *exclusively*. Any other
//!   process opens it read-only and holds it *shared*. That refuses a user
//!   start while the system service runs and a system start while any user
//!   process runs, in either start order, while two unrelated users still
//!   coexist. A user process that starts before the file ever existed is not
//!   seen by a later system start; the file appears on the first system run.
//!
//! ## Owner records
//!
//! Lock files carry no data (Windows locks are mandatory, so a locked file
//! cannot be read by another process). The holder publishes
//! `<role>.owner.json` (pid, public node id, start time) next to the lock for
//! messages and `status`. It is informational only: liveness is always the
//! lock, never the record. No secret key is ever written.
//!
//! The process only refuses; it never stops another process (RO-03).

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::error::CliError;
use crate::role::Role;

/// Test-only stand-in for the root-owned machine lock directory.
#[cfg(feature = "test-utils")]
const ENV_MACHINE_LOCK_DIR: &str = "IROH_TUNNEL_MACHINE_LOCK_DIR";

/// Mode bit that lets other users open a lock file for reading.
#[cfg(unix)]
const OTHERS_READ: u32 = 0o004;

/// Who is recorded as running a role.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnerRecord {
    /// Role name (`"serve"` / `"access"`).
    pub role: String,
    /// Process id of the owner.
    pub pid: u32,
    /// Public node id (never the secret key); `None` until the endpoint's
    /// identity is known.
    pub node_id: Option<String>,
    /// Start time of the owner, in epoch seconds.
    pub started_at: u64,
}

/// Where a running owner was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// The per-user lock directory.
    User,
    /// The machine-wide lock directory.
    Machine,
}

/// Result of probing a role for a live owner.
#[derive(Debug)]
pub enum Liveness {
    /// No process holds the role.
    NotRunning,
    /// A process holds the role.
    Running {
        /// Which lock tier is held.
        tier: Tier,
        /// The owner record, when it can be read.
        owner: Option<OwnerRecord>,
    },
}

/// A start refused by a conflict. `CliError::Conflict` carries no prefix of its
/// own because an install can conflict too; the start paths say so here.
fn start_refused(msg: String) -> anyhow::Error {
    CliError::Conflict(format!("start refused: {msg}")).into()
}

/// Test builds on Windows scope the user tier by `HOME` like Unix does, so
/// parallel real-process tests that isolate `HOME` do not share one lock
/// under the real `LocalAppData`.
#[cfg(all(windows, feature = "test-utils"))]
fn user_dir() -> Result<PathBuf> {
    match std::env::var_os("HOME").map(PathBuf::from) {
        Some(home) if home.is_absolute() => Ok(home.join(".local/state/iroh-tunnel")),
        _ => crate::paths::default_state_dir(),
    }
}

#[cfg(not(all(windows, feature = "test-utils")))]
fn user_dir() -> Result<PathBuf> {
    crate::paths::default_state_dir()
}

#[cfg(feature = "test-utils")]
fn machine_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(ENV_MACHINE_LOCK_DIR).filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    // Test builds never touch the real machine-wide location: without an
    // explicit directory it lives beside the user tier, so tests that
    // isolate HOME are isolated from each other and from a real service.
    user_dir().ok().map(|dir| dir.join("machine"))
}

#[cfg(not(feature = "test-utils"))]
fn machine_dir() -> Option<PathBuf> {
    if cfg!(target_os = "linux") {
        Some(PathBuf::from("/run/iroh-tunnel"))
    } else if cfg!(target_os = "macos") {
        Some(PathBuf::from("/var/run/iroh-tunnel"))
    } else {
        None
    }
}

fn lock_name(role: Role) -> String {
    format!("{}.lock", role.name())
}

fn owner_name(role: Role) -> String {
    format!("{}.owner.json", role.name())
}

fn other_role(role: Role) -> Role {
    match role {
        Role::Serve => Role::Access,
        Role::Access => Role::Serve,
    }
}

/// Open options shared by every lock open: never follow a symlink planted at
/// the lock path (RS-11).
fn base_options() -> OpenOptions {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut options = OpenOptions::new();
        options.custom_flags(libc::O_NOFOLLOW);
        options
    }
    #[cfg(not(unix))]
    OpenOptions::new()
}

fn create_dir(dir: &Path, mode: u32) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(mode)
            .create(dir)
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
        std::fs::create_dir_all(dir)
    }
}

fn create_lock_file(path: &Path, mode: u32) -> std::io::Result<File> {
    let mut options = base_options();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    let file = options.open(path)?;
    #[cfg(unix)]
    {
        // The process umask may have narrowed the mode; other users must be
        // able to open a machine lock for reading.
        use std::os::unix::fs::PermissionsExt;
        if mode & OTHERS_READ != 0 {
            if let Err(e) = file.set_permissions(std::fs::Permissions::from_mode(mode)) {
                tracing::debug!("could not widen lock mode for {}: {e}", path.display());
            }
        }
    }
    Ok(file)
}

/// Outcome of trying to take or test one lock.
enum Lock {
    Held(File),
    Busy,
}

/// Take a role lock exclusively. A status probe holds the same lock for
/// microseconds, so a busy result is retried briefly before it counts as a
/// real owner; a genuine owner is still refused within ~75 ms.
fn claim_lock(file: File) -> Result<Lock> {
    const ATTEMPTS: u32 = 4;
    const PAUSE: std::time::Duration = std::time::Duration::from_millis(25);
    for attempt in 1..=ATTEMPTS {
        match file.try_lock() {
            Ok(()) => return Ok(Lock::Held(file)),
            Err(TryLockError::WouldBlock) if attempt < ATTEMPTS => std::thread::sleep(PAUSE),
            Err(TryLockError::WouldBlock) => return Ok(Lock::Busy),
            Err(TryLockError::Error(e)) => return Err(e.into()),
        }
    }
    Ok(Lock::Busy)
}

#[derive(Clone, Copy)]
pub(crate) enum LockKind {
    Exclusive,
    Shared,
}

fn try_lock(file: File, kind: LockKind) -> Result<Lock> {
    let result = match kind {
        LockKind::Exclusive => file.try_lock(),
        LockKind::Shared => file.try_lock_shared(),
    };
    match result {
        Ok(()) => Ok(Lock::Held(file)),
        Err(TryLockError::WouldBlock) => Ok(Lock::Busy),
        Err(TryLockError::Error(e)) => Err(e.into()),
    }
}

fn read_owner(dir: &Path, role: Role) -> Option<OwnerRecord> {
    let body = std::fs::read_to_string(dir.join(owner_name(role))).ok()?;
    serde_json::from_str(&body).ok()
}

fn write_owner(dir: &Path, role: Role, node_id: Option<&str>) {
    let owner = OwnerRecord {
        role: role.name().to_string(),
        pid: std::process::id(),
        node_id: node_id.map(str::to_string),
        started_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    };
    let body = match serde_json::to_string(&owner) {
        Ok(body) => body,
        Err(e) => {
            tracing::debug!("could not encode owner record: {e}");
            return;
        }
    };
    let path = dir.join(owner_name(role));
    // Informational record: a failed write must not stop the role.
    let tmp = dir.join(format!("{}.{}.tmp", owner_name(role), std::process::id()));
    if let Err(e) = std::fs::write(&tmp, body).and_then(|()| std::fs::rename(&tmp, &path)) {
        let _ = std::fs::remove_file(&tmp);
        tracing::debug!("could not publish owner record in {}: {e}", dir.display());
    }
}

/// Human description of a recorded owner: pid, plus public node id when known.
pub fn describe_owner(owner: Option<&OwnerRecord>) -> String {
    match owner {
        Some(OwnerRecord {
            pid,
            node_id: Some(id),
            ..
        }) => format!("pid {pid}, node_id {id}"),
        Some(o) => format!("pid {}", o.pid),
        None => "owner unknown".to_string(),
    }
}

impl Tier {
    /// Who holds the role at this tier, for messages.
    pub fn label(self) -> &'static str {
        match self {
            Tier::User => "this user",
            Tier::Machine => "the system service or another machine-wide process",
        }
    }
}

/// The refusal for a start that finds `role` already running at `scope`.
fn already_running(role: Role, scope: &str, owner: Option<&OwnerRecord>) -> anyhow::Error {
    start_refused(format!(
        "{} is already running ({scope}; {}); {}",
        role.name(),
        describe_owner(owner),
        stop_hint(role)
    ))
}

fn stop_hint(role: Role) -> String {
    stop_hint_for(role, "")
}

/// [`stop_hint`] whose service command carries `scope_flag` (for example
/// `--system`), for a holder in the other service scope. An empty flag adds
/// nothing.
pub(crate) fn stop_hint_scoped(role: Role, scope_flag: &str) -> String {
    stop_hint_for(role, &format!(" {scope_flag}"))
}

fn stop_hint_for(role: Role, flag_suffix: &str) -> String {
    let name = role.name();
    format!(
        "stop it first (Ctrl+C for a foreground run, `iroh-tunnel {name} service stop{flag_suffix}` for a service); \
         nothing is stopped automatically"
    )
}

/// The locks one running role holds. Dropping releases them.
#[derive(Debug)]
pub(crate) struct RoleClaim {
    role: Role,
    user_dir: PathBuf,
    /// Present when this process holds the machine tier exclusively.
    machine_dir: Option<PathBuf>,
    _locks: Vec<File>,
}

/// Claim `role` for this process, refusing while another process runs it.
///
/// # Errors
///
/// [`CliError::Conflict`] when the role already runs (any start mode, any
/// executable); [`CliError::Config`] when the user state directory cannot be
/// resolved; other I/O errors with the failing path.
pub(crate) fn claim(role: Role) -> Result<RoleClaim> {
    let user_dir = user_dir()?;
    create_dir(&user_dir, 0o700)
        .with_context(|| format!("cannot create state directory {}", user_dir.display()))?;
    let user_path = user_dir.join(lock_name(role));
    let file = create_lock_file(&user_path, 0o600)
        .with_context(|| format!("cannot open role lock {}", user_path.display()))?;
    let user_lock = match claim_lock(file)? {
        Lock::Held(file) => file,
        Lock::Busy => {
            return Err(already_running(
                role,
                Tier::User.label(),
                read_owner(&user_dir, role).as_ref(),
            ))
        }
    };

    let mut locks = vec![user_lock];
    let mut held_machine = None;
    if let Some(dir) = machine_dir() {
        match claim_machine(role, &dir)? {
            MachineClaim::Exclusive(file) => {
                locks.push(file);
                held_machine = Some(dir);
            }
            MachineClaim::Shared(file) => locks.push(file),
            MachineClaim::Absent => {}
        }
    }
    // Replace any record a crashed predecessor left, before anyone can read
    // it as ours: its pid may equal the pid in a stale status snapshot.
    write_owner(&user_dir, role, None);
    if let Some(dir) = &held_machine {
        write_owner(dir, role, None);
    }
    Ok(RoleClaim {
        role,
        user_dir,
        machine_dir: held_machine,
        _locks: locks,
    })
}

/// Errors that mean "this account may not create the machine lock".
fn is_not_elevated(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::PermissionDenied
            | std::io::ErrorKind::ReadOnlyFilesystem
            | std::io::ErrorKind::NotFound
    )
}

enum MachineClaim {
    Exclusive(File),
    Shared(File),
    Absent,
}

fn claim_machine(role: Role, dir: &Path) -> Result<MachineClaim> {
    let path = dir.join(lock_name(role));
    // Elevated side: able to create the directory and write the lock. Only
    // "no permission" means "not elevated"; any other failure is real.
    match create_dir(dir, 0o755).and_then(|()| create_lock_file(&path, 0o644)) {
        Ok(file) => {
            return match claim_lock(file)? {
                Lock::Held(file) => Ok(MachineClaim::Exclusive(file)),
                Lock::Busy => Err(already_running(
                    role,
                    Tier::Machine.label(),
                    read_owner(dir, role).as_ref(),
                )),
            };
        }
        Err(e) if is_not_elevated(&e) => {}
        Err(e) => {
            return Err(anyhow::Error::new(e).context(format!(
                "cannot create machine role lock {}",
                path.display()
            )))
        }
    }
    // Non-elevated side: read-only, shared. Never create anything here.
    let file = match base_options().read(true).open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(MachineClaim::Absent),
        Err(e) => {
            return Err(anyhow::Error::new(e)
                .context(format!("cannot open machine role lock {}", path.display())))
        }
    };
    match try_lock(file, LockKind::Shared)? {
        Lock::Held(file) => Ok(MachineClaim::Shared(file)),
        Lock::Busy => Err(already_running(
            role,
            "a system service",
            read_owner(dir, role).as_ref(),
        )),
    }
}

impl Drop for RoleClaim {
    /// Remove this process's owner records on a graceful stop. The lock files
    /// stay (unlinking a lock inode would allow split ownership), and a
    /// record is removed only while it still names this process.
    fn drop(&mut self) {
        let dirs = std::iter::once(&self.user_dir).chain(self.machine_dir.as_ref());
        for dir in dirs {
            if read_owner(dir, self.role).is_some_and(|o| o.pid == std::process::id()) {
                if let Err(e) = std::fs::remove_file(dir.join(owner_name(self.role))) {
                    tracing::debug!("could not remove owner record in {}: {e}", dir.display());
                }
            }
        }
    }
}

impl RoleClaim {
    /// Publish this process as the owner with its public `node_id` and reject
    /// the same public identity already running in the other role (RO-02).
    ///
    /// The record is written *before* the other role is inspected, so two
    /// racing starts with one key can only both back off, never both run.
    ///
    /// # Errors
    ///
    /// [`CliError::Conflict`] when the other role runs with this identity.
    pub(crate) fn publish_identity(&self, node_id: &str) -> Result<()> {
        write_owner(&self.user_dir, self.role, Some(node_id));
        if let Some(dir) = &self.machine_dir {
            write_owner(dir, self.role, Some(node_id));
        }
        let other = other_role(self.role);
        for liveness in probe_tiers(other)? {
            let Liveness::Running {
                owner: Some(owner), ..
            } = liveness
            else {
                continue;
            };
            if owner.node_id.as_deref() == Some(node_id) {
                return Err(start_refused(format!(
                    "the {} role already runs with the same public identity ({}); \
                     two processes must not share a key — use `config keygen` to create another one",
                    other.name(),
                    describe_owner(Some(&owner))
                )));
            }
        }
        Ok(())
    }
}

/// Test whether `role` is running, without taking ownership.
///
/// Never creates or modifies a lock file.
///
/// # Errors
///
/// I/O errors other than "no such lock" when opening or testing a lock.
pub(crate) fn probe(role: Role) -> Result<Liveness> {
    Ok(probe_tiers(role)?
        .into_iter()
        .next()
        .unwrap_or(Liveness::NotRunning))
}

/// Every tier at which `role` is running (user first), empty when none.
fn probe_tiers(role: Role) -> Result<Vec<Liveness>> {
    let mut seen = vec![tier_liveness_in(role, Tier::User, &user_dir()?)?];
    if let Some(dir) = machine_dir() {
        seen.push(tier_liveness_in(role, Tier::Machine, &dir)?);
    }
    seen.retain(|l| matches!(l, Liveness::Running { .. }));
    Ok(seen)
}

/// Both tiers' lock directories in one value.
#[derive(Debug, Clone)]
pub(crate) struct LockDirs {
    /// The user tier's directory.
    pub user: PathBuf,
    /// The machine tier's directory; `None` where the platform defines none.
    pub machine: Option<PathBuf>,
}

impl LockDirs {
    /// Resolve both directories; `home` selects an explicit account home for
    /// the user tier (Unix only), otherwise the invoking user's directory.
    ///
    /// # Errors
    ///
    /// The user state directory cannot be resolved.
    pub(crate) fn resolve(home: Option<&Path>) -> Result<LockDirs> {
        #[cfg(unix)]
        let user = match home {
            Some(home) => crate::paths::state_dir_in(home),
            None => user_dir()?,
        };
        #[cfg(not(unix))]
        let user = {
            let _ = home;
            user_dir()?
        };
        Ok(LockDirs {
            user,
            machine: machine_dir(),
        })
    }
}

/// The seam the install path resolves lock directories through.
pub(crate) type LockResolver<'a> = &'a dyn Fn(Option<&Path>) -> Result<LockDirs>;

/// Probe exactly `dir` for `role`'s lock and report it as `tier`.
///
/// # Errors
///
/// I/O errors other than "no such lock" when opening or testing the lock.
pub(crate) fn tier_liveness_in(role: Role, tier: Tier, dir: &Path) -> Result<Liveness> {
    // A prober that can open the machine lock for writing tests it
    // exclusively, or it would miss shared (non-elevated) holders.
    let prefer_write = matches!(tier, Tier::Machine);
    if lock_is_held(&dir.join(lock_name(role)), prefer_write)? {
        Ok(Liveness::Running {
            tier,
            owner: read_owner(dir, role),
        })
    } else {
        Ok(Liveness::NotRunning)
    }
}

/// Liveness of `role` at one tier of `dirs`.
///
/// The machine tier probes with a write lock, so another account's shared
/// hold there is seen as running: root running `install --user` can be
/// refused by someone else's user process. This mirrors `probe_tiers`.
///
/// # Errors
///
/// As [`tier_liveness_in`].
pub(crate) fn tier_liveness(role: Role, tier: Tier, dirs: &LockDirs) -> Result<Liveness> {
    let dir = match tier {
        Tier::User => Some(&dirs.user),
        Tier::Machine => dirs.machine.as_ref(),
    };
    dir.map_or(Ok(Liveness::NotRunning), |dir| {
        tier_liveness_in(role, tier, dir)
    })
}

/// A lock held by a test at an explicit directory; released on drop.
#[cfg(test)]
pub(crate) struct TestHold {
    _file: File,
}

/// Hold `role`'s lock in `dir` (which must exist) with `kind`.
///
/// # Errors
///
/// The lock file cannot be created, or another holder already has the lock.
#[cfg(test)]
pub(crate) fn hold_for_test(role: Role, dir: &Path, kind: LockKind) -> Result<TestHold> {
    let path = dir.join(lock_name(role));
    let file = create_lock_file(&path, 0o600)?;
    match try_lock(file, kind)? {
        Lock::Held(file) => Ok(TestHold { _file: file }),
        Lock::Busy => anyhow::bail!("lock {} is already held", path.display()),
    }
}

/// Whether another process holds the lock at `path`; a missing lock is not
/// held. The lock is released again before returning.
fn lock_is_held(path: &Path, prefer_write: bool) -> Result<bool> {
    let writable = if prefer_write {
        match open_existing(path, true) {
            Ok(file) => file,
            Err(e) if is_not_elevated(&e) => None,
            Err(e) => {
                return Err(anyhow::Error::new(e)
                    .context(format!("cannot open role lock {}", path.display())))
            }
        }
    } else {
        None
    };
    let (file, kind) = match writable {
        Some(file) => (Some(file), LockKind::Exclusive),
        None => (
            open_existing(path, false)
                .with_context(|| format!("cannot open role lock {}", path.display()))?,
            LockKind::Shared,
        ),
    };
    match file {
        Some(file) => Ok(matches!(try_lock(file, kind)?, Lock::Busy)),
        None => Ok(false),
    }
}

fn open_existing(path: &Path, write: bool) -> std::io::Result<Option<File>> {
    match base_options().read(true).write(write).open(path) {
        Ok(file) => Ok(Some(file)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Refuse when `role` is running; used before `service start`.
///
/// # Errors
///
/// [`CliError::Conflict`] when a process of the role is detected.
pub(crate) fn ensure_not_running(role: Role) -> Result<()> {
    match probe(role)? {
        Liveness::NotRunning => Ok(()),
        Liveness::Running { tier, owner } => {
            Err(already_running(role, tier.label(), owner.as_ref()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refused_start_says_so() {
        let text = already_running(Role::Serve, "this user", None).to_string();
        assert!(text.starts_with("start refused:"), "{text}");
    }

    #[test]
    fn roles_are_each_others_counterpart() {
        assert_eq!(other_role(Role::Serve), Role::Access);
        assert_eq!(other_role(Role::Access), Role::Serve);
    }

    #[test]
    fn lock_and_owner_names_are_role_scoped() {
        assert_eq!(lock_name(Role::Serve), "serve.lock");
        assert_eq!(owner_name(Role::Access), "access.owner.json");
    }

    #[test]
    fn exclusive_lock_is_busy_for_a_second_holder_and_free_after_drop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.lock");
        let first = create_lock_file(&path, 0o600).unwrap();
        let Lock::Held(first) = try_lock(first, LockKind::Exclusive).unwrap() else {
            panic!("first holder must win");
        };
        let second = create_lock_file(&path, 0o600).unwrap();
        assert!(matches!(
            try_lock(second, LockKind::Exclusive).unwrap(),
            Lock::Busy
        ));
        drop(first);
        let third = create_lock_file(&path, 0o600).unwrap();
        assert!(matches!(
            try_lock(third, LockKind::Exclusive).unwrap(),
            Lock::Held(_)
        ));
    }

    #[test]
    fn owner_record_round_trips_without_secret_material() {
        let dir = tempfile::tempdir().unwrap();
        write_owner(dir.path(), Role::Serve, Some("abc"));
        let owner = read_owner(dir.path(), Role::Serve).unwrap();
        assert_eq!(owner.pid, std::process::id());
        assert_eq!(owner.node_id.as_deref(), Some("abc"));
        let raw = std::fs::read_to_string(dir.path().join("serve.owner.json")).unwrap();
        assert!(!raw.contains("secret"), "{raw}");
    }

    #[test]
    #[cfg(unix)]
    fn lock_dirs_resolve_uses_the_given_home() {
        let home = tempfile::tempdir().unwrap();
        let dirs = LockDirs::resolve(Some(home.path())).unwrap();
        assert_eq!(dirs.user, crate::paths::state_dir_in(home.path()));
    }

    #[test]
    fn lock_dirs_resolve_without_home_matches_the_ambient_dirs() {
        let dirs = LockDirs::resolve(None).ok();
        assert_eq!(dirs.as_ref().map(|d| d.user.clone()), user_dir().ok());
        if let Some(dirs) = dirs {
            assert_eq!(dirs.machine, machine_dir());
        }
    }

    #[test]
    fn tier_liveness_in_reports_a_held_lock() {
        let dir = tempfile::tempdir().unwrap();
        let _hold = hold_for_test(Role::Serve, dir.path(), LockKind::Exclusive).unwrap();
        let seen = tier_liveness_in(Role::Serve, Tier::Machine, dir.path()).unwrap();
        assert!(
            matches!(
                seen,
                Liveness::Running {
                    tier: Tier::Machine,
                    ..
                }
            ),
            "{seen:?}"
        );
    }

    #[test]
    fn tier_liveness_in_is_not_running_for_an_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let seen = tier_liveness_in(Role::Serve, Tier::User, dir.path()).unwrap();
        assert!(matches!(seen, Liveness::NotRunning), "{seen:?}");
    }

    #[test]
    fn tier_liveness_in_is_running_even_without_an_owner_record() {
        // read_owner returns None when <role>.owner.json is missing. A held lock
        // must still read as Running, or a conflict check would permit the install.
        let dir = tempfile::tempdir().unwrap();
        let _hold = hold_for_test(Role::Serve, dir.path(), LockKind::Exclusive).unwrap();
        let Liveness::Running { owner, .. } =
            tier_liveness_in(Role::Serve, Tier::Machine, dir.path()).unwrap()
        else {
            panic!("a held lock must read as Running")
        };
        assert!(owner.is_none(), "no record was written");
    }

    #[test]
    fn tier_liveness_in_is_not_running_for_an_existing_but_unlocked_file() {
        let dir = tempfile::tempdir().unwrap();
        create_lock_file(&dir.path().join(lock_name(Role::Serve)), 0o600).unwrap();
        let seen = tier_liveness_in(Role::Serve, Tier::Machine, dir.path()).unwrap();
        assert!(matches!(seen, Liveness::NotRunning), "{seen:?}");
    }

    #[test]
    fn a_shared_hold_is_seen_on_the_machine_tier_only() {
        // probe_tiers prefers a write lock on the machine tier, so an exclusive
        // probe collides with another account's shared hold. Pinned because it
        // means root running `install --user` can be refused by someone else's
        // user process. Not changed here; recorded.
        let dir = tempfile::tempdir().unwrap();
        let _hold = hold_for_test(Role::Serve, dir.path(), LockKind::Shared).unwrap();
        assert!(matches!(
            tier_liveness_in(Role::Serve, Tier::Machine, dir.path()).unwrap(),
            Liveness::Running { .. }
        ));
        assert!(matches!(
            tier_liveness_in(Role::Serve, Tier::User, dir.path()).unwrap(),
            Liveness::NotRunning
        ));
    }
}
