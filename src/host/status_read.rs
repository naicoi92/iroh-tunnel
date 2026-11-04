//! Read-only classification of a role's status file: where it lives, whether
//! the role is running and what the file held. Terminal-free; the CLI renders
//! it (`iroh-tunnel <role> status`).
//!
//! A missing status file means the role is not running. The command itself
//! is healthy, the config is fine, nothing permission-related failed, and
//! iroh was never reached — none of the specific [`crate::host::error::CliError`]
//! categories fits, so the CLI maps it to the general error (exit 1).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;

use crate::host::ownership;
pub use crate::host::ownership::{Liveness, OwnerRecord, Tier};
use crate::host::status::{Role, StatusPayload, StatusWriter};

/// The classified, read-only state of one role's status: where the file
/// lives, whether the role is running, and what the file held.
#[derive(Debug)]
pub struct RoleStatus {
    pub role: Role,
    pub path: PathBuf,
    pub liveness: Liveness,
    pub snapshot: Snapshot,
}

/// What reading the status file found.
#[derive(Debug)]
pub enum Snapshot {
    /// No file at the status path.
    Absent,
    /// A file that parsed as the role's typed payload; `raw` is its
    /// verbatim contents.
    Present { raw: String, payload: StatusPayload },
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
    StalePid { live_pid: Option<u32> },
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
                Some(owner) if owner.pid != snapshot_pid(payload) => Freshness::StalePid {
                    live_pid: Some(owner.pid),
                },
                Some(_) => Freshness::Current,
                None => Freshness::OwnerUnknown,
            },
        }
    }
}

/// Pid recorded in the snapshot.
pub fn snapshot_pid(payload: &StatusPayload) -> u32 {
    match payload {
        StatusPayload::Serve(file) => file.pid,
        StatusPayload::Access(file) => file.pid,
    }
}

/// Start time (epoch seconds) recorded in the snapshot.
pub fn snapshot_started_at(payload: &StatusPayload) -> u64 {
    match payload {
        StatusPayload::Serve(file) => file.started_at,
        StatusPayload::Access(file) => file.started_at,
    }
}

/// Read and classify a role's status without creating anything.
///
/// # Errors
///
/// Only path resolution, the ownership probe, and non-"not found" read
/// errors. A missing file is [`Snapshot::Absent`]; an unparseable one is
/// [`Snapshot::Invalid`].
pub fn read_role_status(role: Role) -> Result<RoleStatus> {
    let path = StatusWriter::new(role).path()?;
    let liveness = ownership::probe(role)?;
    read_role_status_at(role, path, liveness)
}

fn read_role_status_at(role: Role, path: PathBuf, liveness: Liveness) -> Result<RoleStatus> {
    let snapshot = match std::fs::read_to_string(&path) {
        Ok(raw) => match parse_payload(role, &raw, &path) {
            Ok(payload) => Snapshot::Present { raw, payload },
            Err(e) => Snapshot::Invalid(e),
        },
        // A missing file is "not running", not a hard IO failure the
        // operator must debug. Any OTHER read error (permissions, a
        // directory squatting on the path, ...) is a real failure and
        // propagates with its cause.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Snapshot::Absent,
        Err(e) => {
            return Err(anyhow::Error::new(e)
                .context(format!("failed to read status file: {}", path.display())));
        }
    };
    Ok(RoleStatus {
        role,
        path,
        liveness,
        snapshot,
    })
}

fn parse_payload(role: Role, body: &str, path: &Path) -> Result<StatusPayload> {
    Ok(match role {
        Role::Serve => StatusPayload::Serve(parse_status_file(body, path)?),
        Role::Access => StatusPayload::Access(parse_status_file(body, path)?),
    })
}

/// Parse a status file body as `T`, with the path in the error context.
fn parse_status_file<T: DeserializeOwned>(body: &str, path: &Path) -> Result<T> {
    serde_json::from_str(body).with_context(|| format!("invalid status file: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::status::StatusFile;

    fn serve_file_json(pid: u32) -> String {
        serde_json::to_string(&StatusFile {
            node_id: "abc123".to_string(),
            home_relay: None,
            pid,
            started_at: 1_700_000_000,
            services: Vec::new(),
            connections: Vec::new(),
        })
        .unwrap()
    }

    #[test]
    fn read_status_absent_and_not_running() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve-status.json");
        let status = read_role_status_at(Role::Serve, path, Liveness::NotRunning).unwrap();
        assert!(matches!(status.snapshot, Snapshot::Absent));
        assert_eq!(status.freshness(), Freshness::AbsentNotRunning);
    }

    #[test]
    fn read_status_stale_pid_includes_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve-status.json");
        std::fs::write(&path, serve_file_json(4_000_000)).unwrap();
        let status = read_role_status_at(Role::Serve, path, Liveness::NotRunning).unwrap();
        assert!(matches!(status.snapshot, Snapshot::Present { .. }));
        assert_eq!(status.freshness(), Freshness::StalePid { live_pid: None });
    }

    #[test]
    fn read_status_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve-status.json");
        read_role_status_at(Role::Serve, path, Liveness::NotRunning).unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn read_status_invalid_file_is_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve-status.json");
        std::fs::write(&path, "{ not json").unwrap();
        let status = read_role_status_at(Role::Serve, path, Liveness::NotRunning).unwrap();
        assert!(matches!(status.snapshot, Snapshot::Invalid(_)));
        assert_eq!(status.freshness(), Freshness::Unreadable);
    }
}
