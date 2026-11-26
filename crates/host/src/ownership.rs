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
//! The tier is a fact of the process ([`Tier::current`]): an elevated process
//! is [`Tier::System`], any other is [`Tier::User`]. A process claims only its
//! own tier, and the two tiers run side by side.
//!
//! - **User tier** — `<default state dir>/<role>.lock`, held exclusively. The
//!   location comes from the user's home, never from `--config` or
//!   `IROH_TUNNEL_STATE_DIR`, so foreground and user-service runs of one user
//!   share it however they were started.
//! - **System tier** — `<machine dir>/<role>.claim.lock`, held exclusively:
//!   one per machine per role. It is private to root (Unix `0o600`) or to
//!   SYSTEM and Administrators (Windows), so no other account can open it
//!   and block a system start. The system process also holds
//!   `<machine dir>/<role>.lock` (`0o644`) *shared*, only so the user tier can
//!   detect it. The machine dir is `/run/iroh-tunnel` (Linux),
//!   `/var/run/iroh-tunnel` (macOS; the macOS state dir is separate, under
//!   `/Library`), `%ProgramData%\iroh-tunnel` (Windows).
//! - **Presence file** — `<machine dir>/<role>.user.lock`. The system tier
//!   creates it (0644, unlocked); a user process holds it *shared* through a
//!   read-only open while it runs and never creates anything in the machine
//!   dir. These files only detect the other tier ([`other_tier_running`]); they
//!   never block a start. A user process that started before the file existed
//!   is not seen by a later system start (accepted blind spot).
//!
//! ## Owner records
//!
//! Lock files carry no data (Windows locks are mandatory, so a locked file
//! cannot be read by another process). The holder publishes
//! `<role>.owner.json` (pid, public node id, start time) next to the lock for
//! messages, `status` and the identity check. Publishing is mandatory: a role
//! that cannot write its record does not start. Liveness is always the lock,
//! never the record. The identity check fails closed: while the other role's
//! lock is held exclusively, a record that cannot be read or parsed refuses
//! the start, because the other role may run with the same key. A record
//! whose lock is free (a crash leftover) is never read. The identity check
//! looks only at the claimant's own tier. User-tier records are private
//! (`0o600`); system-tier records are world-readable (`0o644`). No secret key
//! is ever written.
//!
//! The process only refuses; it never stops another process (RO-03).

mod legacy;

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::error::CliError;
use crate::role::Role;
use crate::Tier;

/// Test-only stand-in for the root-owned machine lock directory.
///
/// Test-utils locations are never trust-verified (the harness points this at
/// a user-owned temp dir), unless `IROH_TUNNEL_TEST_VERIFY_MACHINE_DIR=1`
/// (see [`ENV_TEST_VERIFY_MACHINE_DIR`]) is also set.
#[cfg(feature = "test-utils")]
const ENV_MACHINE_LOCK_DIR: &str = "IROH_TUNNEL_MACHINE_LOCK_DIR";

/// Test-only switch: `1` makes test-utils machine directories go through the
/// Windows machine trust check like the real `%ProgramData%` location.
#[cfg(feature = "test-utils")]
const ENV_TEST_VERIFY_MACHINE_DIR: &str = "IROH_TUNNEL_TEST_VERIFY_MACHINE_DIR";

/// The machine-wide lock directory and whether it must be trust-verified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MachineDir {
    pub(crate) path: PathBuf,
    /// Windows only: the directory must pass the machine trust check.
    pub(crate) verify: bool,
}

/// Mode of the machine-dir detection lock and presence files: every account
/// must be able to open them for reading.
const MACHINE_FILE_MODE: u32 = 0o644;

/// Mode of the system tier's claim lock: root only, so no other account can
/// hold it and block a system start.
const CLAIM_FILE_MODE: u32 = 0o600;

/// SDDL of the Windows claim lock: owner Administrators, protected DACL with
/// full control for SYSTEM and Administrators only.
#[cfg(windows)]
const CLAIM_FILE_SDDL: &str = "O:BAG:SYD:P(A;;FA;;;SY)(A;;FA;;;BA)";

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

/// Result of probing a role for a live owner.
#[derive(Debug)]
pub enum Liveness {
    /// No process holds the role.
    NotRunning,
    /// A process holds the role.
    Running {
        /// Which tier's lock is held.
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
        _ => crate::paths::default_state_dir_for(Tier::User),
    }
}

#[cfg(not(all(windows, feature = "test-utils")))]
fn user_dir() -> Result<PathBuf> {
    crate::paths::default_state_dir_for(Tier::User)
}

/// The machine lock directory; `Ok(None)` only where the platform defines
/// no machine tier.
///
/// # Errors
///
/// The test-utils fallback beside the user tier cannot be resolved.
#[cfg(feature = "test-utils")]
pub(crate) fn machine_dir() -> Result<Option<MachineDir>> {
    let verify = std::env::var_os(ENV_TEST_VERIFY_MACHINE_DIR).is_some_and(|v| v == "1");
    if let Some(dir) = std::env::var_os(ENV_MACHINE_LOCK_DIR).filter(|d| !d.is_empty()) {
        return Ok(Some(MachineDir {
            path: PathBuf::from(dir),
            verify,
        }));
    }
    // Test builds never touch the real machine-wide location: without an
    // explicit directory it lives beside the user tier, so tests that
    // isolate HOME are isolated from each other and from a real service.
    Ok(Some(MachineDir {
        path: user_dir()?.join("machine"),
        verify,
    }))
}

/// The Windows machine directory under the resolved `%ProgramData%` folder.
/// Compiled on every target so the fail-closed mapping is testable; only
/// Windows production builds call it.
#[cfg(any(test, all(windows, not(feature = "test-utils"))))]
fn machine_dir_from_program_data(program_data: Result<PathBuf>) -> Result<Option<MachineDir>> {
    let dir = program_data.context("cannot resolve the machine lock directory (%ProgramData%)")?;
    Ok(Some(MachineDir {
        path: dir.join("iroh-tunnel"),
        verify: true,
    }))
}

/// The machine lock directory under `%ProgramData%`.
///
/// # Errors
///
/// `%ProgramData%` cannot be resolved: ownership must fail closed rather than
/// silently skip the machine tier.
#[cfg(all(windows, not(feature = "test-utils")))]
pub(crate) fn machine_dir() -> Result<Option<MachineDir>> {
    machine_dir_from_program_data(crate::service::program_data_dir())
}

/// The platform's machine lock directory; `None` where it defines none.
/// Compiled for tests on every non-Windows target so the real paths are
/// asserted even when `test-utils` replaces the resolver.
#[cfg(all(not(windows), any(test, not(feature = "test-utils"))))]
fn platform_machine_dir() -> Option<MachineDir> {
    let path = if cfg!(target_os = "linux") {
        PathBuf::from("/run/iroh-tunnel")
    } else if cfg!(target_os = "macos") {
        PathBuf::from("/var/run/iroh-tunnel")
    } else {
        return None;
    };
    Some(MachineDir {
        path,
        verify: false,
    })
}

/// The machine lock directory; `Ok(None)` where the platform defines none.
///
/// # Errors
///
/// Never on these platforms; the `Result` matches the other variants.
#[cfg(all(not(windows), not(feature = "test-utils")))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "signature shared with the Windows and test-utils variants, which can fail"
)]
pub(crate) fn machine_dir() -> Result<Option<MachineDir>> {
    Ok(platform_machine_dir())
}

/// The directories one claim or probe needs, resolved before any lock is
/// taken so a resolution error never leaves a half-taken claim.
#[derive(Debug)]
struct TierDirs {
    /// The user tier's directory; `None` when the tier does not need it.
    user: Option<PathBuf>,
    /// The machine directory; `None` where the platform defines none or
    /// (user tier only) it could not be resolved.
    machine: Option<MachineDir>,
}

impl TierDirs {
    /// Resolve the directories `tier` needs with the platform resolver.
    ///
    /// # Errors
    ///
    /// As [`TierDirs::resolve_for_with`].
    fn resolve_for(tier: Tier) -> Result<TierDirs> {
        Self::resolve_for_with(tier, machine_dir)
    }

    /// Resolve the directories `tier` needs, the machine one through
    /// `machine`. The system tier requires it; the user tier only uses it to
    /// notice the presence file, so a failure there is logged and ignored.
    ///
    /// # Errors
    ///
    /// System tier: the machine directory cannot be resolved. User tier: the
    /// user state directory cannot be resolved.
    fn resolve_for_with(
        tier: Tier,
        machine: impl FnOnce() -> Result<Option<MachineDir>>,
    ) -> Result<TierDirs> {
        match tier {
            Tier::System => Ok(TierDirs {
                user: None,
                machine: machine()?,
            }),
            Tier::User => {
                let machine = machine().unwrap_or_else(|e| {
                    tracing::debug!("machine lock directory unavailable: {e:#}");
                    None
                });
                Ok(TierDirs {
                    user: Some(user_dir()?),
                    machine,
                })
            }
        }
    }

    /// The directory holding `tier`'s own lock and owner record.
    ///
    /// # Errors
    ///
    /// The tier has no directory here.
    fn own_dir(&self, tier: Tier) -> Result<&Path> {
        let dir = match tier {
            Tier::User => self.user.as_deref(),
            Tier::System => self.machine.as_ref().map(|m| m.path.as_path()),
        };
        dir.with_context(|| format!("no lock directory for the {}", tier.label()))
    }
}

fn lock_name(role: Role) -> String {
    format!("{}.lock", role.name())
}

/// The system tier's exclusive, admin-only claim lock.
fn claim_lock_name(role: Role) -> String {
    format!("{}.claim.lock", role.name())
}

/// The lock that says `role` runs in `tier`: the user lock, or the system
/// tier's claim lock.
fn own_lock_name(role: Role, tier: Tier) -> String {
    match tier {
        Tier::User => lock_name(role),
        Tier::System => claim_lock_name(role),
    }
}

