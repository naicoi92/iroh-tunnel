//! Peer connection-path reporting for status files.
//!
//! Shared seam between the serve status file (`serve-status.json`, issue #57)
//! and the upcoming access-side status work (#58/#59): turns iroh's live
//! endpoint state into a small, serializable snapshot of *how* a peer is
//! currently reachable.
//!
//! Two layers, deliberately split:
//!
//! - [`peer_path_report`] — the async query against a live [`iroh::Endpoint`]
//!   (`Endpoint::remote_info` snapshot + `Endpoint::bound_sockets`).
//! - [`classify_transport`] / [`sort_transports`] — pure mappers, unit-testable
//!   without an endpoint.
//!
//! ## Semantics
//!
//! iroh negotiates relay and direct paths concurrently, so the transport list
//! is a *list with usage*, not a single mode: a peer can be direct-only (hole
//! punching failed), relay-only, or have both active at once (direct known and
//! reachable, relay kept as fallback). [`TransportStatus::active`] mirrors
//! iroh's own `TransportAddrUsage` snapshot at query time.
//!
//! `local_bound_addrs` are the endpoint's local UDP socket *candidates* (one
//! per address family the endpoint bound) — iroh 1.0 does not expose a
//! local-address-per-path mapping, so these are NOT paired with remote
//! addresses and must not be read as "the local addr of this transport".

use serde::{Deserialize, Serialize};

use iroh::endpoint::TransportAddrUsage;
use iroh::{Endpoint, EndpointId, TransportAddr};

/// How one remote peer is currently reachable, for status files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PeerPathReport {
    /// The remote peer's endpoint id (full node id string).
    pub peer: String,
    /// Transports iroh knows for this peer, active ones first.
    pub transports: Vec<TransportStatus>,
    /// Local UDP socket candidates of *our* endpoint (see module docs —
    /// endpoint-wide, not a per-transport local address).
    pub local_bound_addrs: Vec<String>,
}

/// The kind of network path a transport uses.
///
/// Serialized lowercase (`"relay"` / `"direct"`) — the documented status-file
/// schema stays byte-identical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransportKind {
    /// Via a relay server (`TransportAddr::Relay`).
    Relay,
    /// Direct IP path (`TransportAddr::Ip`).
    Direct,
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

/// Snapshot the current connection path of `peer` on `ep`.
///
/// Returns `None` when iroh has no entry for the peer (e.g. it disconnected
/// and its remote-map entry already expired) — callers skip such peers; the
/// next periodic flush picks them up again if they reconnect.
pub async fn peer_path_report(ep: &Endpoint, peer: EndpointId) -> Option<PeerPathReport> {
    let info = ep.remote_info(peer).await?;
    let mut transports: Vec<TransportStatus> = info
        .addrs()
        .filter_map(|addr_info| {
            let (kind, addr) = classify_transport(addr_info.addr())?;
            Some(TransportStatus {
                kind,
                addr,
                active: matches!(addr_info.usage(), TransportAddrUsage::Active),
            })
        })
        .collect();
    sort_transports(&mut transports);
    // Equal (kind, addr, active) rows are adjacent after sorting; collapse
    // them so iroh reporting a duplicate path (e.g. after re-hole-punch
    // attempts or duplicate relay entries) never duplicates status rows.
    transports.dedup();
    // bound_sockets() ordering is not guaranteed stable across calls; sort
    // for the same reason transports are sorted — two snapshots of an
    // unchanged endpoint must compare equal, or the flush loop would
    // rewrite the status file every 5 s for no real change.
    let mut local_bound_addrs: Vec<String> =
        ep.bound_sockets().iter().map(|a| a.to_string()).collect();
    local_bound_addrs.sort();
    Some(PeerPathReport {
        peer: peer.to_string(),
        transports,
        local_bound_addrs,
    })
}

/// Map a raw iroh transport address to its reportable `(kind, addr)` form.
///
/// `Custom` transports are dropped: iroh-tunnel's `Minimal` endpoint preset
/// registers no custom transports, so one showing up here would be noise, not
/// an operator-actionable path.
fn classify_transport(addr: &TransportAddr) -> Option<(TransportKind, String)> {
    match addr {
        TransportAddr::Relay(url) => Some((TransportKind::Relay, url.to_string())),
        TransportAddr::Ip(addr) => Some((TransportKind::Direct, addr.to_string())),
        // Rationale in the doc comment above — but leave a trace instead of
        // silently vanishing, in case a future endpoint preset ever
        // registers custom transports.
        _ => {
            tracing::debug!(
                ?addr,
                "dropping non relay/direct transport from status report"
            );
            None
        }
    }
}

