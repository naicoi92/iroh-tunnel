//! Read-only classification of a role's status file.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;

use super::schema::StatusPayload;
use super::types::{RoleStatus, Snapshot};
use super::writer::StatusWriter;
use crate::ownership::{self, Liveness};
use crate::role::Role;

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
    use super::super::schema::StatusFile;
    use super::super::types::Freshness;
    use super::*;

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
