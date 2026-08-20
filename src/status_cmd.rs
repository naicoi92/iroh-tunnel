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

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use serde::de::DeserializeOwned;

use crate::conn_path::TransportStatus;
use crate::status::{AccessStatusFile, StatusFile, StatusRole, StatusWriter};

/// Entry point for `iroh-tunnel <role> status [--json]`.
///
pub fn run(role: StatusRole, json: bool) -> Result<()> {
    let writer = StatusWriter::new(role);
    let path = writer.path()?;
    let body = match std::fs::read_to_string(&path) {
        Ok(body) => body,
        // A missing file is "not running", not a hard IO failure the
        // operator must debug — one plain sentence with the path it looked
        // at. Any OTHER read error (permissions, a directory squatting on
        // the path, …) is a real failure and propagates with its cause:
        // misdiagnosing those as "stopped" would hide the actual problem.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(anyhow!(
                "{} is not running (no {} found at {})",
                role.name(),
                writer.file_name(),
                path.display()
            ));
        }
        Err(e) => {
            return Err(anyhow::Error::new(e)
                .context(format!("failed to read status file: {}", path.display())));
        }
    };
    if json {
        // Verbatim file contents, not a re-serialization: anything piping
        // this command must get byte-identical JSON to reading the file.
        // Validated first so a corrupt file fails loudly on this path too,
        // exactly like the table path.
        let _: serde_json::Value = parse_status_file(&body, &path)?;
        println!("{body}");
        return Ok(());
    }
    let table = match role {
        StatusRole::Serve => render_serve_status(&parse_status_file(&body, &path)?),
        StatusRole::Access => render_access_status(&parse_status_file(&body, &path)?),
    };
    println!("{table}");
    Ok(())
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

/// First 8 chars of a node id + `…` — the same shape as the access role's
/// log lines (`access::short_peer_id`) so a short id means the same thing
/// everywhere. The header of each table carries this node's full id; the
/// `--json` output carries everyone's.
fn short_peer_id(peer: &str) -> String {
    let head: String = peer.chars().take(8).collect();
    format!("{head}…")
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
    use crate::conn_path::{PeerPathReport, TransportKind};
    use crate::status::{AccessServiceStatus, PeerConnectionStatus, ServiceStatus};

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
                },
                AccessServiceStatus {
                    name: "db".to_string(),
                    listen_addr: "127.0.0.1:5433".to_string(),
                    peer: OTHER_PEER.to_string(),
                    transports: Vec::new(),
                    local_bound_addrs: Vec::new(),
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
}