/// Order transports for output into a total order: active before inactive,
/// then [`TransportKind::Relay`] before [`TransportKind::Direct`] (enum
/// declaration order), then by `addr` as the final tie-break.
///
/// The tie-break matters for the flush loop's change detection: iroh's own
/// ordering within an equal (active, kind) group can differ between
/// snapshots, and without a deterministic sort the rendered file would
/// compare unequal and get rewritten every flush despite no real change.
fn sort_transports(transports: &mut [TransportStatus]) {
    // Chained comparisons instead of a sort_by_key tuple key: avoids
    // allocating a String key per comparison.
    transports.sort_by(|a, b| {
        std::cmp::Reverse(a.active)
            .cmp(&std::cmp::Reverse(b.active))
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.addr.cmp(&b.addr))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relay_addr() -> TransportAddr {
        TransportAddr::Relay("https://use1-1.relay.iroh.network/".parse().unwrap())
    }

    fn direct_addr() -> TransportAddr {
        TransportAddr::Ip("192.168.1.10:54321".parse().unwrap())
    }

    fn direct_addr_v6() -> TransportAddr {
        TransportAddr::Ip("[2001:db8::1]:443".parse().unwrap())
    }

    #[test]
    fn classify_maps_relay_url() {
        let (kind, addr) = classify_transport(&relay_addr()).unwrap();
        assert_eq!(kind, TransportKind::Relay);
        assert_eq!(addr, "https://use1-1.relay.iroh.network/");
    }

    #[test]
    fn classify_maps_direct_socket_addr() {
        let (kind, addr) = classify_transport(&direct_addr()).unwrap();
        assert_eq!(kind, TransportKind::Direct);
        assert_eq!(addr, "192.168.1.10:54321");

        let (kind, addr) = classify_transport(&direct_addr_v6()).unwrap();
        assert_eq!(kind, TransportKind::Direct);
        // SocketAddr's Display already brackets IPv6 literals.
        assert_eq!(addr, "[2001:db8::1]:443");
    }

    #[test]
    fn classify_drops_custom_transports() {
        // CustomAddr string form: `{hex transport id}_{hex data}`.
        let custom = TransportAddr::Custom("1_".parse().unwrap());
        assert!(classify_transport(&custom).is_none());
    }

    #[test]
    fn sort_puts_active_first_then_relay() {
        let mk = |kind: TransportKind, active: bool| TransportStatus {
            kind,
            addr: "x".to_string(),
            active,
        };
        let mut transports = vec![
            mk(TransportKind::Relay, false),
            mk(TransportKind::Direct, true),
            mk(TransportKind::Direct, false),
        ];
        sort_transports(&mut transports);

        assert_eq!(transports[0], mk(TransportKind::Direct, true)); // the only active one
        assert_eq!(transports[1], mk(TransportKind::Relay, false)); // relay before direct
        assert_eq!(transports[2], mk(TransportKind::Direct, false));
    }

    #[test]
    fn sort_tiebreaks_by_addr_within_equal_groups() {
        // Same (active, kind) — the addr tie-break must make the order a
        // total order, so two snapshots with the same set always render
        // equal (no spurious status rewrites).
        let mk = |addr: &str| TransportStatus {
            kind: TransportKind::Relay,
            addr: addr.to_string(),
            active: true,
        };
        let mut transports = vec![mk("c"), mk("a"), mk("b")];
        sort_transports(&mut transports);
        assert_eq!(
            transports
                .iter()
                .map(|t| t.addr.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b", "c"]
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

    #[tokio::test]
    async fn peer_path_report_returns_none_for_never_connected_peer() {
        // A fresh offline endpoint has no remote-map entry for a peer it has
        // never heard of — `peer_path_report` must return None (the flush
        // loop's skip path), never an empty report.
        let ep = iroh::Endpoint::bind(iroh::endpoint::presets::Minimal)
            .await
            .unwrap();
        let stranger = iroh::SecretKey::generate().public();
        assert!(peer_path_report(&ep, stranger).await.is_none());
    }
}
