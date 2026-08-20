//! Status-file writer for the serve role.
//!
//! Implements T-13. After the serve endpoint is up, we write a JSON snapshot
//! (`node_id`, `home_relay`, `pid`, `started_at`, `services`,
//! `connections`) to the OS state directory so operators and tooling can
//! inspect a running node. Only the serve role writes a status file.
//!
//! ## Path
//!
//! `<state_dir>/iroh-tunnel/serve-status.json`, where `state_dir` is
//! [`dirs::state_dir`] (`~/.local/state` on Linux, `~/Library/Application
//! Support` on macOS; falls back to [`dirs::data_dir`] on platforms where
//! `state_dir` is `None`). The spec sample hard-coded `~/.local/state`, which
//! is Linux/XDG-only; `dirs::state_dir` is the cross-platform equivalent.
//! The file name is role-scoped (`serve-status.json`, renamed from
//! `status.json` in 0.4.0) so a future `access-status.json` can live beside
//! it without ambiguity.
//!
//! ## Testing seam
//!
//! [`StatusFile::save`] resolves the OS state dir itself, which makes it
//! untestable without touching the real filesystem. Two mechanisms make it
//! hermetic:
//!
//! - [`StatusFile::save_to`] takes the destination dir explicitly so tests
//!   can inject a `tempfile` tempdir; `save()` is a one-line delegate.
//! - `status_file_path()` honors the `IROH_TUNNEL_STATE_DIR` environment
//!   variable (an advanced/testing override of the *entire* directory the
//!   file lands in — the `iroh-tunnel` subpath is not appended). Integration
//!   tests use it to point a serve instance at a tempdir.
//!
//! Based on Page 06 v5 §4 (status file schema).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

/// File name of the serve status file (role-scoped).
const STATUS_FILE_NAME: &str = "serve-status.json";

/// Top-level status snapshot written to disk by the serve role.
#[derive(Debug, PartialEq, Serialize)]
pub struct StatusFile {
    /// The serve node's public key (hex), as printed by `serve run`.
    pub node_id: String,
    /// The Iroh home relay URL the node registered with, if any.
    pub home_relay: Option<String>,
    /// OS process id of the running serve instance.
    pub pid: u32,
    /// Unix epoch seconds at which the serve instance started.
    pub started_at: u64,
    /// The services this node is exposing.
    pub services: Vec<ServiceStatus>,
    /// Peers currently connected to this serve node (issue #57).
    pub connections: Vec<PeerConnectionStatus>,
}

/// One row per configured service in the status file.
#[derive(Debug, PartialEq, Serialize)]
pub struct ServiceStatus {
    pub name: String,
    pub protocol: String,
    /// `host:port` of the local service being tunneled.
    pub local_addr: String,
    /// Always 0 in the PoC — connection tracking lands with the production
    /// drain work (see the T-08/T-09 follow-ups).
    pub active_connections: u64,
}

/// One currently-connected remote peer in the status file (issue #57).
///
/// Built by the serve role's flush task from [`crate::conn_path`] snapshots
/// plus its own per-peer service tracking.
#[derive(Debug, PartialEq, Serialize)]
pub struct PeerConnectionStatus {
    /// The remote peer's endpoint id (full node id string).
    pub peer: String,
    /// Service names this peer is connected to (from the connection's ALPN,
    /// merged across all of the peer's connections).
    pub services: Vec<String>,
    /// Transports iroh currently uses or knows for this peer — see
    /// [`crate::conn_path`] for the relay/direct semantics.
    pub transports: Vec<crate::conn_path::TransportStatus>,
    /// Local UDP socket *candidates* of the serve endpoint (endpoint-wide,
    /// not a per-transport local address).
    pub local_bound_addrs: Vec<String>,
}

impl StatusFile {
    /// Write the status file under the OS state directory.
    ///
    /// Convenience entry point for production callers — resolves
    /// [`status_file_path`] and delegates to [`Self::save_to`]. Returns the
    /// path written.
    pub fn save(&self) -> Result<PathBuf> {
        // Compute the parent dir (the OS state dir + "iroh-tunnel"), so save_to
        // can stay dir-focused and create it if missing.
        let path = status_file_path()?;
        let dir = path
            .parent()
            .context("status file path has no parent directory")?
            .to_path_buf();
        self.save_to(&dir).map(|_| path)
    }

