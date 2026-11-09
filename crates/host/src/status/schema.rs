//! Status-file schema: serde DTOs and their `From` impls from engine
//! observations (Page 06 v5 §4).

use iroh_tunnel_engine::observation as obs;
use serde::{Deserialize, Serialize};

use crate::role::Role;

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
    /// Service name.
    pub name: String,
    /// Protocol text (`"tcp"` / `"udp"`).
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
    /// Per-reason drop counters.
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
    /// Datagrams became unusable on the connection mid-session. Until #152 is
    /// resolved, a connection whose datagrams are never usable also counts every
    /// datagram here; see `docs/udp-integration-contract.md`.
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
    /// Service name.
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
/// `StatusWriter` is role-tagged and its save methods take this enum;
/// `StatusWriter::save_to` rejects a mismatched writer/payload pair with
/// a runtime role check that errors in every build profile, instead of
/// silently writing the wrong schema under the right file name.
#[derive(Debug, Clone, PartialEq)]
pub enum StatusPayload {
    /// A serve status file.
    Serve(StatusFile),
    /// An access status file.
    Access(AccessStatusFile),
}

impl StatusPayload {
    /// Pid recorded in the snapshot.
    pub fn pid(&self) -> u32 {
        match self {
            StatusPayload::Serve(file) => file.pid,
            StatusPayload::Access(file) => file.pid,
        }
    }

    /// Start time (epoch seconds) recorded in the snapshot.
    pub fn started_at(&self) -> u64 {
        match self {
            StatusPayload::Serve(file) => file.started_at,
            StatusPayload::Access(file) => file.started_at,
        }
    }

    /// The role whose schema this payload carries.
    pub fn role(&self) -> Role {
        match self {
            StatusPayload::Serve(_) => Role::Serve,
            StatusPayload::Access(_) => Role::Access,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::{sample_access_status, sample_status};
    use super::super::writer::StatusWriter;
    use super::*;

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
    fn payload_exposes_pid_started_at_and_role() {
        let payload = StatusPayload::Serve(sample_status());
        assert_eq!(payload.pid(), 42);
        assert_eq!(payload.started_at(), 1_700_000_000);
        assert_eq!(payload.role(), Role::Serve);
        let payload = StatusPayload::Access(sample_access_status());
        assert_eq!(payload.pid(), 43);
        assert_eq!(payload.started_at(), 1_700_000_001);
        assert_eq!(payload.role(), Role::Access);
    }
}
