//! Status-file machinery shared by both roles (serve T-13, access #59).
//!
//! Each `run` writes a JSON snapshot to the OS state directory so operators
//! and tooling can inspect a running node: serve writes
//! `serve-status.json` (`node_id`, `home_relay`, `pid`, `started_at`,
//! `services`, `connections`), access writes `access-status.json`
//! (`node_id`, `pid`, `started_at`, per-service rows). The write mechanics
//! — atomic save, `IROH_TUNNEL_STATE_DIR` override, change-detect cadence —
//! are role-agnostic and live in [`StatusWriter`]; the schemas are
//! role-specific and live below it.
//!
//! ## Path
//!
//! `<state dir>/<role>-status.json`, where the state dir is
//! [`crate::paths::default_state_dir`] (`~/.local/state/iroh-tunnel` on
//! macOS/Linux, ignoring XDG; `LocalAppData/iroh-tunnel/state` on Windows).
//! The file names are role-scoped (`serve-status.json`, renamed from
//! `status.json` in 0.4.0; `access-status.json` since the #59 work) so both
//! files live beside each other in one directory without ambiguity.
//!
//! ## Testing seam
//!
//! [`StatusWriter::save`] resolves the OS state dir itself, which makes it
//! untestable without touching the real filesystem. Three mechanisms make it
//! hermetic:
//!
//! - [`StatusWriter::save_to`] takes the destination dir explicitly so tests
//!   can inject a `tempfile` tempdir; `save()` is a thin delegate.
//! - [`StatusWriter::save_with_state_dir`] collapses the roles' common
//!   "injected dir or env default" choice into one call (their
//!   `run_with_shutdown_with_state_dir` entry points feed it).
//! - [`StatusWriter::path_with`] honors the `IROH_TUNNEL_STATE_DIR`
//!   environment variable (an advanced/testing override of the *entire*
//!   directory the file lands in — the `iroh-tunnel` subpath is not
//!   appended; an empty value is treated as unset). The resolver is a pure
//!   function of the override value, so unit tests cover it without
//!   mutating process-global environment; integration tests set the real
//!   variable before the tokio runtime exists.
//!
//! Based on Page 06 v5 §4 (status file schema).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use iroh_tunnel_engine::observation as obs;

/// File name of the serve status file (role-scoped).
const SERVE_STATUS_FILE_NAME: &str = "serve-status.json";

/// File name of the access status file (role-scoped, issue #59) — lives in
/// the same state directory as the serve file.
const ACCESS_STATUS_FILE_NAME: &str = "access-status.json";

/// Pre-rename serve file name, removed best-effort on the first successful
/// serve save of the renamed file so upgraded nodes don't leave a stale
/// snapshot that tooling still pointed at the old name would read silently.
const LEGACY_STATUS_FILE_NAME: &str = "status.json";

/// Environment variable overriding the status files' directory entirely
/// (advanced/testing seam; empty is treated as unset). Shared by both
/// roles — one directory, two role-scoped file names.
const ENV_STATE_DIR: &str = "IROH_TUNNEL_STATE_DIR";

/// Top-level status snapshot written to disk by the serve role.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServiceStatus {
    pub name: String,
    pub protocol: String,
    /// `host:port` of the local service being tunneled.
    pub local_addr: String,
    /// Since 0.2.0 this counts *active streams* (in-flight pipes), not iroh
    /// connections — one multiplexed connection carries many channels.
    pub active_connections: u64,
    /// UDP flow diagnostics; present only for a UDP service.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub udp: Option<UdpStatus>,
}

/// UDP flow diagnostics of one service (UDP-06): live flows, the current
/// datagram limit and a drop counter per reason. Counters only grow while the
/// role runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UdpStatus {
    /// Flows that currently carry traffic.
    pub active_flows: u64,
    /// Expired flows whose upstream socket is held as a sink (Serve only).
    pub quarantined_flows: u64,
    /// Largest client payload the connection carries now; `null` until a
    /// connection exists. Changes with the path.
    pub datagram_limit: Option<u64>,
    pub drops: UdpDrops,
}

/// Datagrams dropped, by reason (contract failure table).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UdpDrops {
    /// Larger than the datagram limit; never delivered in part.
    pub oversize: u64,
    /// Shorter than the 4-byte flow header.
    pub malformed: u64,
    /// For a flow that is not live and cannot be admitted.
    pub unknown_flow: u64,
    /// The upstream socket could not send.
    pub upstream_error: u64,
    /// A flow cap was reached, or the upstream socket could not be opened.
    pub cap: u64,
    /// A flow's queue was full; the newest datagram was dropped.
    pub queue_full: u64,
    /// Datagrams became unusable on the connection mid-session.
    pub limit_shrink: u64,
    /// A late reply reached a quarantined socket.
    pub quarantine_reply: u64,
    /// A reply from a source other than the target, where the platform
    /// exposes it.
    pub foreign_source: u64,
}

/// One currently-connected remote peer in the status file (issue #57).
///
/// The path fields (`peer`, `transports`, `local_bound_addrs`) are flattened
/// in from a [`PeerPathReport`] so the two schemas cannot
/// drift apart; the serve role adds only the per-peer `services` merge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PeerConnectionStatus {
    /// Connection-path snapshot for this peer (peer id, transports,
    /// endpoint-wide local UDP candidates), flattened into this row.
    #[serde(flatten)]
    pub path: PeerPathReport,
    /// Service names this peer is connected to (from the connection's ALPN,
    /// merged across all of the peer's connections).
    pub services: Vec<String>,
}

