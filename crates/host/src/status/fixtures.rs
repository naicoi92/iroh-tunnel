//! Shared fixtures of the status unit tests.

use super::schema::{
    AccessServiceStatus, AccessStatusFile, PeerConnectionStatus, PeerPathReport, ServiceStatus,
    StatusFile, TransportKind, TransportStatus,
};

pub(crate) fn sample_status() -> StatusFile {
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

pub(crate) fn sample_access_status() -> AccessStatusFile {
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