/// The presence file user processes hold shared while they run.
fn presence_name(role: Role) -> String {
    format!("{}.user.lock", role.name())
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

/// Create `dir` and any missing parents, each created directory getting
/// exactly `mode` whatever the umask. A directory that already exists is left
/// untouched (never re-chmodded).
pub(crate) fn create_dir(dir: &Path, mode: u32) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut missing: Vec<&Path> = dir
            .ancestors()
            .take_while(|p| !p.as_os_str().is_empty() && std::fs::symlink_metadata(p).is_err())
            .collect();
        missing.reverse();
        for path in missing {
            match std::fs::DirBuilder::new().mode(mode).create(path) {
                Ok(()) => finalize_dir_mode(path, mode)?,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
        }
        match std::fs::symlink_metadata(dir) {
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
        std::fs::create_dir_all(dir)
    }
}

/// Set `mode` on a directory this process just created. The directory is
/// opened `O_DIRECTORY | O_NOFOLLOW` and checked through the handle (a
/// directory owned by the current euid) before `fchmod`, so a path swapped for
/// a symlink or a foreign object after `mkdir` is refused, never chmodded.
#[cfg(unix)]
fn finalize_dir_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    let dir = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)?;
    let meta = dir.metadata()?;
    // SAFETY: geteuid has no preconditions and cannot fail.
    if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "{} is not a directory created by this process",
                path.display()
            ),
        ));
    }
    dir.set_permissions(std::fs::Permissions::from_mode(mode))
}

/// SDDL of the machine directory (D7): owner and group are Administrators and
/// SYSTEM, the DACL is protected, SYSTEM and Administrators have full control
/// and Users may only read.
#[cfg(windows)]
const MACHINE_DIR_SDDL: &str =
    "O:BAG:SYD:PAI(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;BU)";

/// Create the machine directory. Windows creates it with the protected DACL;
/// the parent (`%ProgramData%`) already exists.
fn create_machine_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        crate::service::create_dir_with_sddl(dir, MACHINE_DIR_SDDL)
    }
    #[cfg(not(windows))]
    create_dir(dir, 0o755)
}

/// Create the machine directory when it is missing (with its protected
/// descriptor), or trust-verify an existing one. The Windows service calls
/// this before it opens its log, which lives in this directory, so the first
/// start on a host logs too.
///
/// # Errors
///
/// [`CliError::Permission`] for an existing untrusted directory; the create
/// error otherwise.
///
/// Only a directory that must be verified (the real `%ProgramData%` location)
/// is created here; a test-utils directory is left to its owner.
#[cfg(windows)]
pub(crate) fn ensure_machine_dir(machine: &MachineDir) -> Result<()> {
    if !machine.verify {
        return Ok(());
    }
    match create_machine_dir(&machine.path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            crate::service::verify_machine_dir(&machine.path)
        }
        Err(e) => Err(anyhow::Error::new(e).context(format!(
            "cannot create machine directory {}",
            machine.path.display()
        ))),
    }
}

/// Open `path` as a lock file, creating it when missing. A file this call
/// creates gets exactly `mode` (Unix; set on the open file, so the umask cannot
/// narrow it); an existing file keeps its permissions.
fn create_lock_file(path: &Path, mode: u32) -> std::io::Result<File> {
    let mut create = base_options();
    create.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        create.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    match create.open(path) {
        Ok(file) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(std::fs::Permissions::from_mode(mode))?;
            }
            Ok(file)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            base_options().read(true).write(true).open(path)
        }
        Err(e) => Err(e),
    }
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

/// What reading an owner record found.
#[derive(Debug)]
enum OwnerRead {
    /// No record exists.
    Absent,
    /// A well-formed record.
    Present(OwnerRecord),
    /// A record exists but cannot be read or parsed.
    Unreadable { path: PathBuf, error: anyhow::Error },
}

/// Read `role`'s owner record in `dir`, telling a missing record apart from
/// one that cannot be read or parsed.
fn read_owner_record(dir: &Path, role: Role) -> OwnerRead {
    let path = dir.join(owner_name(role));
    let body = match std::fs::read_to_string(&path) {
        Ok(body) => body,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return OwnerRead::Absent,
        Err(e) => {
            return OwnerRead::Unreadable {
                error: anyhow::Error::new(e).context("cannot read the owner record"),
                path,
            }
        }
    };
    match serde_json::from_str(&body) {
        Ok(owner) => OwnerRead::Present(owner),
        Err(e) => OwnerRead::Unreadable {
            error: anyhow::Error::new(e).context("malformed owner record"),
            path,
        },
    }
}

/// The informational view of an owner record (messages, `status`, `Drop`).
fn read_owner(dir: &Path, role: Role) -> Option<OwnerRecord> {
    match read_owner_record(dir, role) {
        OwnerRead::Present(owner) => Some(owner),
        OwnerRead::Absent | OwnerRead::Unreadable { .. } => None,
    }
}

/// Whether some process holds the lock at `path` exclusively; a missing lock
/// file is not held. Tested with a shared lock on a read-only open, which is
/// released again before returning.
///
/// # Errors
///
/// The lock exists but cannot be opened or tested.
fn held_exclusively(path: &Path) -> Result<bool> {
    let Some(file) = open_existing(path, false)
        .with_context(|| format!("cannot open role lock {}", path.display()))?
    else {
        return Ok(false);
    };
    let lock = try_lock(file, LockKind::Shared)
        .with_context(|| format!("cannot test role lock {}", path.display()))?;
    Ok(matches!(lock, Lock::Busy))
}

/// Whether some process holds the lock at `path` in any mode; a missing file
/// is not held. Tested with an exclusive lock on a read-only open, released
/// again before returning.
///
/// # Errors
///
/// The lock exists but cannot be opened or tested.
fn held_shared(path: &Path) -> Result<bool> {
    let Some(file) = open_existing(path, false)
        .with_context(|| format!("cannot open lock {}", path.display()))?
    else {
        return Ok(false);
    };
    let lock = try_lock(file, LockKind::Exclusive)
        .with_context(|| format!("cannot test lock {}", path.display()))?;
    Ok(matches!(lock, Lock::Busy))
}

/// Mode of an owner record in the user tier: private to the account.
const USER_RECORD_MODE: u32 = 0o600;
/// Mode of an owner record in the machine tier: every account must be able to
/// read the system holder's identity.
const MACHINE_RECORD_MODE: u32 = 0o644;

/// Publish this process as `role`'s owner in `dir` with file `mode` (Unix;
/// applied on the open file, so the umask cannot narrow it).
///
/// # Errors
///
/// The record cannot be encoded, written or renamed into place; the error
/// names the record path.
fn write_owner(dir: &Path, role: Role, node_id: Option<&str>, mode: u32) -> Result<()> {
    let owner = OwnerRecord {
        role: role.name().to_string(),
        pid: std::process::id(),
        node_id: node_id.map(str::to_string),
        started_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    };
    let path = dir.join(owner_name(role));
    let tmp = dir.join(format!("{}.{}.tmp", owner_name(role), std::process::id()));
    let published = serde_json::to_vec(&owner)
        .map_err(anyhow::Error::from)
        .and_then(|body| {
            write_record_file(&tmp, &body, mode)?;
            std::fs::rename(&tmp, &path)?;
            Ok(())
        });
    if let Err(e) = published {
        match std::fs::remove_file(&tmp) {
            Ok(()) => {}
            Err(rm) if rm.kind() == std::io::ErrorKind::NotFound => {}
            Err(rm) => tracing::debug!("could not remove {}: {rm}", tmp.display()),
        }
        return Err(e.context(format!("cannot publish owner record {}", path.display())));
    }
    Ok(())
}

/// Write `body` to `path` with `mode`. Never `create_new`: a temp file left by
/// a killed process whose pid was reused must not block publishing.
fn write_record_file(path: &Path, body: &[u8], mode: u32) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = base_options();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    let mut file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(mode))?;
    }
    file.write_all(body)
}

/// Human description of a recorded owner: pid, plus public node id when known.
#[must_use]
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

/// The refusal for a start that finds `role` already running in `tier`.
fn already_running(role: Role, tier: Tier, owner: Option<&OwnerRecord>) -> anyhow::Error {
    start_refused(format!(
        "{} is already running ({}; {}); {}",
        role.name(),
        tier.label(),
        describe_owner(owner),
        stop_hint(role)
    ))
}

fn stop_hint(role: Role) -> String {
    let name = role.name();
    format!(
        "stop it first (Ctrl+C for a foreground run, `iroh-tunnel {name} service stop` for a service); \
         nothing is stopped automatically"
    )
}

/// The locks one running role holds. Dropping releases them.
#[derive(Debug)]
pub(crate) struct RoleClaim {
    role: Role,
    tier: Tier,
    /// The directories this claim was taken in.
    dirs: TierDirs,
    /// The own-tier lock, plus the shared presence file for a user claim
    /// when it exists.
    _locks: Vec<File>,
}

/// Claim `role` in the current tier, refusing while another process runs it
/// there. The other tier is never consulted (see [`other_tier_running`]).
///
/// # Errors
///
/// [`CliError::Conflict`] when the role already runs in this tier (any start
/// mode, any executable); [`CliError::Config`] when the user state directory
/// cannot be resolved (user tier); the machine lock directory cannot be
/// resolved or created (system tier, checked before any lock is taken); other
/// I/O errors with the failing path.
pub(crate) fn claim(role: Role) -> Result<RoleClaim> {
    let tier = Tier::current();
    claim_in(role, tier, TierDirs::resolve_for(tier)?)
}

/// Claim a configured action's role in its explicit private user scope,
/// independently of the process's elevation and machine-wide ownership.
pub(crate) fn claim_action(role: Role, scope: PathBuf) -> Result<RoleClaim> {
    if !scope.is_absolute() {
        return Err(CliError::Config(
            "IROH_TUNNEL_ACTION_SCOPE must be an absolute directory path".into(),
        )
        .into());
    }
    claim_in(
        role,
        Tier::User,
        TierDirs {
            user: Some(scope),
            machine: None,
        },
    )
}

