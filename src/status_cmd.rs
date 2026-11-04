//! `iroh-tunnel <role> status`: read the role's status file and print it.
//!
//! Implements issue #59. The table renderers are pure functions of the
//! typed status structs (parsed from the real file) — unit tests pin them
//! with golden outputs, and the integration test drives them from a file a
//! live role actually wrote, without spawning the binary.
//!
//! ## Exit codes
//!
//! A missing status file means the role is not running. The command itself
//! is healthy, the config is fine, nothing permission-related failed, and
//! iroh was never reached — none of the specific [`crate::error::CliError`]
//! categories (config/permission/iroh/service) fits. It therefore maps to
//! the general error (exit 1) with an actionable message naming the exact
//! path (see README §Exit codes).

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::de::DeserializeOwned;

use crate::ownership;
pub use crate::ownership::{Liveness, OwnerRecord, Tier};
use crate::status::{
    AccessStatusFile, Role, StatusFile, StatusPayload, StatusRole, StatusWriter, TransportStatus,
};

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

fn snapshot_pid(payload: &StatusPayload) -> u32 {
    match payload {
        StatusPayload::Serve(file) => file.pid,
        StatusPayload::Access(file) => file.pid,
    }
}

fn snapshot_started_at(payload: &StatusPayload) -> u64 {
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

/// Entry point for `iroh-tunnel <role> status [--json]`.
pub fn run(role: StatusRole, json: bool) -> Result<()> {
    let status = read_role_status(role)?;
    let path = &status.path;
    let freshness = status.freshness();
    match status.snapshot {
        Snapshot::Invalid(e) => Err(e),
        Snapshot::Absent => {
            if let Liveness::Running { tier, owner } = &status.liveness {
                if json {
                    // Keep `--json` machine-readable: report the live owner.
                    let value = serde_json::json!({
                        "role": role.name(),
                        "running": true,
                        "pid": owner.as_ref().map(|o| o.pid),
                        "node_id": owner.as_ref().and_then(|o| o.node_id.clone()),
                        "snapshot": null,
                    });
                    return print_line(&value.to_string());
                }
                return print_line(&describe_running(role, *tier, owner.as_ref(), path));
            }
            Err(anyhow!(
                "{} is not running (no {} found at {})",
                role.name(),
                StatusWriter::new(role).file_name(),
                path.display()
            ))
        }
        Snapshot::Present { raw, payload } => {
            // A graceful shutdown removes the file, but a crash or `kill -9`
            // leaves it behind — stale, yet often still useful. Warn on
            // stderr, keep printing on stdout.
            warn_for_freshness(role, &freshness, &payload);
            if json {
                // Verbatim file contents, not a re-serialization: anything
                // piping this command must get byte-identical JSON to
                // reading the file.
                return print_line(&raw);
            }
            let table = match &payload {
                StatusPayload::Serve(file) => render_serve_status(file),
                StatusPayload::Access(file) => render_access_status(file),
            };
            print_line(&table)
        }
    }
}

/// Write one line to stdout, tolerating a closed pipe.
///
/// `println!` panics on a broken pipe (exit 101) — but a consumer closing
/// early (`iroh-tunnel serve status | head`) is a normal exit for this
/// command, not an error. Any other write failure propagates.
fn print_line(line: &str) -> Result<()> {
    let mut out = std::io::stdout().lock();
    match writeln!(out, "{line}") {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(e) => Err(anyhow::Error::new(e).context("failed to write status output")),
    }
}

/// Print the stale-snapshot warning for `freshness`, if any.
fn warn_for_freshness(role: StatusRole, freshness: &Freshness, payload: &StatusPayload) {
    let pid = snapshot_pid(payload);
    let started_at = snapshot_started_at(payload);
    match freshness {
        Freshness::StalePid { live_pid: None } => eprintln!(
            "warning: {} is not running; this snapshot (pid {pid}) is stale, showing the last \
             snapshot from epoch {started_at}",
            role.name()
        ),
        Freshness::StalePid {
            live_pid: Some(live),
        } => eprintln!(
            "warning: {} is running as pid {live}, but this snapshot was written by pid {pid} \
             and is stale; showing the last snapshot from epoch {started_at}",
            role.name()
        ),
        Freshness::OwnerUnknown => eprintln!(
            "warning: {} is running but its owner record is missing, so this snapshot (pid {pid}) \
             cannot be verified as current; showing the last snapshot from epoch {started_at}",
            role.name()
        ),
        Freshness::Current
        | Freshness::LiveWithoutSnapshot
        | Freshness::AbsentNotRunning
        | Freshness::Unreadable => {}
    }
}

/// One line naming the live owner when no snapshot is readable here.
fn describe_running(
    role: StatusRole,
    tier: Tier,
    owner: Option<&OwnerRecord>,
    path: &Path,
) -> String {
    format!(
        "{} is running for {} ({}); no status snapshot at {} (it may use another state directory)",
        role.name(),
        tier.label(),
        ownership::describe_owner(owner),
        path.display()
    )
}

/// Parse a status file body as `T`, with the path in the error context.
fn parse_status_file<T: DeserializeOwned>(body: &str, path: &Path) -> Result<T> {
    serde_json::from_str(body).with_context(|| format!("invalid status file: {}", path.display()))
}

/// Render the serve status file as human-readable tables.
///
/// Pure: a function of the file contents only (no uptime math, no clock),
/// so the golden test output is stable. Two sections — the exposed services
/// (name, protocol, local address, live stream count) and the connected
/// peers with their transports — so an operator sees both "what am I
/// exposing" and "who is connected" without reaching for `--json`.
pub fn render_serve_status(file: &StatusFile) -> String {
    let mut out = format!("node_id: {}\n", file.node_id);
    match &file.home_relay {
        Some(relay) => out.push_str(&format!("home_relay: {relay}\n")),
        None => out.push_str("home_relay: none\n"),
    }
    out.push_str(&format!(
        "pid: {}  started_at: {}\n",
        file.pid, file.started_at
    ));

    let services = if file.services.is_empty() {
        "services: none".to_string()
    } else {
        let rows: Vec<Vec<Vec<String>>> = file
            .services
            .iter()
            .map(|svc| {
                vec![
                    vec![svc.name.clone()],
                    vec![svc.protocol.clone()],
                    vec![svc.local_addr.clone()],
                    // The file's `active_connections` — active streams, see
                    // the schema docs in `status.rs`.
                    vec![svc.active_connections.to_string()],
                ]
            })
            .collect();
        render_table(&["SERVICE", "PROTOCOL", "LOCAL ADDR", "STREAMS"], &rows)
    };
    let connections = if file.connections.is_empty() {
        "connections: none".to_string()
    } else {
        let rows: Vec<Vec<Vec<String>>> = file
            .connections
            .iter()
            .map(|conn| {
                vec![
                    vec![short_peer_id(&conn.path.peer)],
                    vec![conn.services.join(",")],
                    transport_lines(&conn.path.transports, "(none)"),
                ]
            })
            .collect();
        render_table(&["PEER", "SERVICES", "TRANSPORTS"], &rows)
    };
    out.push_str(&format!("\n{services}\n\n{connections}"));
    out
}

/// Render the access status file as a human-readable table.
///
/// Pure, like [`render_serve_status`]. A service with no live connection
/// still gets a row — its configured peer with `(no connection)` — because
/// "which peer is this service supposed to reach" is exactly what an
/// operator troubleshooting a tunnel wants to see.
pub fn render_access_status(file: &AccessStatusFile) -> String {
    let mut out = format!("node_id: {}\n", file.node_id);
    out.push_str(&format!(
        "pid: {}  started_at: {}\n",
        file.pid, file.started_at
    ));
    if file.services.is_empty() {
        out.push_str("services: none");
        return out;
    }
    out.push('\n');
    let rows: Vec<Vec<Vec<String>>> = file
        .services
        .iter()
        .map(|svc| {
            vec![
                vec![svc.name.clone()],
                vec![svc.listen_addr.clone()],
                vec![short_peer_id(&svc.peer)],
                transport_lines(&svc.transports, "(no connection)"),
            ]
        })
        .collect();
    out.push_str(&render_table(
        &["SERVICE", "LISTEN", "PEER", "TRANSPORTS"],
        &rows,
    ));
    out
}

/// Short node id via the shared [`crate::conn_path::short_peer_id`] — one
/// definition shared with the access role's log lines, so a short id means
/// the same thing everywhere. The header of each table carries this node's
/// full id; the `--json` output carries everyone's.
fn short_peer_id(peer: &str) -> String {
    crate::conn_path::short_peer_id(peer)
}

/// One line per transport: `relay <url> [active]` / `direct <addr>`. The
/// `[active]` marker is appended only while iroh is actively sending on the
/// path — a bare `kind addr` line is a known-but-idle candidate.
fn transport_lines(transports: &[TransportStatus], empty_label: &str) -> Vec<String> {
    if transports.is_empty() {
        return vec![empty_label.to_string()];
    }
    transports
        .iter()
        .map(|t| {
            if t.active {
                format!("{} {} [active]", t.kind, t.addr)
            } else {
                format!("{} {}", t.kind, t.addr)
            }
        })
        .collect()
}

/// Render `headers` + `rows` as a padded text table, separated by two
/// spaces. Each cell is a list of lines: a multi-line cell (e.g. several
/// transports) writes its continuation lines aligned under the first, with
/// the earlier columns blank. Hand-rolled — two fixed table shapes do not
/// justify a table-rendering dependency.
fn render_table(headers: &[&str], rows: &[Vec<Vec<String>>]) -> String {
    // Column width = widest line across the header and every cell.
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (col, cell) in row.iter().enumerate() {
            for line in cell {
                widths[col] = widths[col].max(line.chars().count());
            }
        }
    }
    let mut lines: Vec<String> = Vec::new();
    lines.extend(padded_lines(
        &headers
            .iter()
            .map(|h| vec![(**h).to_string()])
            .collect::<Vec<_>>(),
        &widths,
    ));
    lines.push(
        widths
            .iter()
            .map(|w| "-".repeat(*w))
            .collect::<Vec<_>>()
            .join("  "),
    );
    for row in rows {
        lines.extend(padded_lines(row, &widths));
    }
    lines.join("\n")
}