    /// Write the status file as `<dir>/serve-status.json`, creating `dir` if
    /// needed.
    ///
    /// Atomic: the JSON is written to a sibling temp file
    /// (`serve-status.json.tmp`) and renamed into place, so a concurrent
    /// reader never observes a half-written file. Returns the path written.
    ///
    /// This is the testable core — production callers use [`Self::save`];
    /// tests inject a `tempfile::tempdir()` here.
    pub fn save_to(&self, dir: &Path) -> Result<PathBuf> {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("failed to create status dir: {}", dir.display()))?;
        let path = dir.join(STATUS_FILE_NAME);
        let content = serde_json::to_string_pretty(self).context("failed to encode status JSON")?;

        // Write to a temp file in the same directory, then rename — atomic on
        // POSIX, and on Windows for same-volume same-directory renames.
        let temp = dir.join(format!("{STATUS_FILE_NAME}.tmp"));
        std::fs::write(&temp, &content)
            .with_context(|| format!("failed to write status file: {}", temp.display()))?;
        // If rename fails, the temp file is stale — clean it up so repeated
        // failures don't accumulate `serve-status.json.tmp` on disk. The
        // original rename error is still what we return; a cleanup failure is
        // best-effort.
        if let Err(e) = std::fs::rename(&temp, &path) {
            let _ = std::fs::remove_file(&temp);
            return Err(e)
                .with_context(|| format!("failed to finalize status file: {}", path.display()));
        }
        Ok(path)
    }
}