/// [`claim`] for `tier` in already resolved directories.
///
/// # Errors
///
/// As [`claim`], except resolution.
fn claim_in(role: Role, tier: Tier, dirs: TierDirs) -> Result<RoleClaim> {
    let locks = match tier {
        Tier::User => claim_user(role, &dirs)?,
        Tier::System => claim_system(role, &dirs)?,
    };
    let claim = RoleClaim {
        role,
        tier,
        dirs,
        _locks: locks,
    };
    // Replace any record a crashed predecessor left, before anyone can read
    // it as ours: its pid may equal the pid in a stale status snapshot.
    // A failure drops `claim`, which releases the locks.
    let (dir, mode) = claim.record_dir()?;
    write_owner(dir, role, None, mode)?;
    Ok(claim)
}

/// Take the user tier: its own lock exclusively, then the system tier's
/// presence file shared when it exists. Never creates a machine-dir file.
fn claim_user(role: Role, dirs: &TierDirs) -> Result<Vec<File>> {
    let user_dir = dirs.own_dir(Tier::User)?;
    create_dir(user_dir, 0o700)
        .with_context(|| format!("cannot create state directory {}", user_dir.display()))?;
    let user_path = user_dir.join(lock_name(role));
    let file = create_lock_file(&user_path, 0o600)
        .with_context(|| format!("cannot open role lock {}", user_path.display()))?;
    let user_lock = match claim_lock(file)? {
        Lock::Held(file) => file,
        Lock::Busy => {
            return Err(already_running(
                role,
                Tier::User,
                read_owner(user_dir, role).as_ref(),
            ))
        }
    };
    let mut locks = vec![user_lock];
    if let Some(machine) = &dirs.machine {
        locks.extend(hold_presence(&machine.path.join(presence_name(role))));
    }
    Ok(locks)
}

/// Hold the presence file shared through a read-only open. Best effort: it
/// only lets the system tier notice this process, so every failure (absent
/// file, no permission, a probe holding it) is logged and the start goes on.
fn hold_presence(path: &Path) -> Option<File> {
    let file = match open_existing(path, false) {
        Ok(Some(file)) => file,
        Ok(None) => return None,
        Err(e) => {
            tracing::debug!("cannot open presence file {}: {e}", path.display());
            return None;
        }
    };
    hold_shared(file, path)
}

/// Hold `file` (opened at `path`) shared, best effort: the other tier's probe
/// holds it exclusively for microseconds, so a busy lock is retried briefly;
/// a lasting holder or an error is logged and yields `None`.
fn hold_shared(file: File, path: &Path) -> Option<File> {
    const ATTEMPTS: u32 = 4;
    const PAUSE: std::time::Duration = std::time::Duration::from_millis(25);
    for attempt in 1..=ATTEMPTS {
        match file.try_lock_shared() {
            Ok(()) => return Some(file),
            Err(TryLockError::WouldBlock) if attempt < ATTEMPTS => std::thread::sleep(PAUSE),
            Err(TryLockError::WouldBlock) => {
                tracing::debug!("{} stays locked; holding it skipped", path.display());
            }
            Err(TryLockError::Error(e)) => {
                tracing::debug!("cannot lock {}: {e}", path.display());
                return None;
            }
        }
    }
    None
}

/// The refusal for a start that finds a system instance of an earlier version.
fn earlier_version_running(role: Role, owner: &OwnerRecord) -> anyhow::Error {
    start_refused(format!(
        "{} is already running (a system instance from an earlier version, {}); {}",
        role.name(),
        describe_owner(Some(owner)),
        stop_hint(role)
    ))
}

/// Take the system tier: the claim lock exclusively and the detection lock
/// shared, then make sure the presence file exists.
fn claim_system(role: Role, dirs: &TierDirs) -> Result<Vec<File>> {
    let machine = dirs
        .machine
        .as_ref()
        .context("this platform defines no machine lock directory")?;
    let locks = claim_system_lock(role, &machine.path, machine.verify)?;
    ensure_presence_in(role, &machine.path)?;
    Ok(locks)
}

/// Create the machine directory (verifying an existing one where `verify`),
/// take `<role>.claim.lock` there exclusively, then hold `<role>.lock` shared
/// for the user tier to detect. Only the claim lock can refuse: it is
/// admin-only, while anyone may hold the world-readable detection lock, which
/// is then skipped (at worst the user tier misses the notice).
///
/// `verify` (Windows only) requires an existing directory to pass the machine
/// trust check before anything is created in it; a refusal is a permission
/// error and the directory is never repaired.
///
/// # Errors
///
/// [`CliError::Conflict`] when the claim lock is held;
/// [`CliError::Permission`] for an untrusted directory; the create/open error
/// with the path otherwise.
fn claim_system_lock(role: Role, dir: &Path, verify: bool) -> Result<Vec<File>> {
    let path = dir.join(claim_lock_name(role));
    #[cfg(windows)]
    let dir_exists = std::fs::symlink_metadata(dir).is_ok();
    #[cfg(windows)]
    if dir_exists && verify {
        crate::service::verify_machine_dir(dir)?;
    }
    #[cfg(not(windows))]
    let _ = verify;
    // Windows creates only a missing directory (`CreateDirectoryW` fails on an
    // existing one); Unix keeps its idempotent create.
    // An unverified (test-utils) directory may sit under missing ancestors,
    // which Unix creates too; the protected `%ProgramData%` one never does.
    #[cfg(windows)]
    let ensure_dir = if dir_exists {
        Ok(())
    } else if verify {
        create_machine_dir(dir)
    } else {
        std::fs::create_dir_all(dir)
    };
    #[cfg(not(windows))]
    let ensure_dir = create_machine_dir(dir);
    // Another process may have created the directory since the check above:
    // trust it only after verification.
    #[cfg(windows)]
    let ensure_dir = match ensure_dir {
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            if verify {
                crate::service::verify_machine_dir(dir)?;
            }
            Ok(())
        }
        other => other,
    };
    let file = ensure_dir
        .and_then(|()| create_claim_file(&path, verify))
        .with_context(|| format!("cannot create machine role lock {}", path.display()))?;
    let claim = match claim_lock(file)? {
        Lock::Held(file) => file,
        Lock::Busy => {
            return Err(already_running(
                role,
                Tier::System,
                read_owner(dir, role).as_ref(),
            ))
        }
    };
    // An earlier version's system process holds `<role>.lock` exclusively and
    // has no claim lock, so the claim lock above cannot see it.
    if let Some(owner) = legacy::earlier_version_holder(dir, role)? {
        drop(claim);
        return Err(earlier_version_running(role, &owner));
    }
    let detect_path = dir.join(lock_name(role));
    let detect = create_lock_file(&detect_path, MACHINE_FILE_MODE)
        .with_context(|| format!("cannot create machine role lock {}", detect_path.display()))?;
    let mut locks = vec![claim];
    locks.extend(hold_shared(detect, &detect_path));
    Ok(locks)
}

/// Open the claim lock at `path`, creating it admin-only when missing: Unix
/// `0o600`; Windows (where `verify` marks the real `%ProgramData%` location)
/// with [`CLAIM_FILE_SDDL`]. An existing file keeps its permissions.
fn create_claim_file(path: &Path, verify: bool) -> std::io::Result<File> {
    #[cfg(windows)]
    if verify {
        return match crate::service::create_file_with_sddl(path, CLAIM_FILE_SDDL) {
            Ok(file) => Ok(file),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                base_options().read(true).write(true).open(path)
            }
            Err(e) => Err(e),
        };
    }
    let _ = verify;
    create_lock_file(path, CLAIM_FILE_MODE)
}

/// Create `<role>.user.lock` in `dir` when missing, readable by everyone and
/// not locked.
fn ensure_presence_in(role: Role, dir: &Path) -> Result<()> {
    let path = dir.join(presence_name(role));
    create_lock_file(&path, MACHINE_FILE_MODE)
        .map(drop)
        .with_context(|| format!("cannot create presence file {}", path.display()))
}

/// Make sure the system tier's presence file of `role` exists (created when
/// missing, never locked), so user instances started afterwards are visible
/// to the system tier. Called by a system `run` claim and by a system
/// `service install`.
///
/// # Errors
///
/// The machine directory cannot be resolved, created or trusted, or the file
/// cannot be created.
pub(crate) fn ensure_presence_file(role: Role) -> Result<()> {
    let machine = machine_dir()?.context("this platform defines no machine lock directory")?;
    #[cfg(windows)]
    ensure_machine_dir(&machine)?;
    #[cfg(not(windows))]
    create_machine_dir(&machine.path)
        .with_context(|| format!("cannot create machine directory {}", machine.path.display()))?;
    ensure_presence_in(role, &machine.path)
}

impl Drop for RoleClaim {
    /// Remove this process's owner record on a graceful stop. The lock files
    /// stay (unlinking a lock inode would allow split ownership), and a
    /// record is removed only while it still names this process.
    fn drop(&mut self) {
        if let Ok((dir, _)) = self.record_dir() {
            if read_owner(dir, self.role).is_some_and(|o| o.pid == std::process::id()) {
                if let Err(e) = std::fs::remove_file(dir.join(owner_name(self.role))) {
                    tracing::debug!("could not remove owner record in {}: {e}", dir.display());
                }
            }
        }
    }
}

impl RoleClaim {
    /// The directory this process publishes its owner record in (its own
    /// tier's) with the record's file mode.
    fn record_dir(&self) -> Result<(&Path, u32)> {
        let mode = match self.tier {
            Tier::User => USER_RECORD_MODE,
            Tier::System => MACHINE_RECORD_MODE,
        };
        Ok((self.dirs.own_dir(self.tier)?, mode))
    }