/// The physical lines of one table row: line `i` of the output carries line
/// `i` of every cell, padded to its column width (trailing blanks trimmed).
fn padded_lines(cells: &[Vec<String>], widths: &[usize]) -> Vec<String> {
    let height = cells.iter().map(Vec::len).max().unwrap_or(1);
    (0..height)
        .map(|line| {
            let mut out = String::new();
            for (col, cell) in cells.iter().enumerate() {
                let text = cell.get(line).map(String::as_str).unwrap_or("");
                out.push_str(&format!("{text:<width$}", width = widths[col]));
                if col + 1 < widths.len() {
                    out.push_str("  ");
                }
            }
            out.trim_end().to_string()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::{
        AccessServiceStatus, PeerConnectionStatus, PeerPathReport, ServiceStatus, TransportKind,
    };

    /// A realistic 64-hex-char serve peer id (same shape as real node ids,
    /// so the short-id truncation is exercised exactly as in production).
    const SERVE_PEER: &str = "1aa27080cc694eb1e756c5e9260eb267208f7b677e6a74e53146bf417fb31f84";
    const OTHER_PEER: &str = "f00dfacefeeddeadbeefcafe1234567890abcdef1234567890abcdef12345678";

    fn sample_serve() -> StatusFile {
        StatusFile {
            node_id: "abc123".to_string(),
            home_relay: Some("https://relay.example/".to_string()),
            pid: 42,
            started_at: 1_700_000_000,
            services: vec![ServiceStatus {
                name: "echo".to_string(),
                protocol: "tcp".to_string(),
                local_addr: "127.0.0.1:8080".to_string(),
                active_connections: 2,
                udp: None,
            }],
            connections: vec![PeerConnectionStatus {
                path: PeerPathReport {
                    peer: SERVE_PEER.to_string(),
                    transports: vec![
                        TransportStatus {
                            kind: TransportKind::Relay,
                            addr: "https://relay.example/".to_string(),
                            active: true,
                        },
                        TransportStatus {
                            kind: TransportKind::Direct,
                            addr: "192.168.1.10:52618".to_string(),
                            active: false,
                        },
                    ],
                    local_bound_addrs: vec!["0.0.0.0:52110".to_string()],
                },
                services: vec!["echo".to_string()],
            }],
        }
    }

    fn sample_access() -> AccessStatusFile {
        AccessStatusFile {
            node_id: "acc789".to_string(),
            pid: 43,
            started_at: 1_700_000_001,
            services: vec![
                AccessServiceStatus {
                    name: "echo".to_string(),
                    listen_addr: "127.0.0.1:8080".to_string(),
                    peer: SERVE_PEER.to_string(),
                    transports: vec![
                        TransportStatus {
                            kind: TransportKind::Relay,
                            addr: "https://relay.example/".to_string(),
                            active: true,
                        },
                        TransportStatus {
                            kind: TransportKind::Direct,
                            addr: "192.168.1.10:52618".to_string(),
                            active: false,
                        },
                    ],
                    local_bound_addrs: vec!["0.0.0.0:52111".to_string()],
                    udp: None,
                },
                AccessServiceStatus {
                    name: "db".to_string(),
                    listen_addr: "127.0.0.1:5433".to_string(),
                    peer: OTHER_PEER.to_string(),
                    transports: Vec::new(),
                    local_bound_addrs: Vec::new(),
                    udp: None,
                },
            ],
        }
    }

    #[test]
    fn serve_table_golden() {
        let got = render_serve_status(&sample_serve());
        let want = "\
node_id: abc123
home_relay: https://relay.example/
pid: 42  started_at: 1700000000

SERVICE  PROTOCOL  LOCAL ADDR      STREAMS
-------  --------  --------------  -------
echo     tcp       127.0.0.1:8080  2

PEER       SERVICES  TRANSPORTS
---------  --------  -------------------------------------
1aa27080…  echo      relay https://relay.example/ [active]
                     direct 192.168.1.10:52618";
        assert_eq!(got, want);
    }

    #[test]
    fn serve_table_without_connections_or_relay() {
        let mut sample = sample_serve();
        sample.home_relay = None;
        sample.connections.clear();
        let got = render_serve_status(&sample);
        let want = "\
node_id: abc123
home_relay: none
pid: 42  started_at: 1700000000

SERVICE  PROTOCOL  LOCAL ADDR      STREAMS
-------  --------  --------------  -------
echo     tcp       127.0.0.1:8080  2

connections: none";
        assert_eq!(got, want);
    }

    #[test]
    fn access_table_golden() {
        let got = render_access_status(&sample_access());
        let want = "\
node_id: acc789
pid: 43  started_at: 1700000001

SERVICE  LISTEN          PEER       TRANSPORTS
-------  --------------  ---------  -------------------------------------
echo     127.0.0.1:8080  1aa27080…  relay https://relay.example/ [active]
                                    direct 192.168.1.10:52618
db       127.0.0.1:5433  f00dface…  (no connection)";
        assert_eq!(got, want);
    }

    #[test]
    fn access_table_without_services() {
        let mut sample = sample_access();
        sample.services.clear();
        let got = render_access_status(&sample);
        let want = "\
node_id: acc789
pid: 43  started_at: 1700000001
services: none";
        assert_eq!(got, want);
    }

    #[test]
    fn short_peer_id_takes_first_eight_chars() {
        assert_eq!(short_peer_id(SERVE_PEER), "1aa27080…");
        // Shorter ids are kept whole, with the ellipsis marking truncation.
        assert_eq!(short_peer_id("peer456"), "peer456…");
    }

    fn serve_file_json(pid: u32) -> String {
        let mut file = sample_serve();
        file.pid = pid;
        serde_json::to_string(&file).unwrap()
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