/// Format a `host:port` pair for the machine-readable status file, bracketing
/// IPv6 literals (`[::1]:8080`) so the result is unambiguous. Plain IPv4
/// addresses and hostnames are left as `host:port`.
///
/// Co-located with [`ServiceStatus::local_addr`] — the field that consumes it —
/// so the format is defined next to the schema that depends on it.
pub(crate) fn format_local_addr(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// Resolve the status file path under the OS state directory.
///
/// The `IROH_TUNNEL_STATE_DIR` environment variable, when set, replaces the
/// resolved base dir *entirely* — the file lands directly in it (no
/// `iroh-tunnel` subpath). This is an advanced/testing seam: it lets
/// integration tests point a serve instance at a tempdir without touching the
/// real state dir, and lets packaging relocate the file.
fn status_file_path() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("IROH_TUNNEL_STATE_DIR") {
        return Ok(PathBuf::from(dir).join(STATUS_FILE_NAME));
    }
    // state_dir() is None on Windows; fall back to data_dir() so the file still
    // lands somewhere sensible rather than erroring.
    let base = dirs::state_dir()
        .or_else(dirs::data_dir)
        .context("could not determine state directory")?;
    Ok(base.join("iroh-tunnel").join(STATUS_FILE_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_status() -> StatusFile {
        StatusFile {
            node_id: "abc123".to_string(),
            home_relay: Some("https://relay.example/".to_string()),
            pid: 42,
            started_at: 1_700_000_000,
            services: vec![ServiceStatus {
                name: "echo".to_string(),
                protocol: "tcp".to_string(),
                local_addr: "127.0.0.1:8080".to_string(),
                active_connections: 0,
            }],
            connections: vec![PeerConnectionStatus {
                peer: "peer456".to_string(),
                services: vec!["echo".to_string()],
                transports: vec![crate::conn_path::TransportStatus {
                    kind: "relay".to_string(),
                    addr: "https://relay.example/".to_string(),
                    active: true,
                }],
                local_bound_addrs: vec!["0.0.0.0:52110".to_string()],
            }],
        }
    }

    #[test]
    fn save_to_writes_json_file_into_injected_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let status = sample_status();
        let written = status.save_to(tmp.path()).unwrap();

        // Path is `<tmp>/serve-status.json`.
        assert_eq!(written, tmp.path().join("serve-status.json"));
        assert!(written.exists(), "status file should exist after save_to");

        // Contents are valid JSON with the expected shape.
        let body = std::fs::read_to_string(&written).unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(&body).expect("status file should be valid JSON");
        assert_eq!(parsed["node_id"], "abc123");
        assert_eq!(parsed["pid"], 42);
        assert_eq!(parsed["services"][0]["name"], "echo");
    }

    #[test]
    fn connections_array_serializes_with_documented_schema() {
        // Issue #57: the `connections` array must carry peer, services,
        // transports (kind/addr/active) and local_bound_addrs, under exactly
        // those field names.
        let status = sample_status();
        let json = serde_json::to_value(&status).unwrap();

        let conns = json["connections"].as_array().unwrap();
        assert_eq!(conns.len(), 1);
        assert_eq!(conns[0]["peer"], "peer456");
        assert_eq!(conns[0]["services"], serde_json::json!(["echo"]));
        assert_eq!(conns[0]["transports"][0]["kind"], "relay");
        assert_eq!(conns[0]["transports"][0]["addr"], "https://relay.example/");
        assert_eq!(conns[0]["transports"][0]["active"], true);
        assert_eq!(
            conns[0]["local_bound_addrs"],
            serde_json::json!(["0.0.0.0:52110"])
        );

        // On disk the same schema must round-trip through save_to.
        let tmp = tempfile::tempdir().unwrap();
        status.save_to(tmp.path()).unwrap();
        let body = std::fs::read_to_string(tmp.path().join("serve-status.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["connections"][0]["peer"], "peer456");
        assert_eq!(
            parsed["connections"][0]["services"],
            serde_json::json!(["echo"])
        );
    }

    #[test]
    fn save_to_creates_missing_parent_dir() {
        // A nested path that doesn't exist yet — save_to must mkdir -p it.
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("a").join("b").join("c");
        let status = sample_status();

        let written = status.save_to(&nested).unwrap();
        assert!(written.exists(), "status file should exist after save_to");
        assert!(nested.is_dir(), "parent dir should have been created");
    }

    #[test]
    fn save_to_uses_atomic_rename_temp_file_gone() {
        // After a successful save_to, the sibling `serve-status.json.tmp`
        // must not linger — atomic rename removes it.
        let tmp = tempfile::tempdir().unwrap();
        let status = sample_status();
        status.save_to(tmp.path()).unwrap();

        let temp = tmp.path().join("serve-status.json.tmp");
        assert!(!temp.exists(), "temp file should be gone after rename");
    }

    #[test]
    fn save_to_overwrites_existing_status() {
        // A second save_to on the same dir replaces the previous file
        // atomically — no stale contents.
        let tmp = tempfile::tempdir().unwrap();
        let mut status = sample_status();
        status.save_to(tmp.path()).unwrap();

        status.node_id = "xyz789".to_string();
        status.save_to(tmp.path()).unwrap();

        let body = std::fs::read_to_string(tmp.path().join("serve-status.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["node_id"], "xyz789");
    }

    #[test]
    fn format_local_addr_brackets_ipv6_literal() {
        // IPv6 addresses contain ':' and must be bracketed to be unambiguous.
        assert_eq!(format_local_addr("::1", 8080), "[::1]:8080");
        assert_eq!(format_local_addr("fe80::1", 443), "[fe80::1]:443");
    }

    #[test]
    fn format_local_addr_passes_ipv4_through_unchanged() {
        assert_eq!(format_local_addr("127.0.0.1", 8080), "127.0.0.1:8080");
        assert_eq!(format_local_addr("0.0.0.0", 9000), "0.0.0.0:9000");
    }

    #[test]
    fn format_local_addr_passes_hostname_through_unchanged() {
        assert_eq!(format_local_addr("localhost", 8080), "localhost:8080");
        assert_eq!(format_local_addr("db.internal", 5432), "db.internal:5432");
    }

    #[test]
    fn save_to_cleans_up_temp_file_when_rename_fails() {
        // Drive rename failure by making the destination path unwritable:
        // create a *directory* at serve-status.json's slot so the rename
        // target is occupied by an incompatible entry. This forces rename()
        // to fail on most platforms; if it doesn't fail, the test asserts
        // the post-condition (no stale temp) only when the rename actually
        // failed, so it can't false-positive on a platform where rename
        // overwrites a directory.
        let tmp = tempfile::tempdir().unwrap();
        // Block the destination: a directory at serve-status.json.
        std::fs::create_dir_all(tmp.path().join("serve-status.json")).unwrap();

        let status = sample_status();
        let res = status.save_to(tmp.path());

        if res.is_err() {
            // The load-bearing assertion: the temp file must not linger after
            // a rename failure.
            assert!(
                !tmp.path().join("serve-status.json.tmp").exists(),
                "temp file leaked after rename failure"
            );
        }
        // If rename somehow succeeded (platform-specific), the temp file is
        // gone anyway because the rename consumed it — no extra assertion.
    }

    #[test]
    fn save_honors_iroh_tunnel_state_dir_override() {
        // The env seam: with IROH_TUNNEL_STATE_DIR set, `save()` must land
        // the file directly in that dir (no `iroh-tunnel` subpath) instead
        // of the real OS state dir. Env mutation is process-global; no other
        // unit test reads this variable, so the override is safe here.
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("IROH_TUNNEL_STATE_DIR", tmp.path());

        let written = sample_status().save().unwrap();

        std::env::remove_var("IROH_TUNNEL_STATE_DIR");
        assert_eq!(written, tmp.path().join("serve-status.json"));
        assert!(
            written.exists(),
            "status file should exist under the override dir"
        );
    }
}