    /// Publish this process as the owner with its public `node_id` and reject
    /// the same public identity already running in the other role (RO-02).
    ///
    /// The record is written *before* the other role is inspected, so two
    /// racing starts with one key can only both back off, never both run.
    /// Only this tier's directory is inspected, and only while the other
    /// role's lock there is held exclusively, so a crash leftover or the other
    /// tier's record never blocks a start.
    ///
    /// # Errors
    ///
    /// The record cannot be published; [`CliError::Conflict`] when the other
    /// role runs with this identity or runs with an unreadable record.
    pub(crate) fn publish_identity(&self, node_id: &str) -> Result<()> {
        let (dir, mode) = self.record_dir()?;
        write_owner(dir, self.role, Some(node_id), mode)?;
        let other = other_role(self.role);
        if let Some(owner) = running_owner(dir, other, self.tier)? {
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

/// The owner record of `role` running in `tier`'s `dir`: `None` when its own
/// lock there is not held exclusively, or the holder has published no record or no
/// identity yet.
///
/// An unreadable record is retried briefly (a reader racing a Windows rename,
/// a writer mid-publish, a status probe's short exclusive hold) and then
/// refused: the holder's identity cannot be determined, so it must not be
/// assumed different.
///
/// # Errors
///
/// [`CliError::Conflict`] when the record stays unreadable while the lock is
/// held; lock I/O errors.
fn running_owner(dir: &Path, role: Role, tier: Tier) -> Result<Option<OwnerRecord>> {
    const ATTEMPTS: u32 = 4;
    const PAUSE: std::time::Duration = std::time::Duration::from_millis(25);
    let lock = dir.join(own_lock_name(role, tier));
    let mut attempt = 1;
    loop {
        if !held_exclusively(&lock)? {
            return Ok(None);
        }
        match read_owner_record(dir, role) {
            OwnerRead::Absent => return Ok(None),
            OwnerRead::Present(owner) => return Ok(Some(owner)),
            OwnerRead::Unreadable { .. } if attempt < ATTEMPTS => {
                attempt += 1;
                // Blocks at most ~75 ms, only on the unreadable-record path and
                // once per start, like the surrounding synchronous file I/O.
                std::thread::sleep(PAUSE);
            }
            OwnerRead::Unreadable { path, error } => {
                return Err(start_refused(format!(
                    "the {} role is running but its identity cannot be determined ({}: {error:#}); {}",
                    role.name(),
                    path.display(),
                    stop_hint(role)
                )));
            }
        }
    }
}

/// Test whether `role` is running in the current tier, without taking
/// ownership.
///
/// Never creates or modifies a lock file.
///
/// # Errors
///
/// The tier's directory cannot be resolved; I/O errors other than "no such
/// lock" when opening or testing the lock.
pub(crate) fn probe(role: Role) -> Result<Liveness> {
    probe_tier(role, Tier::current())
}

/// Test whether `role` is running in `tier`, without taking ownership.
///
/// # Errors
///
/// As [`probe`].
pub(crate) fn probe_tier(role: Role, tier: Tier) -> Result<Liveness> {
    let dirs = TierDirs::resolve_for(tier)?;
    tier_liveness_in(role, tier, dirs.own_dir(tier)?)
}

/// Whether `role` also runs in the tier other than `tier`, as seen from
/// `tier`: a user process tests the system tier's detection lock, a system
/// process tests the presence file, each with an exclusive try-lock. Only a lock is read, never an owner record. Any I/O error
/// is logged at debug level and reads as "not running", so the notice can
/// never block a start.
#[must_use]
pub fn other_tier_running(role: Role, tier: Tier) -> Option<Tier> {
    let machine = match machine_dir() {
        Ok(Some(machine)) => machine,
        Ok(None) => return None,
        Err(e) => {
            tracing::debug!("cannot resolve the machine lock directory: {e:#}");
            return None;
        }
    };
    other_tier_running_in(role, tier, &machine.path)
}

/// [`other_tier_running`] in an explicit machine directory.
fn other_tier_running_in(role: Role, tier: Tier, machine: &Path) -> Option<Tier> {
    let (name, other) = match tier {
        Tier::User => (lock_name(role), Tier::System),
        Tier::System => (presence_name(role), Tier::User),
    };
    let path = machine.join(name);
    // Both are held shared by the other tier, so an exclusive test sees them.
    // A non-admin may hold the detection lock too: at worst a false notice.
    match held_shared(&path) {
        Ok(true) => Some(other),
        Ok(false) => None,
        Err(e) => {
            tracing::debug!("cannot probe the {}: {e:#}", other.label());
            None
        }
    }
}

/// Probe exactly `dir` for `role`'s own lock in `tier` (the user lock, or
/// the system tier's claim lock, which only an elevated process can open) and
/// report it as `tier`.
///
/// # Errors
///
/// I/O errors other than "no such lock" when opening or testing the lock.
pub(crate) fn tier_liveness_in(role: Role, tier: Tier, dir: &Path) -> Result<Liveness> {
    if held_exclusively(&dir.join(own_lock_name(role, tier)))? {
        return Ok(Liveness::Running {
            tier,
            owner: read_owner(dir, role),
        });
    }
    if tier == Tier::System {
        if let Some(owner) = legacy::earlier_version_holder(dir, role)? {
            return Ok(Liveness::Running {
                tier,
                owner: Some(owner),
            });
        }
    }
    Ok(Liveness::NotRunning)
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

fn open_existing(path: &Path, write: bool) -> std::io::Result<Option<File>> {
    match base_options().read(true).write(write).open(path) {
        Ok(file) => Ok(Some(file)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Wait until no process holds `role` in `tier`, polling every 100 ms; a
/// holder in the other tier is ignored.
///
/// # Errors
///
/// [`CliError::Service`] when the role still runs in `tier` after `timeout`;
/// errors from [`probe_tier`].
#[cfg_attr(
    not(any(windows, test)),
    expect(dead_code, reason = "only the Windows system verbs wait for release")
)]
pub(crate) fn wait_released_in(role: Role, tier: Tier, timeout: std::time::Duration) -> Result<()> {
    wait_released_with(role, timeout, |role| probe_tier(role, tier))
}

/// [`wait_released_in`] with the probe injected, so tests need no real lock.
fn wait_released_with(
    role: Role,
    timeout: std::time::Duration,
    mut probe: impl FnMut(Role) -> Result<Liveness>,
) -> Result<()> {
    const POLL: std::time::Duration = std::time::Duration::from_millis(100);
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if matches!(probe(role)?, Liveness::NotRunning) {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(CliError::Service(format!(
                "{} did not stop within {timeout:?}; not starting a second instance",
                role.name(),
            ))
            .into());
        }
        std::thread::sleep(POLL);
    }
}

/// Refuse when `role` is running in `tier`; used before `service start`.
/// A holder in the other tier never refuses.
///
/// # Errors
///
/// [`CliError::Conflict`] when a process of the role runs in `tier`.
pub(crate) fn ensure_not_running_in(role: Role, tier: Tier) -> Result<()> {
    match probe_tier(role, tier)? {
        Liveness::NotRunning => Ok(()),
        Liveness::Running { tier, owner } => Err(already_running(role, tier, owner.as_ref())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Poll `released` for up to 2 s. Between fork and exec a process that
    /// another test in this multi-threaded binary spawns shares every open
    /// file description (`O_CLOEXEC` closes them only at exec), and a flock
    /// belongs to the description: a lock just dropped can read as held for
    /// that instant. A lock that is really leaked stays held and still fails.
    fn eventually(mut released: impl FnMut() -> bool) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if released() {
                return true;
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Whether `kind` can be taken on `path` now (then released again).
    fn lockable(path: &Path, kind: LockKind) -> bool {
        let file = base_options().read(true).open(path).unwrap();
        matches!(try_lock(file, kind).unwrap(), Lock::Held(_))
    }

    #[cfg(unix)]
    #[test]
    fn finalize_dir_mode_refuses_a_symlink_and_leaves_its_target_alone() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        std::fs::create_dir(&target).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700)).unwrap();
        let link = root.path().join("link");
        symlink(&target, &link).unwrap();
        assert!(finalize_dir_mode(&link, 0o755).is_err());
        let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[cfg(unix)]
    #[test]
    fn finalize_dir_mode_sets_the_mode_of_a_real_directory() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("d");
        std::fs::create_dir(&dir).unwrap();
        finalize_dir_mode(&dir, 0o755).unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);
    }

    #[test]
    fn a_refused_start_says_so() {
        let text = already_running(Role::Serve, Tier::User, None).to_string();
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
        assert_eq!(claim_lock_name(Role::Access), "access.claim.lock");
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
        assert!(eventually(|| lockable(&path, LockKind::Exclusive)));
    }

    #[test]
    fn owner_record_round_trips_without_secret_material() {
        let dir = tempfile::tempdir().unwrap();
        write_owner(dir.path(), Role::Serve, Some("abc"), 0o600).unwrap();
        let owner = read_owner(dir.path(), Role::Serve).unwrap();
        assert_eq!(owner.pid, std::process::id());
        assert_eq!(owner.node_id.as_deref(), Some("abc"));
        let raw = std::fs::read_to_string(dir.path().join("serve.owner.json")).unwrap();
        assert!(!raw.contains("secret"), "{raw}");
    }

    #[test]
    fn machine_dir_from_program_data_joins_iroh_tunnel_and_verifies() {
        let machine = machine_dir_from_program_data(Ok("C:\\ProgramData".into()))
            .unwrap()
            .expect("a machine dir");
        assert!(
            machine.path.ends_with("iroh-tunnel"),
            "{}",
            machine.path.display()
        );
        assert!(machine.verify);
    }

    #[test]
    fn machine_dir_from_program_data_propagates_a_resolution_failure() {
        let err = machine_dir_from_program_data(Err(anyhow::anyhow!("hr 0x80004005")))
            .expect_err("resolution failure must propagate");
        let text = format!("{err:#}");
        assert!(text.contains("%ProgramData%"), "{text}");
        assert!(text.contains("0x80004005"), "{text}");
    }

    #[test]
    fn tier_dirs_resolution_fails_when_program_data_cannot_be_resolved() {
        let err = TierDirs::resolve_for_with(Tier::System, || {
            machine_dir_from_program_data(Err(anyhow::anyhow!("hr")))
        })
        .expect_err("must fail closed");
        assert!(format!("{err:#}").contains("%ProgramData%"), "{err:#}");
    }

    #[test]
    fn user_tier_resolution_survives_a_machine_directory_failure() {
        let dirs = TierDirs::resolve_for_with(Tier::User, || {
            machine_dir_from_program_data(Err(anyhow::anyhow!("hr")))
        })
        .expect("a user claim never needs the machine directory");
        assert!(dirs.machine.is_none());
        assert!(dirs.user.is_some());
    }

    #[test]
    fn system_claim_lock_is_exclusive_and_detection_lock_shared() {
        let dir = tempfile::tempdir().unwrap();
        let first = claim_system_lock(Role::Serve, dir.path(), false).unwrap();
        let claim = dir.path().join(claim_lock_name(Role::Serve));
        let detect = dir.path().join(lock_name(Role::Serve));
        assert!(
            !lockable(&claim, LockKind::Shared),
            "claim held exclusively"
        );
        assert!(lockable(&detect, LockKind::Shared), "detection held shared");
        assert!(!lockable(&detect, LockKind::Exclusive), "detection held");
        drop(first);
        assert!(eventually(|| lockable(&claim, LockKind::Exclusive)));
        assert!(eventually(|| lockable(&detect, LockKind::Exclusive)));
    }

    #[test]
    fn non_admin_holder_cannot_block_the_system_claim() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = temp_dirs(&tmp);
        let machine = machine_path(&dirs);
        std::fs::create_dir_all(&machine).unwrap();
        // Any account can open the world-readable detection lock and hold it.
        let _squatter = hold_for_test(Role::Serve, &machine, LockKind::Shared).unwrap();
        let claim = claim_in(Role::Serve, Tier::System, dirs)
            .unwrap_or_else(|e| panic!("a shared holder must not refuse: {e:#}"));
        assert!(matches!(
            tier_liveness_in(Role::Serve, Tier::System, &machine).unwrap(),
            Liveness::Running {
                tier: Tier::System,
                ..
            }
        ));
        drop(claim);
    }

    #[test]
    fn system_claim_creates_a_missing_machine_dir_and_its_ancestors() {
        let tmp = tempfile::tempdir().unwrap();
        let machine = tmp.path().join("state/iroh-tunnel/machine");
        let dirs = TierDirs {
            user: None,
            machine: Some(MachineDir {
                path: machine.clone(),
                verify: false,
            }),
        };
        let claim = claim_in(Role::Serve, Tier::System, dirs)
            .unwrap_or_else(|e| panic!("a missing machine dir must be created: {e:#}"));
        assert!(machine.join(claim_lock_name(Role::Serve)).is_file());
        drop(claim);
    }

    #[test]
    fn second_system_claim_is_refused_through_the_claim_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let machine = machine_path(&temp_dirs(&tmp));
        std::fs::create_dir_all(&machine).unwrap();
        // Only the claim lock is held; the detection lock is free.
        let claim = create_lock_file(&machine.join(claim_lock_name(Role::Serve)), 0o600).unwrap();
        let Lock::Held(_claim) = try_lock(claim, LockKind::Exclusive).unwrap() else {
            panic!("first holder must win");
        };
        let err = claim_in(Role::Serve, Tier::System, temp_dirs(&tmp)).expect_err("refused");
        let msg = conflict_message(&err);
        assert!(msg.contains("already running"), "{msg}");
        assert!(msg.contains("system service"), "{msg}");
    }

    #[cfg(unix)]
    #[test]
    fn existing_claim_lock_keeps_its_mode() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(claim_lock_name(Role::Serve));
        std::fs::write(&path, "").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        drop(claim_system_lock(Role::Serve, tmp.path(), false).unwrap());
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640, "must not be re-chmodded: {mode:o}");
    }

    fn running() -> Result<Liveness> {
        Ok(Liveness::Running {
            tier: Tier::User,
            owner: None,
        })
    }

    #[test]
    fn wait_released_times_out() {
        let err = wait_released_with(Role::Access, Duration::from_millis(200), |_| running())
            .unwrap_err();
        match err.downcast_ref::<CliError>() {
            Some(CliError::Service(m)) => {
                assert!(m.contains("did not stop within 200ms"), "{m}");
                assert!(m.contains("not starting a second instance"), "{m}");
            }
            other => panic!("expected Service, got {other:?}"),
        }
    }

    #[test]
    fn wait_released_returns_once_the_role_is_free() {
        let mut polls = 0;
        wait_released_with(Role::Access, Duration::from_secs(5), |_| {
            polls += 1;
            if polls < 3 {
                running()
            } else {
                Ok(Liveness::NotRunning)
            }
        })
        .unwrap();
        assert_eq!(polls, 3);
    }

    #[cfg(windows)]
    fn sddl_of(path: &Path) -> String {
        let out = std::process::Command::new("pwsh")
            .args(["-NoProfile", "-Command"])
            .arg(format!("(Get-Acl -LiteralPath '{}').Sddl", path.display()))
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// Machine-dir tests write under `%ProgramData%`, outside the temp dir, so
    /// they only run where the e2e service environment opts in.
    #[cfg(windows)]
    fn programdata_enabled() -> bool {
        if std::env::var_os("IROH_TUNNEL_E2E_SERVICE").is_none() {
            eprintln!("skipped: IROH_TUNNEL_E2E_SERVICE is unset (writes under %ProgramData%)");
            return false;
        }
        true
    }

    /// A unique directory under `%ProgramData%`, removed on drop.
    #[cfg(windows)]
    struct ProgramDataDir(PathBuf);

    #[cfg(windows)]
    impl ProgramDataDir {
        fn new() -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let base = crate::service::program_data_dir().unwrap();
            Self(base.join(format!("iroh-tunnel-test-{}-{nanos}", std::process::id())))
        }
    }

    #[cfg(windows)]
    impl Drop for ProgramDataDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(windows)]
    #[test]
    fn untrusted_existing_machine_dir_is_refused() {
        if !programdata_enabled() {
            return;
        }
        let guard = ProgramDataDir::new();
        let dir = guard.0.clone();
        std::fs::create_dir(&dir).unwrap();
        let grant = std::process::Command::new("icacls")
            .arg(&dir)
            .args(["/grant", "*S-1-1-0:M"])
            .output()
            .unwrap();
        assert!(grant.status.success(), "{grant:?}");
        let before = sddl_of(&dir);
        let err = claim_system_lock(Role::Serve, &dir, true).expect_err("refused");
        match err.downcast_ref::<CliError>() {
            Some(CliError::Permission(m)) => {
                assert!(m.contains("S-1-1-0"), "{m}");
                assert!(
                    m.contains(dir.file_name().unwrap().to_str().unwrap()),
                    "{m}"
                );
            }
            other => panic!("expected Permission, got {other:?}"),
        }
        assert_eq!(before, sddl_of(&dir), "the directory must not be repaired");
    }

    #[cfg(windows)]
    #[test]
    fn existing_trusted_machine_dir_is_accepted() {
        if !programdata_enabled() {
            return;
        }
        let guard = ProgramDataDir::new();
        create_machine_dir(&guard.0).unwrap();
        drop(claim_system_lock(Role::Serve, &guard.0, true).unwrap());
    }

    #[cfg(windows)]
    #[test]
    fn created_machine_dir_has_the_protected_sddl() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("iroh-tunnel");
        create_machine_dir(&dir).unwrap();
        let sddl = sddl_of(&dir);
        assert!(sddl.starts_with("O:BA"), "{sddl}");
        // CreateDirectoryW with an explicit descriptor drops the AI
        // (auto-inherited) control flag; the DACL stays protected (P).
        let dacl = sddl.split_once("D:").expect("DACL present").1;
        let aces = "(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;BU)";
        assert!(
            dacl == format!("P{aces}") || dacl == format!("PAI{aces}"),
            "{sddl}"
        );
    }

    #[test]
    fn tier_liveness_in_reports_a_held_lock() {
        let dir = tempfile::tempdir().unwrap();
        let _hold = hold_for_test(Role::Serve, dir.path(), LockKind::Exclusive).unwrap();
        let seen = tier_liveness_in(Role::Serve, Tier::User, dir.path()).unwrap();
        assert!(
            matches!(
                seen,
                Liveness::Running {
                    tier: Tier::User,
                    ..
                }
            ),
            "{seen:?}"
        );
        // The system tier is judged by its claim lock only.
        let seen = tier_liveness_in(Role::Serve, Tier::System, dir.path()).unwrap();
        assert!(matches!(seen, Liveness::NotRunning), "{seen:?}");
        let _system = claim_system_lock(Role::Serve, dir.path(), false);
        let seen = tier_liveness_in(Role::Serve, Tier::System, dir.path()).unwrap();
        assert!(
            matches!(
                seen,
                Liveness::Running {
                    tier: Tier::System,
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
            tier_liveness_in(Role::Serve, Tier::User, dir.path()).unwrap()
        else {
            panic!("a held lock must read as Running")
        };
        assert!(owner.is_none(), "no record was written");
    }

    #[test]
    fn tier_liveness_in_is_not_running_for_an_existing_but_unlocked_file() {
        let dir = tempfile::tempdir().unwrap();
        create_lock_file(&dir.path().join(lock_name(Role::Serve)), 0o600).unwrap();
        create_lock_file(&dir.path().join(claim_lock_name(Role::Serve)), 0o600).unwrap();
        for tier in [Tier::User, Tier::System] {
            let seen = tier_liveness_in(Role::Serve, tier, dir.path()).unwrap();
            assert!(matches!(seen, Liveness::NotRunning), "{seen:?}");
        }
        let seen = tier_liveness_in(Role::Serve, Tier::System, dir.path()).unwrap();
        assert!(matches!(seen, Liveness::NotRunning), "{seen:?}");
    }

    #[test]
    fn a_shared_hold_is_not_an_owner_in_either_tier() {
        let dir = tempfile::tempdir().unwrap();
        let _hold = hold_for_test(Role::Serve, dir.path(), LockKind::Shared).unwrap();
        for tier in [Tier::System, Tier::User] {
            assert!(matches!(
                tier_liveness_in(Role::Serve, tier, dir.path()).unwrap(),
                Liveness::NotRunning
            ));
        }
    }

    /// Fresh user and machine tier directories under `tmp`.
    fn temp_dirs(tmp: &tempfile::TempDir) -> TierDirs {
        TierDirs {
            user: Some(tmp.path().join("user")),
            machine: Some(MachineDir {
                path: tmp.path().join("machine"),
                verify: false,
            }),
        }
    }

    fn machine_path(dirs: &TierDirs) -> PathBuf {
        dirs.machine.as_ref().unwrap().path.clone()
    }

    fn user_path(dirs: &TierDirs) -> PathBuf {
        dirs.user.clone().unwrap()
    }

    fn conflict_message(err: &anyhow::Error) -> String {
        match err.downcast_ref::<CliError>() {
            Some(CliError::Conflict(m)) => m.clone(),
            other => panic!("expected Conflict, got {other:?}: {err:#}"),
        }
    }

    const CORRUPT: &str = "{not json";

    #[test]
    fn write_owner_failure_names_the_record_path() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("missing");
        let err = write_owner(&missing, Role::Serve, None, 0o600).expect_err("must fail");
        assert!(format!("{err:#}").contains("serve.owner.json"), "{err:#}");
    }

    /// Publish into `dir` over a temp file pre-created with `initial` mode
    /// (what a narrowing umask produces) and return the record's mode.
    /// Fork-free on purpose: a child spawned from this multi-threaded test
    /// process would inherit concurrent tests' lock descriptors.
    #[cfg(unix)]
    fn published_mode(initial: u32, mode: u32) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let temp = tmp
            .path()
            .join(format!("serve.owner.json.{}.tmp", std::process::id()));
        std::fs::write(&temp, "").unwrap();
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(initial)).unwrap();
        write_owner(tmp.path(), Role::Serve, None, mode).unwrap();
        std::fs::metadata(tmp.path().join("serve.owner.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777
    }

    #[cfg(unix)]
    #[test]
    // Stands in for the spec scenario "Machine record readable under a restrictive umask": umask is process-global and a child-process test inherits concurrent tests' lock fds.
    fn machine_owner_record_mode_ignores_the_creation_mode() {
        let mode = published_mode(0o600, MACHINE_RECORD_MODE);
        assert_eq!(mode, 0o644, "mode {mode:o}");
    }

    #[cfg(unix)]
    #[test]
    fn user_owner_record_is_private_even_over_a_wider_temp() {
        let mode = published_mode(0o644, USER_RECORD_MODE);
        assert_eq!(mode, 0o600, "mode {mode:o}");
    }

    #[test]
    fn read_owner_record_distinguishes_absent_present_and_corrupt() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        assert!(matches!(
            read_owner_record(dir, Role::Serve),
            OwnerRead::Absent
        ));
        write_owner(dir, Role::Serve, Some("n"), 0o600).unwrap();
        assert!(matches!(
            read_owner_record(dir, Role::Serve),
            OwnerRead::Present(_)
        ));
        std::fs::write(dir.join("serve.owner.json"), CORRUPT).unwrap();
        match read_owner_record(dir, Role::Serve) {
            OwnerRead::Unreadable { path, .. } => {
                assert!(path.ends_with("serve.owner.json"), "{}", path.display());
            }
            other => panic!("expected Unreadable, got {other:?}"),
        }
    }

    /// Claim `role` in `tier` over a fresh pair of directories under `tmp`.
    fn claim_tier(role: Role, tier: Tier, tmp: &tempfile::TempDir) -> RoleClaim {
        claim_in(role, tier, temp_dirs(tmp)).unwrap()
    }

    #[test]
    fn publish_identity_refuses_the_same_key_in_the_other_role() {
        for tier in [Tier::User, Tier::System] {
            let tmp = tempfile::tempdir().unwrap();
            let serve = claim_tier(Role::Serve, tier, &tmp);
            serve.publish_identity("K").unwrap();
            let access = claim_tier(Role::Access, tier, &tmp);
            let err = access.publish_identity("K").expect_err("same key refused");
            let msg = conflict_message(&err);
            assert!(msg.contains("same public identity"), "{msg}");
        }
    }

    #[test]
    fn publish_identity_refuses_when_the_running_other_role_record_is_corrupt() {
        for tier in [Tier::User, Tier::System] {
            let tmp = tempfile::tempdir().unwrap();
            let dirs = temp_dirs(&tmp);
            let record_dir = dirs.own_dir(tier).unwrap().to_path_buf();
            let _serve = claim_tier(Role::Serve, tier, &tmp);
            std::fs::write(record_dir.join("serve.owner.json"), CORRUPT).unwrap();
            let access = claim_in(Role::Access, tier, dirs).unwrap();
            let err = access.publish_identity("A").expect_err("must refuse");
            let msg = conflict_message(&err);
            assert!(msg.starts_with("start refused:"), "{msg}");
            assert!(msg.contains("identity cannot be determined"), "{msg}");
            assert!(msg.contains("serve.owner.json"), "{msg}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn publish_identity_refuses_when_the_running_other_role_record_is_unreadable() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = temp_dirs(&tmp);
        let record = user_path(&dirs).join("serve.owner.json");
        let _serve = claim_tier(Role::Serve, Tier::User, &tmp);
        std::fs::remove_file(&record).unwrap();
        std::fs::create_dir(&record).unwrap();
        let access = claim_in(Role::Access, Tier::User, dirs).unwrap();
        let err = access.publish_identity("A").expect_err("must refuse");
        let msg = conflict_message(&err);
        assert!(msg.contains("identity cannot be determined"), "{msg}");
        assert!(msg.contains("serve.owner.json"), "{msg}");
    }

    #[test]
    fn publish_identity_proceeds_while_the_other_role_has_no_record() {
        for tier in [Tier::User, Tier::System] {
            let tmp = tempfile::tempdir().unwrap();
            let serve = claim_tier(Role::Serve, tier, &tmp);
            let access = claim_tier(Role::Access, tier, &tmp);
            access.publish_identity("A").unwrap();
            let dirs = temp_dirs(&tmp);
            std::fs::remove_file(dirs.own_dir(tier).unwrap().join("serve.owner.json")).unwrap();
            access.publish_identity("A").unwrap();
            drop((access, serve));
        }
    }

    #[test]
    fn publish_identity_ignores_a_corrupt_record_of_a_stopped_role() {
        for tier in [Tier::User, Tier::System] {
            let tmp = tempfile::tempdir().unwrap();
            let dirs = temp_dirs(&tmp);
            let dir = dirs.own_dir(tier).unwrap().to_path_buf();
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("serve.owner.json"), CORRUPT).unwrap();
            let access = claim_in(Role::Access, tier, dirs).unwrap();
            access.publish_identity("A").unwrap();
        }
    }

    #[test]
    fn identity_check_ignores_other_tier() {
        let tmp = tempfile::tempdir().unwrap();
        let system = claim_tier(Role::Serve, Tier::System, &tmp);
        system.publish_identity("K").unwrap();
        let user = claim_tier(Role::Access, Tier::User, &tmp);
        user.publish_identity("K").unwrap();
        let access_system = claim_tier(Role::Access, Tier::System, &tmp);
        drop(access_system);
    }

    #[test]
    fn corrupt_machine_record_ignored_for_user_claim() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = temp_dirs(&tmp);
        let machine = machine_path(&dirs);
        std::fs::create_dir_all(&machine).unwrap();
        let _hold = hold_for_test(Role::Serve, &machine, LockKind::Exclusive).unwrap();
        std::fs::write(machine.join("serve.owner.json"), CORRUPT).unwrap();
        let access = claim_in(Role::Access, Tier::User, dirs).unwrap();
        access.publish_identity("A").unwrap();
    }

    #[test]
    fn held_exclusively_distinguishes_free_shared_and_exclusive() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(lock_name(Role::Serve));
        assert!(!held_exclusively(&path).unwrap());
        let shared = hold_for_test(Role::Serve, tmp.path(), LockKind::Shared).unwrap();
        assert!(!held_exclusively(&path).unwrap());
        drop(shared);
        assert!(eventually(|| lockable(&path, LockKind::Exclusive)));
        let _exclusive = hold_for_test(Role::Serve, tmp.path(), LockKind::Exclusive).unwrap();
        assert!(held_exclusively(&path).unwrap());
    }

    #[test]
    fn a_leftover_temp_record_does_not_block_publishing() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = temp_dirs(&tmp);
        let user = user_path(&dirs);
        std::fs::create_dir_all(&user).unwrap();
        let leftover = user.join(format!("serve.owner.json.{}.tmp", std::process::id()));
        std::fs::write(&leftover, "stale").unwrap();
        let _serve = claim_in(Role::Serve, Tier::User, dirs).unwrap();
        let owner = read_owner(&user, Role::Serve).expect("record published");
        assert_eq!(owner.pid, std::process::id());
    }

    #[test]
    fn claim_fails_when_the_owner_record_cannot_be_written() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = temp_dirs(&tmp);
        let blocker = user_path(&dirs).join("serve.owner.json");
        std::fs::create_dir_all(blocker.join("inner")).unwrap();
        let err = claim_in(Role::Serve, Tier::User, dirs)
            .expect_err("publish failure must fail the claim");
        assert!(format!("{err:#}").contains("serve.owner.json"), "{err:#}");
        std::fs::remove_dir_all(&blocker).unwrap();
        claim_tier(Role::Serve, Tier::User, &tmp);
    }

    // --- tier claims ---

    fn presence(dirs: &TierDirs) -> PathBuf {
        machine_path(dirs).join(presence_name(Role::Serve))
    }

    #[test]
    fn system_claim_creates_presence_file_unlocked() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = temp_dirs(&tmp);
        let path = presence(&dirs);
        let _claim = claim_in(Role::Serve, Tier::System, dirs).unwrap();
        assert!(path.exists());
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
        let file = File::open(&path).unwrap();
        file.try_lock().expect("the system claim must not hold it");
    }

    #[test]
    fn user_claim_holds_presence_shared_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = temp_dirs(&tmp);
        let path = presence(&dirs);
        drop(claim_in(Role::Serve, Tier::System, temp_dirs(&tmp)).unwrap());
        let claim = claim_in(Role::Serve, Tier::User, dirs).unwrap();
        assert!(held_shared(&path).unwrap(), "presence must be held");
        // Shared: a second user process can hold it too.
        let other = File::open(&path).unwrap();
        other.try_lock_shared().unwrap();
        drop(other);
        drop(claim);
        assert!(
            eventually(|| !held_shared(&path).unwrap()),
            "released on drop"
        );
    }

    #[test]
    fn user_claim_without_machine_dir_creates_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = temp_dirs(&tmp);
        let machine = machine_path(&dirs);
        let _claim = claim_in(Role::Serve, Tier::User, dirs).unwrap();
        assert!(!machine.exists(), "no machine directory may be created");
    }

    #[test]
    fn user_claim_with_a_machine_dir_but_no_presence_creates_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = temp_dirs(&tmp);
        let machine = machine_path(&dirs);
        std::fs::create_dir_all(&machine).unwrap();
        let _claim = claim_in(Role::Serve, Tier::User, dirs).unwrap();
        assert_eq!(std::fs::read_dir(&machine).unwrap().count(), 0);
    }

    #[test]
    fn user_claim_survives_an_unresolvable_machine_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let mut dirs = temp_dirs(&tmp);
        dirs.machine = None;
        claim_in(Role::Serve, Tier::User, dirs).unwrap();
    }

    #[test]
    fn system_claim_without_a_machine_dir_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let mut dirs = temp_dirs(&tmp);
        dirs.machine = None;
        claim_in(Role::Serve, Tier::System, dirs).expect_err("no machine dir");
    }

    #[test]
    fn system_and_user_claims_coexist() {
        let tmp = tempfile::tempdir().unwrap();
        let system = claim_tier(Role::Serve, Tier::System, &tmp);
        let user = claim_tier(Role::Serve, Tier::User, &tmp);
        let dirs = temp_dirs(&tmp);
        assert!(read_owner(&machine_path(&dirs), Role::Serve).is_some());
        assert!(read_owner(&user_path(&dirs), Role::Serve).is_some());
        drop((system, user));
    }

    #[test]
    fn second_system_claim_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let _first = claim_tier(Role::Serve, Tier::System, &tmp);
        let err = claim_in(Role::Serve, Tier::System, temp_dirs(&tmp)).expect_err("refused");
        let msg = conflict_message(&err);
        assert!(msg.contains("already running"), "{msg}");
        assert!(msg.contains("system service"), "{msg}");
    }

    #[test]
    fn second_user_claim_is_refused_and_names_the_user_session() {
        let tmp = tempfile::tempdir().unwrap();
        let _first = claim_tier(Role::Serve, Tier::User, &tmp);
        let err = claim_in(Role::Serve, Tier::User, temp_dirs(&tmp)).expect_err("refused");
        let msg = conflict_message(&err);
        assert!(msg.contains("already running"), "{msg}");
        assert!(msg.contains("user session"), "{msg}");
    }

    // --- probes ---

    /// Run `name` in a child process whose machine dir is under its fresh
    /// `HOME`, with the tier chosen per call through `IROH_TUNNEL_TEST_TIER`.
    /// The body claims user and system tiers with distinct node ids (via the
    /// production seam: `claim`, `probe_tier`, `wait_released_in`).
    #[cfg(unix)]
    fn in_two_tier_home(name: &str, body: impl FnOnce(&Path)) {
        crate::fixtures::in_isolated_home(name, |home| {
            // SAFETY: single test thread in an isolated child process.
            unsafe {
                std::env::set_var(ENV_MACHINE_LOCK_DIR, home.join("machine"));
            }
            body(home);
        });
    }

    /// Claim as `tier` through the production `claim`.
    #[cfg(unix)]
    fn claim_as(role: Role, tier: Tier) -> RoleClaim {
        // SAFETY: single test thread in an isolated child process.
        unsafe {
            std::env::set_var(
                "IROH_TUNNEL_TEST_TIER",
                if tier == Tier::System {
                    "system"
                } else {
                    "user"
                },
            );
        }
        claim(role).unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn probe_tier_sees_only_its_tier() {
        const NAME: &str = "ownership::tests::probe_tier_sees_only_its_tier";
        in_two_tier_home(NAME, |_| {
            let system = claim_as(Role::Serve, Tier::System);
            system.publish_identity("SYSTEM-NODE").unwrap();
            let user = claim_as(Role::Serve, Tier::User);
            user.publish_identity("USER-NODE").unwrap();
            for (tier, node) in [(Tier::User, "USER-NODE"), (Tier::System, "SYSTEM-NODE")] {
                let Liveness::Running { tier: seen, owner } =
                    probe_tier(Role::Serve, tier).unwrap()
                else {
                    panic!("{tier:?} must read as running");
                };
                assert_eq!(seen, tier);
                assert_eq!(owner.unwrap().node_id.as_deref(), Some(node));
            }
        });
    }

    #[cfg(unix)]
    #[test]
    fn probe_follows_the_current_tier() {
        const NAME: &str = "ownership::tests::probe_follows_the_current_tier";
        in_two_tier_home(NAME, |_| {
            let system = claim_as(Role::Serve, Tier::System);
            system.publish_identity("SYSTEM-NODE").unwrap();
            let _user = claim_as(Role::Serve, Tier::User);
            let Liveness::Running { tier, owner } = probe(Role::Serve).unwrap() else {
                panic!("user tier is running");
            };
            assert_eq!(tier, Tier::User);
            assert_ne!(owner.unwrap().node_id.as_deref(), Some("SYSTEM-NODE"));
        });
    }

    #[cfg(unix)]
    #[test]
    fn probe_tier_does_not_see_the_other_tier() {
        const NAME: &str = "ownership::tests::probe_tier_does_not_see_the_other_tier";
        in_two_tier_home(NAME, |_| {
            let _system = claim_as(Role::Serve, Tier::System);
            assert!(matches!(
                probe_tier(Role::Serve, Tier::User).unwrap(),
                Liveness::NotRunning
            ));
        });
    }

    /// Pretend this account is the admin: the record it writes is trusted.
    #[cfg(unix)]
    fn trust_me_as_admin() {
        // SAFETY: single test thread in an isolated child process; geteuid is
        // always safe.
        unsafe {
            std::env::set_var(
                "IROH_TUNNEL_TEST_TRUSTED_OWNER",
                libc::geteuid().to_string(),
            );
        }
    }

    /// An earlier version's system process: the exclusive `<role>.lock` and an
    /// owner record naming `pid`, in `machine` (no claim lock).
    #[cfg(unix)]
    fn old_system_holder(machine: &Path, pid: u32) -> TestHold {
        std::fs::create_dir_all(machine).unwrap();
        let hold = hold_for_test(Role::Serve, machine, LockKind::Exclusive).unwrap();
        let record = OwnerRecord {
            role: "serve".into(),
            pid,
            node_id: None,
            started_at: 1,
        };
        std::fs::write(
            machine.join(owner_name(Role::Serve)),
            serde_json::to_string(&record).unwrap(),
        )
        .unwrap();
        hold
    }

    #[cfg(unix)]
    fn dead_pid() -> u32 {
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        pid
    }

    #[cfg(unix)]
    #[test]
    fn earlier_version_system_instance_refuses_claim() {
        const NAME: &str = "ownership::tests::earlier_version_system_instance_refuses_claim";
        in_two_tier_home(NAME, |home| {
            trust_me_as_admin();
            let machine = home.join("machine");
            let _old = old_system_holder(&machine, std::process::id());
            let dirs = TierDirs::resolve_for(Tier::System).unwrap();
            let err = claim_in(Role::Serve, Tier::System, dirs).expect_err("refused");
            let msg = conflict_message(&err);
            assert!(msg.contains("earlier version"), "{msg}");
            assert!(msg.contains("stop it first"), "{msg}");
            assert!(matches!(
                err.downcast_ref::<CliError>(),
                Some(CliError::Conflict(_))
            ));
            // The claim lock taken on the way was released again.
            let claim = machine.join(claim_lock_name(Role::Serve));
            assert!(lockable(&claim, LockKind::Exclusive));
        });
    }

    #[cfg(unix)]
    #[test]
    fn earlier_version_instance_is_probed_running() {
        const NAME: &str = "ownership::tests::earlier_version_instance_is_probed_running";
        in_two_tier_home(NAME, |home| {
            trust_me_as_admin();
            let _old = old_system_holder(&home.join("machine"), std::process::id());
            let Liveness::Running { tier, owner } = probe_tier(Role::Serve, Tier::System).unwrap()
            else {
                panic!("must read as running");
            };
            assert_eq!(tier, Tier::System);
            assert_eq!(owner.unwrap().pid, std::process::id());
            assert!(matches!(
                probe_tier(Role::Serve, Tier::User).unwrap(),
                Liveness::NotRunning
            ));
        });
    }

    #[cfg(unix)]
    #[test]
    fn wait_released_in_waits_for_earlier_version_holder() {
        const NAME: &str = "ownership::tests::wait_released_in_waits_for_earlier_version_holder";
        in_two_tier_home(NAME, |home| {
            trust_me_as_admin();
            let old = old_system_holder(&home.join("machine"), std::process::id());
            let err = wait_released_in(Role::Serve, Tier::System, Duration::from_millis(250))
                .expect_err("still held");
            assert!(matches!(
                err.downcast_ref::<CliError>(),
                Some(CliError::Service(_))
            ));
            drop(old);
            wait_released_in(Role::Serve, Tier::System, Duration::from_millis(500)).unwrap();
        });
    }

    #[cfg(unix)]
    #[test]
    fn non_admin_exclusive_holder_without_record_does_not_block() {
        const NAME: &str =
            "ownership::tests::non_admin_exclusive_holder_without_record_does_not_block";
        in_two_tier_home(NAME, |home| {
            trust_me_as_admin();
            let machine = home.join("machine");
            std::fs::create_dir_all(&machine).unwrap();
            let _squatter = hold_for_test(Role::Serve, &machine, LockKind::Exclusive).unwrap();
            assert!(matches!(
                probe_tier(Role::Serve, Tier::System).unwrap(),
                Liveness::NotRunning
            ));
            let dirs = TierDirs::resolve_for(Tier::System).unwrap();
            drop(claim_in(Role::Serve, Tier::System, dirs).expect("must proceed"));
        });
    }

    #[cfg(unix)]
    #[test]
    fn untrusted_owner_record_does_not_block() {
        const NAME: &str = "ownership::tests::untrusted_owner_record_does_not_block";
        in_two_tier_home(NAME, |home| {
            // No test owner: only root is trusted, and a non-root account (or
            // root itself, which then has nothing to prove) writes the record.
            let machine = home.join("machine");
            let _old = old_system_holder(&machine, std::process::id());
            // SAFETY: geteuid is always safe.
            if unsafe { libc::geteuid() } == 0 {
                return;
            }
            assert!(matches!(
                probe_tier(Role::Serve, Tier::System).unwrap(),
                Liveness::NotRunning
            ));
        });
    }

    #[cfg(unix)]
    #[test]
    fn stale_record_with_dead_pid_does_not_block() {
        const NAME: &str = "ownership::tests::stale_record_with_dead_pid_does_not_block";
        in_two_tier_home(NAME, |home| {
            trust_me_as_admin();
            let _old = old_system_holder(&home.join("machine"), dead_pid());
            assert!(matches!(
                probe_tier(Role::Serve, Tier::System).unwrap(),
                Liveness::NotRunning
            ));
            let dirs = TierDirs::resolve_for(Tier::System).unwrap();
            drop(claim_in(Role::Serve, Tier::System, dirs).expect("must proceed"));
        });
    }

    #[cfg(unix)]
    #[test]
    fn wait_released_in_ignores_other_tier() {
        const NAME: &str = "ownership::tests::wait_released_in_ignores_other_tier";
        in_two_tier_home(NAME, |_| {
            let user = claim_as(Role::Serve, Tier::User);
            // A user holder never delays a system wait ...
            wait_released_in(Role::Serve, Tier::System, Duration::from_millis(300)).unwrap();
            // ... but does delay a user wait, until it exits.
            let err = wait_released_in(Role::Serve, Tier::User, Duration::from_millis(200))
                .expect_err("own tier still held");
            assert!(matches!(
                err.downcast_ref::<CliError>(),
                Some(CliError::Service(_))
            ));
            drop(user);
            wait_released_in(Role::Serve, Tier::User, Duration::from_millis(300)).unwrap();
        });
    }

    #[test]
    fn user_sees_system_holder() {
        let tmp = tempfile::tempdir().unwrap();
        let machine = machine_path(&temp_dirs(&tmp));
        assert_eq!(
            other_tier_running_in(Role::Serve, Tier::User, &machine),
            None
        );
        let _system = claim_tier(Role::Serve, Tier::System, &tmp);
        assert_eq!(
            other_tier_running_in(Role::Serve, Tier::User, &machine),
            Some(Tier::System)
        );
        assert_eq!(
            other_tier_running_in(Role::Access, Tier::User, &machine),
            None,
            "another role is not the same instance"
        );
    }

    #[test]
    fn system_sees_shared_user_holder() {
        let tmp = tempfile::tempdir().unwrap();
        let machine = machine_path(&temp_dirs(&tmp));
        let _system = claim_tier(Role::Serve, Tier::System, &tmp);
        let _user = claim_tier(Role::Serve, Tier::User, &tmp);
        assert_eq!(
            other_tier_running_in(Role::Serve, Tier::System, &machine),
            Some(Tier::User)
        );
    }

    #[test]
    fn system_sees_none_after_last_user_exits() {
        let tmp = tempfile::tempdir().unwrap();
        let machine = machine_path(&temp_dirs(&tmp));
        let _system = claim_tier(Role::Serve, Tier::System, &tmp);
        let presence_path = machine.join(presence_name(Role::Serve));
        let first = hold_presence(&presence_path).unwrap();
        let second = hold_presence(&presence_path).unwrap();
        drop(first);
        assert_eq!(
            other_tier_running_in(Role::Serve, Tier::System, &machine),
            Some(Tier::User),
            "one user holder is left"
        );
        drop(second);
        assert!(eventually(|| other_tier_running_in(
            Role::Serve,
            Tier::System,
            &machine
        )
        .is_none()));
    }

    #[test]
    fn blind_spot_user_before_presence_is_not_seen() {
        let tmp = tempfile::tempdir().unwrap();
        let machine = machine_path(&temp_dirs(&tmp));
        let _user = claim_tier(Role::Serve, Tier::User, &tmp);
        let _system = claim_tier(Role::Serve, Tier::System, &tmp);
        assert_eq!(
            other_tier_running_in(Role::Serve, Tier::System, &machine),
            None
        );
    }

    #[test]
    fn probe_io_error_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        // The machine "directory" is a regular file: opening anything under
        // it fails with something other than NotFound.
        let machine = tmp.path().join("machine-is-a-file");
        std::fs::write(&machine, "").unwrap();
        for tier in [Tier::User, Tier::System] {
            assert_eq!(other_tier_running_in(Role::Serve, tier, &machine), None);
        }
        // A lock path that is a directory is not a holder either.
        let dir = tmp.path().join("m");
        std::fs::create_dir_all(dir.join(lock_name(Role::Serve))).unwrap();
        assert_eq!(other_tier_running_in(Role::Serve, Tier::User, &dir), None);
    }

    #[test]
    fn ensure_presence_in_is_idempotent_and_never_locks() {
        let tmp = tempfile::tempdir().unwrap();
        ensure_presence_in(Role::Access, tmp.path()).unwrap();
        ensure_presence_in(Role::Access, tmp.path()).unwrap();
        assert!(!held_shared(&tmp.path().join(presence_name(Role::Access))).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn existing_lock_and_presence_keep_their_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dirs = temp_dirs(&tmp);
        let machine = machine_path(&dirs);
        std::fs::create_dir_all(&machine).unwrap();
        for name in [lock_name(Role::Serve), presence_name(Role::Serve)] {
            let path = machine.join(name);
            std::fs::write(&path, "").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let _claim = claim_in(Role::Serve, Tier::System, dirs).unwrap();
        for name in [lock_name(Role::Serve), presence_name(Role::Serve)] {
            let mode = std::fs::metadata(machine.join(&name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "{name} must not be re-chmodded: {mode:o}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn new_machine_dir_lock_and_presence_are_public_under_umask_077() {
        const NAME: &str =
            "ownership::tests::new_machine_dir_lock_and_presence_are_public_under_umask_077";
        crate::fixtures::in_isolated_home(NAME, |home| {
            use std::os::unix::fs::PermissionsExt;
            // SAFETY: umask(2) only changes this child process's creation mask.
            unsafe { libc::umask(0o077) };
            let machine = home.join("run/iroh-tunnel");
            let dirs = TierDirs {
                user: None,
                machine: Some(MachineDir {
                    path: machine.clone(),
                    verify: false,
                }),
            };
            let _claim = claim_in(Role::Serve, Tier::System, dirs).unwrap();
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&machine), 0o755, "machine dir");
            assert_eq!(mode(&machine.join("serve.lock")), 0o644);
            assert_eq!(mode(&machine.join("serve.claim.lock")), 0o600, "claim lock");
            assert_eq!(mode(&machine.join("serve.user.lock")), 0o644);
            assert_eq!(mode(&machine.join("serve.owner.json")), 0o644);
            assert_eq!(mode(machine.parent().unwrap()), 0o755);
        });
    }

    #[cfg(unix)]
    #[test]
    fn existing_directory_is_never_chmodded() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("d");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        create_dir(&dir, 0o755).unwrap();
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    #[cfg(unix)]
    #[test]
    fn machine_owner_record_is_world_readable_under_umask_077() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dirs = temp_dirs(&tmp);
        let machine = machine_path(&dirs);
        std::fs::create_dir_all(&machine).unwrap();
        let temp = machine.join(format!("serve.owner.json.{}.tmp", std::process::id()));
        std::fs::write(&temp, "").unwrap();
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600)).unwrap();
        let _claim = claim_in(Role::Serve, Tier::System, dirs).unwrap();
        let mode = std::fs::metadata(machine.join("serve.owner.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o644, "{mode:o}");
    }

    #[cfg(all(unix, not(windows)))]
    #[test]
    fn machine_dir_is_platform_runtime_path() {
        let machine = platform_machine_dir().expect("a machine dir on this platform");
        #[cfg(target_os = "linux")]
        assert_eq!(machine.path, Path::new("/run/iroh-tunnel"));
        #[cfg(target_os = "macos")]
        assert_eq!(machine.path, Path::new("/var/run/iroh-tunnel"));
        assert!(!machine.verify);
    }
}