/// How one remote peer is currently reachable, for status files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PeerPathReport {
    /// The remote peer's endpoint id (full node id string).
    pub peer: String,
    /// Transports iroh knows for this peer, active ones first.
    pub transports: Vec<TransportStatus>,
    /// Local UDP socket candidates of *our* endpoint (endpoint-wide, not a
    /// per-transport local address).
    pub local_bound_addrs: Vec<String>,
}

/// The kind of network path a transport uses, serialized lowercase
/// (`"relay"` / `"direct"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransportKind {
    /// Via a relay server.
    Relay,
    /// Direct IP path.
    Direct,
}

impl std::fmt::Display for TransportKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Matches the serde lowercase form.
        f.write_str(match self {
            TransportKind::Relay => "relay",
            TransportKind::Direct => "direct",
        })
    }
}

/// One transport path to a remote peer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransportStatus {
    /// `"relay"` (via a relay server URL) or `"direct"` (IP path).
    pub kind: TransportKind,
    /// The relay URL or `SocketAddr` (`host:port`, IPv6 bracketed) in use.
    pub addr: String,
    /// Whether iroh is actively sending on this transport at query time.
    pub active: bool,
}

impl From<&obs::TransportObservation> for TransportStatus {
    fn from(t: &obs::TransportObservation) -> Self {
        Self {
            kind: match t.kind {
                obs::TransportKind::Relay => TransportKind::Relay,
                obs::TransportKind::Direct => TransportKind::Direct,
            },
            addr: t.addr.clone(),
            active: t.active,
        }
    }
}

impl From<&obs::PeerPathObservation> for PeerPathReport {
    fn from(p: &obs::PeerPathObservation) -> Self {
        Self {
            peer: p.peer.clone(),
            transports: p.transports.iter().map(TransportStatus::from).collect(),
            local_bound_addrs: p.local_bound_addrs.clone(),
        }
    }
}

impl From<&obs::UdpObservation> for UdpStatus {
    fn from(u: &obs::UdpObservation) -> Self {
        Self {
            active_flows: u.active_flows,
            quarantined_flows: u.quarantined_flows,
            datagram_limit: u.datagram_limit,
            drops: UdpDrops {
                oversize: u.drops.oversize,
                malformed: u.drops.malformed,
                unknown_flow: u.drops.unknown_flow,
                upstream_error: u.drops.upstream_error,
                cap: u.drops.cap,
                queue_full: u.drops.queue_full,
                limit_shrink: u.drops.limit_shrink,
                quarantine_reply: u.drops.quarantine_reply,
                foreign_source: u.drops.foreign_source,
            },
        }
    }
}

impl From<&obs::ServeObservation> for StatusFile {
    fn from(o: &obs::ServeObservation) -> Self {
        Self {
            node_id: o.node_id.clone(),
            home_relay: o.home_relay.clone(),
            pid: o.pid,
            started_at: o.started_at,
            services: o
                .services
                .iter()
                .map(|s| ServiceStatus {
                    name: s.name.clone(),
                    protocol: s.protocol.clone(),
                    local_addr: s.local_addr.clone(),
                    active_connections: s.active_connections,
                    udp: s.udp.as_ref().map(UdpStatus::from),
                })
                .collect(),
            connections: o
                .connections
                .iter()
                .map(|c| PeerConnectionStatus {
                    path: PeerPathReport::from(&c.path),
                    services: c.services.clone(),
                })
                .collect(),
        }
    }
}

impl From<&obs::ServeObservation> for StatusPayload {
    fn from(o: &obs::ServeObservation) -> Self {
        Self::Serve(StatusFile::from(o))
    }
}

impl From<&obs::AccessObservation> for StatusPayload {
    fn from(o: &obs::AccessObservation) -> Self {
        Self::Access(AccessStatusFile::from(o))
    }
}

impl From<&obs::AccessObservation> for AccessStatusFile {
    fn from(o: &obs::AccessObservation) -> Self {
        Self {
            node_id: o.node_id.clone(),
            pid: o.pid,
            started_at: o.started_at,
            services: o
                .services
                .iter()
                .map(|s| AccessServiceStatus {
                    name: s.name.clone(),
                    listen_addr: s.listen_addr.clone(),
                    peer: s.peer.clone(),
                    transports: s.transports.iter().map(TransportStatus::from).collect(),
                    local_bound_addrs: s.local_bound_addrs.clone(),
                    udp: s.udp.as_ref().map(UdpStatus::from),
                })
                .collect(),
        }
    }
}

/// Top-level status snapshot written to disk by the access role (issue #59).
///
/// Mirror of [`StatusFile`]'s role-scoped shape. There is no `home_relay`
/// field: the access node registers with relays only to dial out, and which
/// relay carried a given dial is already answered per service by that
/// service's `transports`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccessStatusFile {
    /// The access node's own public key (hex), as printed by `access run`.
    pub node_id: String,
    /// OS process id of the running access instance.
    pub pid: u32,
    /// Unix epoch seconds at which the access instance started.
    pub started_at: u64,
    /// One row per configured service, in config order.
    pub services: Vec<AccessServiceStatus>,
}

