//! Native observations of a running role, and the sink that publishes them.
//!
//! Role code builds these plain values from live state and hands them to an
//! injected [`StatusSink`]; it never touches the status-file schema. The
//! host side maps an observation to the status-file JSON (see
//! the host status module) and writes it. Observations carry no timestamp, so
//! `PartialEq` decides "changed since the last save" on rendered content only.

use std::time::Duration;

/// How often each role's status flush task re-renders its observation.
///
/// Shared so the two roles' cadence can never drift: 5 s is near-live for
/// operators while keeping the endpoint query and disk write off the hot
/// path.
pub(crate) const STATUS_FLUSH_INTERVAL: Duration = Duration::from_secs(5);

/// What the serve role observes about itself.
#[derive(Clone, PartialEq, Debug)]
pub struct ServeObservation {
    /// Full endpoint id of this serve node.
    pub node_id: String,
    /// Home relay URL, if the endpoint has one.
    pub home_relay: Option<String>,
    /// Operating-system process id of the running role.
    pub pid: u32,
    /// Role start time, in seconds since the Unix epoch.
    pub started_at: u64,
    /// Configured Serve services.
    pub services: Vec<ServiceObservation>,
    /// Currently connected remote peers.
    pub connections: Vec<PeerConnectionObservation>,
}

/// One configured serve service.
#[derive(Clone, PartialEq, Debug)]
pub struct ServiceObservation {
    /// Service name.
    pub name: String,
    /// Protocol name, `"tcp"` or `"udp"`.
    pub protocol: String,
    /// Local `host:port` the service forwards to.
    pub local_addr: String,
    /// Active streams (in-flight pipes), not iroh connections.
    pub active_connections: u64,
    /// Present only for a UDP service.
    pub udp: Option<UdpObservation>,
}

/// UDP flow diagnostics of one service.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct UdpObservation {
    /// Number of live Flows (count).
    pub active_flows: u64,
    /// Number of Flows currently in Quarantine (count).
    pub quarantined_flows: u64,
    /// Current Datagram limit in bytes; `None` when not yet known.
    pub datagram_limit: Option<u64>,
    /// Datagrams dropped so far, by reason.
    pub drops: UdpDropCounts,
}

/// Datagrams dropped, by reason.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct UdpDropCounts {
    /// Datagrams dropped for exceeding the Datagram limit (count).
    pub oversize: u64,
    /// Datagrams dropped because they could not be parsed (count).
    pub malformed: u64,
    /// Datagrams dropped for an unknown Flow (count).
    pub unknown_flow: u64,
    /// Datagrams dropped on an upstream socket error (count).
    pub upstream_error: u64,
    /// Datagrams dropped because the Flow cap was reached (count).
    pub cap: u64,
    /// Datagrams dropped because a Flow queue was full (count).
    pub queue_full: u64,
    /// Datagrams dropped because the datagram limit
    /// became unusable during the session incarnation (count).
    pub limit_shrink: u64,
    /// Replies dropped for a Flow in Quarantine (count).
    pub quarantine_reply: u64,
    /// Datagrams dropped for arriving from an unexpected source (count).
    pub foreign_source: u64,
}

/// One connected remote peer of the serve role.
#[derive(Clone, PartialEq, Debug)]
pub struct PeerConnectionObservation {
    /// How this peer is currently reachable.
    pub path: PeerPathObservation,
    /// Service names this peer is connected to.
    pub services: Vec<String>,
}

/// How one remote peer is currently reachable.
#[derive(Clone, PartialEq, Debug)]
pub struct PeerPathObservation {
    /// The remote peer's endpoint id (full node id string).
    pub peer: String,
    /// Transports iroh knows for this peer, active ones first.
    pub transports: Vec<TransportObservation>,
    /// Endpoint-wide local UDP socket candidates.
    pub local_bound_addrs: Vec<String>,
}

/// What the access role observes about itself.
#[derive(Clone, PartialEq, Debug)]
pub struct AccessObservation {
    /// Full endpoint id of this Access node.
    pub node_id: String,
    /// Operating-system process id of the running role.
    pub pid: u32,
    /// Role start time, in seconds since the Unix epoch.
    pub started_at: u64,
    /// Configured Access services.
    pub services: Vec<AccessServiceObservation>,
}

/// One configured access service.
#[derive(Clone, PartialEq, Debug)]
pub struct AccessServiceObservation {
    /// Service name.
    pub name: String,
    /// Local `host:port` the service listens on.
    pub listen_addr: String,
    /// Configured serve peer (full node id).
    pub peer: String,
    /// Transports iroh knows for the Serve peer, active ones first.
    pub transports: Vec<TransportObservation>,
    /// Endpoint-wide local UDP socket candidates.
    pub local_bound_addrs: Vec<String>,
    /// Present only for a UDP service.
    pub udp: Option<UdpObservation>,
}

/// One transport path to a remote peer.
#[derive(Clone, PartialEq, Debug)]
pub struct TransportObservation {
    /// Whether the path goes via a relay or directly.
    pub kind: TransportKind,
    /// The relay URL or `SocketAddr` in use.
    pub addr: String,
    /// Whether iroh is actively sending on this transport.
    pub active: bool,
}

/// The kind of network path a transport uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TransportKind {
    /// Via a relay server.
    Relay,
    /// Direct IP path.
    Direct,
}

impl std::fmt::Display for TransportKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            TransportKind::Relay => "relay",
            TransportKind::Direct => "direct",
        })
    }
}
