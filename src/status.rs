//! Status-file writer for the serve role.
//!
//! Implements T-13. After the serve endpoint is up, we write a JSON snapshot
//! (`node_id`, `home_relay`, `pid`, `started_at`, `services`) to the OS state
//! directory so operators and tooling can inspect a running node. Only the
//! serve role writes a status file.
//!
//! ## Path
//!
//! `<state_dir>/iroh-tunnel/status.json`, where `state_dir` is
//! [`dirs::state_dir`] (`~/.local/state` on Linux, `~/Library/Application
//! Support` on macOS; falls back to [`dirs::data_dir`] on platforms where
//! `state_dir` is `None`). The spec sample hard-coded `~/.local/state`, which is
//! Linux/XDG-only; `dirs::state_dir` is the cross-platform equivalent.
//!
//! Based on Page 06 v5 §4 (status file schema).

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Serialize;

/// Top-level status snapshot written to disk by the serve role.
#[derive(Debug, Serialize)]
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
}

/// One row per configured service in the status file.
#[derive(Debug, Serialize)]
pub struct ServiceStatus {
    pub name: String,
    pub protocol: String,
    /// `host:port` of the local service being tunneled.
    pub local_addr: String,
    /// Always 0 in the PoC — connection tracking lands with the production
    /// drain work (see the T-08/T-09 follow-ups).
    pub active_connections: u64,
}

impl StatusFile {
    /// Write the status file to the OS state directory, creating it if needed.
    /// Returns the path written.
    ///
    /// Atomic: the JSON is written to a sibling temp file and renamed into
    /// place, so a concurrent reader never observes a half-written file.
    pub fn save(&self) -> Result<PathBuf> {
        let path = status_file_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create state dir: {}", parent.display()))?;
        }
        let content = serde_json::to_string_pretty(self).context("failed to encode status JSON")?;

        // Write to a temp file in the same directory, then rename — atomic on
        // POSIX, and on Windows for same-volume same-directory renames.
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, &content)
            .with_context(|| format!("failed to write status file: {}", temp.display()))?;
        std::fs::rename(&temp, &path)
            .with_context(|| format!("failed to finalize status file: {}", path.display()))?;
        Ok(path)
    }
}

/// Resolve the status file path under the OS state directory.
fn status_file_path() -> Result<PathBuf> {
    // state_dir() is None on Windows; fall back to data_dir() so the file still
    // lands somewhere sensible rather than erroring.
    let base = dirs::state_dir()
        .or_else(dirs::data_dir)
        .context("could not determine state directory")?;
    Ok(base.join("iroh-tunnel").join("status.json"))
}