/// One configured service in the access status file (issue #59).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccessServiceStatus {
    pub name: String,
    /// `host:port` local clients connect to.
    pub listen_addr: String,
    /// The serve peer this service is configured to dial (full node id) —
    /// present from the config, so it shows before the first connection.
    pub peer: String,
    /// Live transports of the service's shared multiplexed connection,
    /// queried fresh at each flush. Empty while the service has no live
    /// connection.
    pub transports: Vec<TransportStatus>,
    /// This endpoint's local UDP socket *candidates* (endpoint-wide, not a
    /// per-transport local address — same semantics as the serve file).
    pub local_bound_addrs: Vec<String>,
    /// UDP flow diagnostics; present only for a UDP service.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub udp: Option<UdpStatus>,
}

/// The typed snapshot a role saves — re-tying each schema to its role at
/// the save boundary.
///
/// [`StatusWriter`] is role-tagged and its save methods take this enum;
/// [`StatusWriter::save_to`] rejects a mismatched writer/payload pair with
/// a runtime role check that errors in every build profile, instead of
/// silently writing the wrong schema under the right file name.
#[derive(Debug, Clone, PartialEq)]
pub enum StatusPayload {
    Serve(StatusFile),
    Access(AccessStatusFile),
}

impl StatusPayload {
    /// The role whose schema this payload carries.
    pub fn role(&self) -> StatusRole {
        match self {
            StatusPayload::Serve(_) => StatusRole::Serve,
            StatusPayload::Access(_) => StatusRole::Access,
        }
    }
}

/// Which role a status file belongs to.
///
/// The two roles share every write mechanic (atomic save, env override,
/// change-detect cadence) and differ only in the file name — this enum is
/// the entire role-specific surface of the writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Serve,
    Access,
}

/// Former name of [`Role`]; removed in Task 20.
pub type StatusRole = Role;

impl Role {
    /// File name of this role's status file.
    pub fn file_name(self) -> &'static str {
        match self {
            Role::Serve => SERVE_STATUS_FILE_NAME,
            Role::Access => ACCESS_STATUS_FILE_NAME,
        }
    }

    /// The role's CLI name (`"serve"` / `"access"`).
    pub fn name(self) -> &'static str {
        match self {
            Role::Serve => "serve",
            Role::Access => "access",
        }
    }
}

/// Atomic writer for one role's status file.
///
/// Owns the parts shared by both roles so they cannot drift: the
/// temp+fsync+rename save, the `IROH_TUNNEL_STATE_DIR` override, and the
/// injected-state-dir testing seam. Role differences reduce to the file
/// name via [`StatusWriter::serve`] / [`StatusWriter::access`].
#[derive(Debug, Clone, Copy)]
pub struct StatusWriter(StatusRole);

/// Monotonic per-process counter making each save's temp file name unique
/// (see [`StatusWriter::save_to`]).
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

impl StatusWriter {
    /// Writer targeting `role`'s status file.
    pub const fn new(role: StatusRole) -> Self {
        Self(role)
    }

    /// Writer for the serve role (`serve-status.json`).
    pub const fn serve() -> Self {
        Self(StatusRole::Serve)
    }

    /// Writer for the access role (`access-status.json`, issue #59).
    pub const fn access() -> Self {
        Self(StatusRole::Access)
    }

    /// The role this writer targets.
    pub fn role(&self) -> StatusRole {
        self.0
    }

