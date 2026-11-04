//! Result types of reading a role's status.

use std::path::PathBuf;

use super::schema::StatusPayload;
use crate::ownership::Liveness;
use crate::role::Role;

/// The classified, read-only state of one role's status: where the file
/// lives, whether the role is running, and what the file held.
#[derive(Debug)]
pub struct RoleStatus {
    /// The role the status belongs to.
    pub role: Role,
    /// The status file path (it may not exist).
    pub path: PathBuf,
    /// Whether the role runs, from the role lock.
    pub liveness: Liveness,
    /// What the status file held.
    pub snapshot: Snapshot,
}

/// What reading the status file found.
#[derive(Debug)]
pub enum Snapshot {
    /// No file at the status path.
    Absent,
    /// A file that parsed as the role's typed payload; `raw` is its
    /// verbatim contents.
    Present {
        /// The file contents, verbatim.
        raw: String,
        /// The parsed payload.
        payload: StatusPayload,
    },
    /// A file that exists but does not parse.
    Invalid(anyhow::Error),
}

/// How the snapshot relates to the role's ownership lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Freshness {
    /// Role running and the snapshot was written by its recorded owner.
    Current,
    /// Role running, but no snapshot is readable at this path.
    LiveWithoutSnapshot,
    /// No snapshot and the role is not running.
    AbsentNotRunning,
    /// The snapshot file exists but could not be parsed.
    Unreadable,
    /// The snapshot is left over from another pid; `live_pid` is the current
    /// owner, `None` when the role is not running.
    StalePid {
        /// The pid currently holding the role, if any.
        live_pid: Option<u32>,
    },
    /// Role running but its owner record is missing, so the snapshot cannot
    /// be verified.
    OwnerUnknown,
}

impl RoleStatus {
    /// Judge the snapshot against the role's ownership locks (AC-RO-03,
    /// AC-RO-05, AC-RO-06).
    ///
    /// A pid is never proof of life: a crash leaves the file behind and the
    /// OS may hand the pid to an unrelated process. The kernel lock held by
    /// the running role is the only liveness signal, on every platform.
    pub fn freshness(&self) -> Freshness {
        match (&self.snapshot, &self.liveness) {
            (Snapshot::Invalid(_), _) => Freshness::Unreadable,
            (Snapshot::Absent, Liveness::NotRunning) => Freshness::AbsentNotRunning,
            (Snapshot::Absent, Liveness::Running { .. }) => Freshness::LiveWithoutSnapshot,
            (Snapshot::Present { .. }, Liveness::NotRunning) => {
                Freshness::StalePid { live_pid: None }
            }
            (Snapshot::Present { payload, .. }, Liveness::Running { owner, .. }) => match owner {
                Some(owner) if owner.pid != payload.pid() => Freshness::StalePid {
                    live_pid: Some(owner.pid),
                },
                Some(_) => Freshness::Current,
                None => Freshness::OwnerUnknown,
            },
        }
    }
}
