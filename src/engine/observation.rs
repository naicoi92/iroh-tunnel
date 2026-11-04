//! Native observations of a running role, and the sink that publishes them.
//!
//! Role code builds these plain values from live state and hands them to an
//! injected [`StatusSink`]; it never touches the status-file schema. The
//! host side maps an observation to the status-file JSON (see
//! the host status module) and writes it. Observations carry no timestamp, so
//! `PartialEq` decides "changed since the last save" on rendered content only.

use std::future::Future;
use std::pin::Pin;

use std::time::Duration;

use crate::engine::ready::OnReady;

/// How often each role's status flush task re-renders its observation.
///
/// Shared so the two roles' cadence can never drift: 5 s is near-live for
/// operators while keeping the endpoint query and disk write off the hot
/// path.
pub(crate) const STATUS_FLUSH_INTERVAL: Duration = Duration::from_secs(5);

/// Boxed, sendable future (local alias; no `futures` dependency).
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What the serve role observes about itself.
#[derive(Clone, PartialEq, Debug)]
pub struct ServeObservation {
    pub node_id: String,
    pub home_relay: Option<String>,
    pub pid: u32,
    pub started_at: u64,
    pub services: Vec<ServiceObservation>,
    pub connections: Vec<PeerConnectionObservation>,
}

/// One configured serve service.
#[derive(Clone, PartialEq, Debug)]
pub struct ServiceObservation {
    pub name: String,
    pub protocol: String,
    pub local_addr: String,
    /// Active streams (in-flight pipes), not iroh connections.
    pub active_connections: u64,
    /// Present only for a UDP service.
    pub udp: Option<UdpObservation>,
}

/// UDP flow diagnostics of one service.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct UdpObservation {
    pub active_flows: u64,
    pub quarantined_flows: u64,
    pub datagram_limit: Option<u64>,
    pub drops: UdpDropCounts,
}

/// Datagrams dropped, by reason.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct UdpDropCounts {
    pub oversize: u64,
    pub malformed: u64,
    pub unknown_flow: u64,
    pub upstream_error: u64,
    pub cap: u64,
    pub queue_full: u64,
    pub limit_shrink: u64,
    pub quarantine_reply: u64,
    pub foreign_source: u64,
}

/// One connected remote peer of the serve role.
#[derive(Clone, PartialEq, Debug)]
pub struct PeerConnectionObservation {
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
    pub node_id: String,
    pub pid: u32,
    pub started_at: u64,
    pub services: Vec<AccessServiceObservation>,
}

/// One configured access service.
#[derive(Clone, PartialEq, Debug)]
pub struct AccessServiceObservation {
    pub name: String,
    pub listen_addr: String,
    /// Configured serve peer (full node id).
    pub peer: String,
    pub transports: Vec<TransportObservation>,
    pub local_bound_addrs: Vec<String>,
    /// Present only for a UDP service.
    pub udp: Option<UdpObservation>,
}

/// One transport path to a remote peer.
#[derive(Clone, PartialEq, Debug)]
pub struct TransportObservation {
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

/// Publishes observations of type `O`; the host decides where they go.
pub struct StatusSink<O> {
    save: Box<dyn FnMut(O) -> BoxFuture<'static, anyhow::Result<()>> + Send>,
}

impl<O> StatusSink<O> {
    pub fn new(
        save: impl FnMut(O) -> BoxFuture<'static, anyhow::Result<()>> + Send + 'static,
    ) -> Self {
        Self {
            save: Box::new(save),
        }
    }

    /// Persist one observation; resolves once the save has finished.
    pub fn save(&mut self, observation: O) -> BoxFuture<'static, anyhow::Result<()>> {
        (self.save)(observation)
    }
}

/// Host-provided hooks of one role run.
pub struct RoleHooks<O> {
    pub on_ready: OnReady,
    /// `None` runs the role without publishing status.
    pub status: Option<StatusSink<O>>,
}