    /// The file name this writer saves under.
    pub fn file_name(&self) -> &'static str {
        self.0.file_name()
    }

    /// Resolve this role's status file path under the OS state directory.
    pub fn path(&self) -> Result<PathBuf> {
        self.path_with(std::env::var_os(ENV_STATE_DIR).as_deref())
    }

    /// Pure core of [`StatusWriter::path`]: the path as a function of an
    /// optional env-override value, kept separate so tests never mutate
    /// process-global state.
    ///
    /// The `IROH_TUNNEL_STATE_DIR` environment variable, when set to a
    /// non-empty value, replaces the resolved base dir *entirely* — the file
    /// lands directly in it (no `iroh-tunnel` subpath). This is an
    /// advanced/testing seam: it lets integration tests point a role
    /// instance at a tempdir without touching the real state dir, and lets
    /// packaging relocate the file. An empty value is treated as unset
    /// (standard env-var semantics) so a stray `IROH_TUNNEL_STATE_DIR=`
    /// cannot make the path relative to the current working directory.
    pub fn path_with(&self, state_dir_override: Option<&OsStr>) -> Result<PathBuf> {
        if let Some(dir) = state_dir_override.filter(|dir| !dir.is_empty()) {
            // Absolutize so a relative override (`IROH_TUNNEL_STATE_DIR=.`)
            // is resolved against the CWD at resolution time. Note this runs
            // per save — a process that changes its CWD would change the
            // target dir; pin the resolved path once if that ever becomes a
            // concern.
            let dir = std::path::absolute(dir).with_context(|| {
                format!("invalid {ENV_STATE_DIR}: {}", Path::new(dir).display())
            })?;
            return Ok(dir.join(self.file_name()));
        }
        Ok(crate::paths::default_state_dir()?.join(self.file_name()))
    }

    /// Write `value` under the env-aware default path for this role.
    ///
    /// Convenience entry point for production callers — resolves
    /// [`Self::path`] and delegates to [`Self::save_to`]. Returns the path
    /// written.
    pub fn save(&self, value: &StatusPayload) -> Result<PathBuf> {
        // Compute the parent dir (the OS state dir + "iroh-tunnel"), so
        // save_to can stay dir-focused and create it if missing.
        let path = self.path()?;
        let dir = path
            .parent()
            .context("status file path has no parent directory")?
            .to_path_buf();
        self.save_to(&dir, value).map(|_| path)
    }

    /// Write `value` honoring an optional injected state dir: `Some(dir)`
    /// writes `<dir>/<file_name>` directly; `None` uses [`Self::save`]
    /// (env-aware default path).
    ///
    /// The seam the roles' `run_with_shutdown_with_state_dir` entry points
    /// feed: production passes `None`, integration tests inject a tempdir
    /// without mutating process-global environment.
    pub fn save_with_state_dir(
        &self,
        state_dir: Option<&Path>,
        value: &StatusPayload,
    ) -> Result<PathBuf> {
        match state_dir {
            Some(dir) => self.save_to(dir, value),
            None => self.save(value),
        }
    }

    /// The path [`Self::save_with_state_dir`] writes to for this state-dir
    /// choice: `Some(dir)` → `<dir>/<file_name>`, `None` → the env-aware
    /// default.
    ///
    /// The read/remove counterpart of the save side, for callers that clean
    /// the file up on graceful shutdown — the flush task owns the writer,
    /// so the run loop resolves the same path up front instead of keeping a
    /// second handle to it.
    pub fn path_for_state_dir(&self, state_dir: Option<&Path>) -> Result<PathBuf> {
        match state_dir {
            Some(dir) => Ok(dir.join(self.file_name())),
            None => self.path(),
        }
    }

    /// Write `value` as `<dir>/<file_name>`, creating `dir` if needed.
    ///
    /// Atomic: the JSON is written to a sibling temp file
    /// (`<file_name>.tmp.<pid>.<n>`) and renamed into place, so a concurrent
    /// reader never observes a half-written file. The temp name is unique
    /// per save so two processes sharing a state dir cannot interleave their
    /// temp writes. Returns the path written.
    ///
    /// This is the testable core — production callers use [`Self::save`] or
    /// [`Self::save_with_state_dir`]; tests inject a `tempfile::tempdir()`
    /// here.
    pub fn save_to(&self, dir: &Path, value: &StatusPayload) -> Result<PathBuf> {
        // The payload enum re-ties schema to role at this boundary. A
        // mismatched writer/payload pair is a programmer error — bailed in
        // EVERY build profile (not just debug), because silently writing
        // one role's schema under the other's file name is a corrupting
        // failure no test fleet may paper over.
        if self.0 != value.role() {
            anyhow::bail!(
                "status writer role ({}) does not match payload role ({})",
                self.0.name(),
                value.role().name()
            );
        }
        let path = dir.join(self.file_name());
        // Serialize the INNER schema — the enum is a type-level tie, not a
        // JSON tag.
        let content = match value {
            StatusPayload::Serve(file) => serde_json::to_string_pretty(file),
            StatusPayload::Access(file) => serde_json::to_string_pretty(file),
        }
        .context("failed to encode status JSON")?;

        // Under a system service root writes into a user-owned directory:
        // go through a verified handle instead of paths (SM-04).
        #[cfg(unix)]
        if let Some(uid) = crate::service::trust::trusted_uid_from_env()? {
            let temp = format!(
                "{}.tmp.{}.{}",
                self.file_name(),
                std::process::id(),
                TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
            );
            crate::service::trust::publish(dir, uid, self.file_name(), &temp, content.as_bytes())?;
            // Best-effort, like the path-based branch below.
            let _ = crate::service::trust::remove_in(dir, uid, LEGACY_STATUS_FILE_NAME);
            return Ok(path);
        }
        std::fs::create_dir_all(dir)
            .with_context(|| format!("failed to create status dir: {}", dir.display()))?;
        // Write to a temp file in the same directory, then rename — atomic
        // on POSIX, and on Windows for same-volume same-directory renames.
        // `<name>.tmp.<pid>.<counter>` keeps every save's temp file unique
        // (two processes can share an injected state dir) while the rename
        // stays single-file atomic.
        let temp = dir.join(format!(
            "{}.tmp.{}.{}",
            self.file_name(),
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        // Write + fsync the temp file before renaming: the atomic-rename
        // guarantee below covers concurrent readers, and the fsync covers
        // durability — without it a crash right after the rename can leave
        // an empty destination. One save per change (throttled by the flush
        // loop), so the cost is negligible.
        let mut file = std::fs::File::create(&temp)
            .with_context(|| format!("failed to create status temp file: {}", temp.display()))?;
        std::io::Write::write_all(&mut file, content.as_bytes())
            .and_then(|()| file.sync_all())
            .with_context(|| format!("failed to write status file: {}", temp.display()))?;
        // If rename fails, the temp file is stale — clean it up so repeated
        // failures don't accumulate temp files on disk. The original rename
        // error is still what we return; a cleanup failure is best-effort.
        if let Err(e) = std::fs::rename(&temp, &path) {
            let _ = std::fs::remove_file(&temp);
            return Err(e)
                .with_context(|| format!("failed to finalize status file: {}", path.display()));
        }
        // Best-effort removal of the pre-rename legacy serve file, on every
        // successful save of EITHER role: both roles share one state
        // directory, so an access-only host upgrading from the pre-rename
        // era must clean the stale file too. Tooling pointed at the old
        // name would otherwise read a stale snapshot silently (worse than
        // an error); failure to remove is ignored.
        let _ = std::fs::remove_file(dir.join(LEGACY_STATUS_FILE_NAME));
        Ok(path)
    }
}

/// Just the `pid` field of a status file — the minimal probe
/// [`remove_own_status_file`] needs, so the ownership check works against
/// both roles' schemas (unknown fields are ignored by serde).
#[derive(Deserialize)]
struct StatusFilePid {
    pid: u32,
}

/// Remove `writer`'s status file on graceful shutdown — but only while it
/// is still OURS.
///
/// Two processes can share one state dir (the injected seam allows it, and
/// nothing else about the layout forbids it). The flush loop only rewrites
/// when the rendered snapshot CHANGES, so a co-resident idle instance would
/// not necessarily recreate its file after a blind removal — `<role>
/// status` would then report "not running" for a live role until its next
/// real change. The file's own `pid` field decides ownership: remove only
/// when it matches the current process. A missing file (never written),
/// unreadable file, or one that fails to parse is skipped — nothing to
/// clean, or not ours to judge.
pub(crate) async fn remove_own_status_file(writer: StatusWriter, state_dir: Option<&Path>) {
    let state_dir = state_dir.map(Path::to_path_buf);
    // Blocking filesystem work stays off the async workers (RS-07).
    if let Err(error) = tokio::task::spawn_blocking(move || {
        remove_own_status_file_blocking(writer, state_dir.as_deref())
    })
    .await
    {
        tracing::debug!("status cleanup task failed: {error}");
    }
}

/// Blocking twin of [`remove_own_status_file`] for the forced-exit path in
/// `src/main.rs`, where the async runtime is no longer driving cleanup.
/// Resolves the env-aware default directory, like a normal graceful stop.
pub fn remove_own_status_file_for_exit(role: StatusRole) {
    remove_own_status_file_blocking(StatusWriter::new(role), None);
}

fn remove_own_status_file_blocking(writer: StatusWriter, state_dir: Option<&Path>) {
    let Ok(path) = writer.path_for_state_dir(state_dir) else {
        return;
    };
    #[cfg(unix)]
    if let Ok(Some(uid)) = crate::service::trust::trusted_uid_from_env() {
        use crate::service::trust::{read_in, remove_in};
        let (Some(dir), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str()))
        else {
            return;
        };
        let ours = read_in(dir, uid, name).is_some_and(|body| {
            matches!(
                serde_json::from_str::<StatusFilePid>(&body),
                Ok(probe) if probe.pid == std::process::id()
            )
        });
        if ours {
            if let Err(e) = remove_in(dir, uid, name) {
                tracing::debug!("failed to remove status file: {e:#}");
            }
        }
        return;
    }
    let owns = match std::fs::read_to_string(&path) {
        Ok(body) => {
            matches!(
                serde_json::from_str::<StatusFilePid>(&body),
                Ok(probe) if probe.pid == std::process::id()
            )
        }
        Err(_) => false,
    };
    if !owns {
        return;
    }
    if let Err(e) = std::fs::remove_file(&path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::debug!("failed to remove status file: {e}");
        }
    }
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
                udp: None,
            }],
            connections: vec![PeerConnectionStatus {
                path: PeerPathReport {
                    peer: "peer456".to_string(),
                    transports: vec![TransportStatus {
                        kind: TransportKind::Relay,
                        addr: "https://relay.example/".to_string(),
                        active: true,
                    }],
                    local_bound_addrs: vec!["0.0.0.0:52110".to_string()],
                },
                services: vec!["echo".to_string()],
            }],
        }
    }

    fn sample_access_status() -> AccessStatusFile {
        AccessStatusFile {
            node_id: "acc789".to_string(),
            pid: 43,
            started_at: 1_700_000_001,
            services: vec![AccessServiceStatus {
                name: "echo".to_string(),
                listen_addr: "127.0.0.1:8080".to_string(),
                peer: "peer456".to_string(),
                transports: vec![TransportStatus {
                    kind: TransportKind::Direct,
                    addr: "192.168.1.10:52618".to_string(),
                    active: true,
                }],
                local_bound_addrs: vec!["0.0.0.0:52111".to_string()],
                udp: None,
            }],
        }
    }

    const SERVE_GOLDEN: &str = r#"{"node_id":"node-s","home_relay":"https://relay.example/","pid":42,"started_at":1700000000,"services":[{"name":"echo","protocol":"tcp","local_addr":"127.0.0.1:8080","active_connections":2},{"name":"dns","protocol":"udp","local_addr":"[::1]:5353","active_connections":0,"udp":{"active_flows":3,"quarantined_flows":1,"datagram_limit":1200,"drops":{"oversize":1,"malformed":2,"unknown_flow":3,"upstream_error":4,"cap":5,"queue_full":6,"limit_shrink":7,"quarantine_reply":8,"foreign_source":9}}}],"connections":[{"peer":"peer-a","transports":[{"kind":"relay","addr":"https://relay.example/","active":true},{"kind":"direct","addr":"10.0.0.2:1","active":false}],"local_bound_addrs":["0.0.0.0:1","[::]:2"],"services":["echo","dns"]},{"peer":"peer-b","transports":[],"local_bound_addrs":[],"services":["echo"]}]}"#;
    const ACCESS_GOLDEN: &str = r#"{"node_id":"node-a","pid":43,"started_at":1700000001,"services":[{"name":"echo","listen_addr":"127.0.0.1:9","peer":"peer-a","transports":[{"kind":"relay","addr":"https://relay.example/","active":true},{"kind":"direct","addr":"10.0.0.2:1","active":true}],"local_bound_addrs":["0.0.0.0:1"],"udp":{"active_flows":3,"quarantined_flows":1,"datagram_limit":1200,"drops":{"oversize":1,"malformed":2,"unknown_flow":3,"upstream_error":4,"cap":5,"queue_full":6,"limit_shrink":7,"quarantine_reply":8,"foreign_source":9}}},{"name":"idle","listen_addr":"127.0.0.1:10","peer":"peer-a","transports":[],"local_bound_addrs":[]}]}"#;

    fn udp_observation() -> obs::UdpObservation {
        obs::UdpObservation {
            active_flows: 3,
            quarantined_flows: 1,
            datagram_limit: Some(1200),
            drops: obs::UdpDropCounts {
                oversize: 1,
                malformed: 2,
                unknown_flow: 3,
                upstream_error: 4,
                cap: 5,
                queue_full: 6,
                limit_shrink: 7,
                quarantine_reply: 8,
                foreign_source: 9,
            },
        }
    }

    fn transport(kind: obs::TransportKind, addr: &str, active: bool) -> obs::TransportObservation {
        obs::TransportObservation {
            kind,
            addr: addr.to_string(),
            active,
        }
    }

    fn serve_observation() -> obs::ServeObservation {
        obs::ServeObservation {
            node_id: "node-s".into(),
            home_relay: Some("https://relay.example/".into()),
            pid: 42,
            started_at: 1_700_000_000,
            services: vec![
                obs::ServiceObservation {
                    name: "echo".into(),
                    protocol: "tcp".into(),
                    local_addr: "127.0.0.1:8080".into(),
                    active_connections: 2,
                    udp: None,
                },
                obs::ServiceObservation {
                    name: "dns".into(),
                    protocol: "udp".into(),
                    local_addr: "[::1]:5353".into(),
                    active_connections: 0,
                    udp: Some(udp_observation()),
                },
            ],
            connections: vec![
                obs::PeerConnectionObservation {
                    path: obs::PeerPathObservation {
                        peer: "peer-a".into(),
                        transports: vec![
                            transport(obs::TransportKind::Relay, "https://relay.example/", true),
                            transport(obs::TransportKind::Direct, "10.0.0.2:1", false),
                        ],
                        local_bound_addrs: vec!["0.0.0.0:1".into(), "[::]:2".into()],
                    },
                    services: vec!["echo".into(), "dns".into()],
                },
                obs::PeerConnectionObservation {
                    path: obs::PeerPathObservation {
                        peer: "peer-b".into(),
                        transports: vec![],
                        local_bound_addrs: vec![],
                    },
                    services: vec!["echo".into()],
                },
            ],
        }
    }

    fn access_observation() -> obs::AccessObservation {
        obs::AccessObservation {
            node_id: "node-a".into(),
            pid: 43,
            started_at: 1_700_000_001,
            services: vec![
                obs::AccessServiceObservation {
                    name: "echo".into(),
                    listen_addr: "127.0.0.1:9".into(),
                    peer: "peer-a".into(),
                    transports: vec![
                        transport(obs::TransportKind::Relay, "https://relay.example/", true),
                        transport(obs::TransportKind::Direct, "10.0.0.2:1", true),
                    ],
                    local_bound_addrs: vec!["0.0.0.0:1".into()],
                    udp: Some(udp_observation()),
                },
                obs::AccessServiceObservation {
                    name: "idle".into(),
                    listen_addr: "127.0.0.1:10".into(),
                    peer: "peer-a".into(),
                    transports: vec![],
                    local_bound_addrs: vec![],
                    udp: None,
                },
            ],
        }
    }

    #[test]
    fn observation_mapping_is_field_for_field() {
        // Goldens were captured from the pre-observation DTO serialization
        // of the same values: the on-disk schema must not move.
        assert_eq!(
            serde_json::to_string(&StatusFile::from(&serve_observation())).unwrap(),
            SERVE_GOLDEN
        );
        assert_eq!(
            serde_json::to_string(&AccessStatusFile::from(&access_observation())).unwrap(),
            ACCESS_GOLDEN
        );
    }

    #[test]
    fn transport_kind_serializes_lowercase() {
        assert_eq!(
            serde_json::to_value(TransportKind::Relay).unwrap(),
            serde_json::json!("relay")
        );
        assert_eq!(
            serde_json::to_value(TransportKind::Direct).unwrap(),
            serde_json::json!("direct")
        );
        let back: TransportKind = serde_json::from_value(serde_json::json!("direct")).unwrap();
        assert_eq!(back, TransportKind::Direct);
    }

    #[test]
    fn peer_path_report_serializes_with_documented_field_names() {
        let report = PeerPathReport {
            peer: "k7gp…nodeid".to_string(),
            transports: vec![TransportStatus {
                kind: TransportKind::Relay,
                addr: "https://use1-1.relay.iroh.network/".to_string(),
                active: true,
            }],
            local_bound_addrs: vec!["0.0.0.0:52110".to_string()],
        };
        let json = serde_json::to_value(&report).unwrap();

        assert_eq!(json["peer"], "k7gp…nodeid");
        assert_eq!(json["local_bound_addrs"][0], "0.0.0.0:52110");
        assert_eq!(json["transports"][0]["kind"], "relay");
        assert_eq!(
            json["transports"][0]["addr"],
            "https://use1-1.relay.iroh.network/"
        );
        assert_eq!(json["transports"][0]["active"], true);

        // Round-trips ( Deserialize is part of the schema contract for the
        // #58/#59 reuse).
        let back: PeerPathReport = serde_json::from_value(json).unwrap();
        assert_eq!(back, report);
    }

    #[test]
    fn transport_kind_display_matches_serde_form() {
        // The two spellings of a kind must stay identical: serde drives
        // the status-file schema, Display drives the log rendering.
        for kind in [TransportKind::Relay, TransportKind::Direct] {
            assert_eq!(
                serde_json::to_value(kind).unwrap(),
                serde_json::json!(kind.to_string())
            );
        }
    }

    #[test]
    fn observation_equality_ignores_nothing_rendered() {
        let base = serve_observation();
        assert_eq!(base, serve_observation());
        let mut flipped = serve_observation();
        flipped.connections[0].path.transports[0].active = false;
        assert_ne!(base, flipped);
        let mut access = access_observation();
        assert_eq!(access, access_observation());
        access.services[0].transports[0].active = false;
        assert_ne!(access, access_observation());
    }

    #[test]
    fn writer_selects_the_role_scoped_file_name() {
        assert_eq!(StatusWriter::serve().file_name(), "serve-status.json");
        assert_eq!(StatusWriter::access().file_name(), "access-status.json");
        assert_eq!(StatusWriter::serve().role(), StatusRole::Serve);
        assert_eq!(StatusWriter::access().role(), StatusRole::Access);
    }

    #[test]
    fn save_to_writes_serve_json_file_into_injected_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let status = sample_status();
        let written = StatusWriter::serve()
            .save_to(tmp.path(), &StatusPayload::Serve(status))
            .unwrap();

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
    fn save_to_writes_access_json_file_into_injected_dir() {
        // Issue #59: the access writer targets its own file name in the same
        // injected dir — and must NOT touch the serve file.
        let tmp = tempfile::tempdir().unwrap();
        let status = sample_access_status();
        let written = StatusWriter::access()
            .save_to(tmp.path(), &StatusPayload::Access(status))
            .unwrap();

        assert_eq!(written, tmp.path().join("access-status.json"));
        assert!(
            written.exists(),
            "access status file should exist after save_to"
        );
        assert!(
            !tmp.path().join("serve-status.json").exists(),
            "access save must not create the serve file"
        );

        let body = std::fs::read_to_string(&written).unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(&body).expect("status file should be valid JSON");
        assert_eq!(parsed["node_id"], "acc789");
        assert_eq!(parsed["services"][0]["peer"], "peer456");
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
        StatusWriter::serve()
            .save_to(tmp.path(), &StatusPayload::Serve(status))
            .unwrap();
        let body = std::fs::read_to_string(tmp.path().join("serve-status.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["connections"][0]["peer"], "peer456");
        assert_eq!(
            parsed["connections"][0]["services"],
            serde_json::json!(["echo"])
        );
    }

    #[test]
    fn access_status_serializes_with_documented_schema() {
        // Issue #59: one row per service — name, listen_addr, peer,
        // transports, local_bound_addrs — under exactly those field names,
        // and the file round-trips back into the typed struct (the CLI
        // render path depends on Deserialize).
        let json = serde_json::to_value(sample_access_status()).unwrap();
        assert_eq!(json["node_id"], "acc789");
        assert_eq!(json["pid"], 43);
        assert_eq!(json["started_at"], 1_700_000_001);
        let svc = &json["services"][0];
        assert_eq!(svc["name"], "echo");
        assert_eq!(svc["listen_addr"], "127.0.0.1:8080");
        assert_eq!(svc["peer"], "peer456");
        assert_eq!(svc["transports"][0]["kind"], "direct");
        assert_eq!(svc["transports"][0]["addr"], "192.168.1.10:52618");
        assert_eq!(svc["transports"][0]["active"], true);
        assert_eq!(
            svc["local_bound_addrs"],
            serde_json::json!(["0.0.0.0:52111"])
        );

        let body = serde_json::to_string(&json).unwrap();
        let roundtrip: AccessStatusFile = serde_json::from_str(&body).unwrap();
        assert_eq!(roundtrip, sample_access_status());
    }

    #[test]
    fn save_to_creates_missing_parent_dir() {
        // A nested path that doesn't exist yet — save_to must mkdir -p it.
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("a").join("b").join("c");
        let status = sample_status();

        let written = StatusWriter::serve()
            .save_to(&nested, &StatusPayload::Serve(status))
            .unwrap();
        assert!(written.exists(), "status file should exist after save_to");
        assert!(nested.is_dir(), "parent dir should have been created");
    }

    #[test]
    fn save_to_uses_atomic_rename_temp_file_gone() {
        // After a successful save_to, no sibling temp file may linger —
        // atomic rename consumes it. Temp names carry a unique
        // `.tmp.<pid>.<n>` suffix, so scan for the prefix.
        let tmp = tempfile::tempdir().unwrap();
        let status = sample_status();
        StatusWriter::serve()
            .save_to(tmp.path(), &StatusPayload::Serve(status))
            .unwrap();

        assert!(
            !leftover_temp_files(tmp.path(), "serve-status.json.tmp").exists(),
            "serve temp file leaked after rename"
        );
        StatusWriter::access()
            .save_to(tmp.path(), &StatusPayload::Access(sample_access_status()))
            .unwrap();
        assert!(
            !leftover_temp_files(tmp.path(), "access-status.json.tmp").exists(),
            "access temp file leaked after rename"
        );
    }

    #[test]
    fn save_to_overwrites_existing_status() {
        // A second save_to on the same dir replaces the previous file
        // atomically — no stale contents.
        let tmp = tempfile::tempdir().unwrap();
        let mut status = sample_status();
        StatusWriter::serve()
            .save_to(tmp.path(), &StatusPayload::Serve(status.clone()))
            .unwrap();

        status.node_id = "xyz789".to_string();
        StatusWriter::serve()
            .save_to(tmp.path(), &StatusPayload::Serve(status))
            .unwrap();

        let body = std::fs::read_to_string(tmp.path().join("serve-status.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["node_id"], "xyz789");
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
        let res = StatusWriter::serve().save_to(tmp.path(), &StatusPayload::Serve(status));

        if res.is_err() {
            // The load-bearing assertion: the temp file must not linger
            // after a rename failure.
            assert!(
                !leftover_temp_files(tmp.path(), "serve-status.json.tmp").exists(),
                "temp file leaked after rename failure"
            );
        }
        // If rename somehow succeeded (platform-specific), the temp file is
        // gone anyway because the rename consumed it — no extra assertion.
    }

    #[test]
    fn status_file_path_override_lands_directly_in_the_dir() {
        // Pure resolver: an override replaces the whole directory — the file
        // lands directly in it, no `iroh-tunnel` subpath. Both roles share
        // the resolver and differ only in the trailing file name.
        let path = StatusWriter::serve()
            .path_with(Some(OsStr::new("/tmp/status-override")))
            .unwrap();
        // Suffix assertions instead of exact-equality: `std::path::absolute`
        // yields a drive-rooted path with backslashes on Windows, so an
        // exact `PathBuf` compare would fail on a supported platform. What
        // the test actually proves: the override replaces the dir entirely
        // (absolute, no `iroh-tunnel` subpath) and keeps the file name.
        assert!(path.is_absolute());
        assert!(
            path.ends_with(std::path::Path::new("status-override").join("serve-status.json")),
            "override must replace the dir entirely, got {path:?}"
        );
        let access_path = StatusWriter::access()
            .path_with(Some(OsStr::new("/tmp/status-override")))
            .unwrap();
        assert!(access_path
            .ends_with(std::path::Path::new("status-override").join("access-status.json")));
    }

    #[test]
    fn empty_status_file_path_override_is_treated_as_unset() {
        // Standard env semantics: an empty value must not be honored — it
        // would make the path relative to the CWD. It resolves exactly like
        // an absent variable: the OS state dir + `iroh-tunnel` subpath.
        let empty = StatusWriter::serve()
            .path_with(Some(OsStr::new("")))
            .unwrap();
        let unset = StatusWriter::serve().path_with(None).unwrap();
        assert_eq!(empty, unset);
        assert_eq!(
            unset,
            crate::paths::default_state_dir()
                .unwrap()
                .join("serve-status.json")
        );
    }

    #[test]
    fn relative_status_file_path_override_is_absolutized() {
        // A relative override must resolve against the CWD once, at
        // resolution time — the returned path is absolute either way.
        let path = StatusWriter::serve()
            .path_with(Some(OsStr::new("rel/state")))
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(
            Path::new(&path).is_absolute(),
            "override path must be absolute, got {path}"
        );
    }
    #[tokio::test]
    async fn remove_own_status_file_spares_a_foreign_pid() {
        // Two processes sharing one state dir: this process's shutdown
        // cleanup must NOT remove a file another live instance wrote.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        StatusWriter::access()
            .save_to(
                &dir,
                &StatusPayload::Access(sample_access_status_with_pid(1)),
            )
            .unwrap();

        remove_own_status_file(StatusWriter::access(), Some(&dir)).await;

        assert!(
            dir.join("access-status.json").exists(),
            "a file owned by another pid must survive this process's cleanup"
        );
    }

    #[tokio::test]
    async fn remove_own_status_file_removes_own_pid() {
        // The file carries THIS process's pid — graceful shutdown owns it.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        StatusWriter::serve()
            .save_to(
                &dir,
                &StatusPayload::Serve(sample_status_with_pid(std::process::id())),
            )
            .unwrap();

        remove_own_status_file(StatusWriter::serve(), Some(&dir)).await;

        assert!(
            !dir.join("serve-status.json").exists(),
            "our own status file must be removed on cleanup"
        );
    }

    #[tokio::test]
    async fn remove_own_status_file_skips_missing_file() {
        // Never written (or already gone) — cleanup is a no-op, not an
        // error.
        let tmp = tempfile::tempdir().unwrap();
        remove_own_status_file(StatusWriter::serve(), Some(tmp.path())).await;
        assert!(!tmp.path().join("serve-status.json").exists());
    }

    /// [`sample_status`] with a chosen top-level `pid` for the ownership
    /// tests.
    fn sample_status_with_pid(pid: u32) -> StatusFile {
        let mut status = sample_status();
        status.pid = pid;
        status
    }

    /// [`sample_access_status`] with a chosen top-level `pid`.
    fn sample_access_status_with_pid(pid: u32) -> AccessStatusFile {
        let mut status = sample_access_status();
        status.pid = pid;
        status
    }

    /// A stand-in entry for any leftover temp file under `dir` (they are
    /// named `<prefix>.<pid>.<n>`).
    fn leftover_temp_files(dir: &Path, prefix: &str) -> std::path::PathBuf {
        // Any existing entry starting with the temp prefix counts; used with
        // `.exists()` so the caller can assert absence.
        dir.read_dir()
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| e.file_name().to_string_lossy().starts_with(prefix))
            .map(|e| e.path())
            .unwrap_or_else(|| dir.join(format!("{prefix}.none")))
    }
}
