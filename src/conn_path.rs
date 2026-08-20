//! Peer connection-path reporting for status files and access logs.
//!
//! Shared seam between the serve status file (`serve-status.json`, issue
//! #57) and the access-side connection-path logs (issue #58): turns iroh's
//! live endpoint state into a small, serializable snapshot of *how* a peer
//! is currently reachable.
//!
//! Two layers, deliberately split:
//!
//! - [`peer_path_report`] — the async query against a live [`iroh::Endpoint`]
//!   (`Endpoint::remote_info` snapshot + `Endpoint::bound_sockets`).
//! - [`classify_transport`] / [`sort_transports`] / [`diff_transports`] /
//!   [`poller_step`] — pure mappers, unit-testable without an endpoint.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransportKind {
    /// Via a relay server (`TransportAddr::Relay`).
    Relay,
    /// Direct IP path (`TransportAddr::Ip`).
    Direct,
}

impl std::fmt::Display for TransportKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Matches the serde lowercase form (see the serde attr above).
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

// ---------------------------------------------------------------------------
// Path-change diff + log rendering (access logs, issue #58)
// ---------------------------------------------------------------------------

/// What changed between two transport snapshots, as computed by
/// [`diff_transports`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PathChangeSummary {
    /// Active transports before the change, in snapshot order.
    pub before_active: Vec<TransportStatus>,
    /// Active transports after the change, in snapshot order.
    pub after_active: Vec<TransportStatus>,
    /// Kinds that gained at least one newly-active address.
    pub kinds_added: Vec<TransportKind>,
    /// Kinds that lost at least one previously-active address.
    pub kinds_removed: Vec<TransportKind>,
}

/// Diff two transport snapshots for operator-visible path changes (#58).
///
/// Compares only the *active set per kind* — the `(kind, addr)` pairs with
/// `active == true`. Sort-order differences and changes among inactive
/// candidates never surface to operators, so they return `None`. An address
/// swap inside one kind *does* count: the kind then appears in both
/// `kinds_added` and `kinds_removed`.
///
/// The (common) no-change tick returns `None` without cloning anything:
/// membership is decided on borrowed hash sets, and the active-transport
/// vectors are only built for a real change.
pub(crate) fn diff_transports(
    before: &[TransportStatus],
    after: &[TransportStatus],
) -> Option<PathChangeSummary> {
    // A nested fn (not a closure): the borrowed-tuple return needs true
    // for<'a> universality, which closure lifetime elision cannot express.
    fn active_set(ts: &[TransportStatus]) -> std::collections::HashSet<(&TransportKind, &str)> {
        ts.iter()
            .filter(|t| t.active)
            .map(|t| (&t.kind, t.addr.as_str()))
            .collect()
    }
    let before_set = active_set(before);
    let after_set = active_set(after);
    if before_set == after_set {
        return None;
    }

    // Same kind-derivation order as `sort_transports` so the summary is
    // deterministic regardless of snapshot order.
    let mut kinds_added: Vec<TransportKind> = after_set
        .difference(&before_set)
        .map(|(kind, _)| **kind)
        .collect();
    let mut kinds_removed: Vec<TransportKind> = before_set
        .difference(&after_set)
        .map(|(kind, _)| **kind)
        .collect();
    kinds_added.sort();
    kinds_removed.sort();
    kinds_added.dedup();
    kinds_removed.dedup();

    Some(PathChangeSummary {
        before_active: before.iter().filter(|t| t.active).cloned().collect(),
        after_active: after.iter().filter(|t| t.active).cloned().collect(),
        kinds_added,
        kinds_removed,
    })
}

/// Render the *active* transports compactly for log lines:
/// `relay=<url>` / `direct=<addr>`, comma-separated — e.g.
/// `relay=https://use1-1.relay.iroh.network/, direct=192.168.1.10:52618`.
///
/// Inactive candidates are skipped: log lines describe the live data path;
/// the serve status file carries the full list-with-usage.
pub(crate) fn render_active_transports(transports: &[TransportStatus]) -> String {
    transports
        .iter()
        .filter(|t| t.active)
        .map(|t| format!("{}={}", t.kind, t.addr))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The active kind names of a snapshot, `+`-joined for the from/to clauses
/// of a path-change line: `relay`, `direct`, or `relay+direct`. An empty
/// active set renders as `none`.
pub(crate) fn render_active_kinds(transports: &[TransportStatus]) -> String {
    let mut kinds: Vec<TransportKind> = transports
        .iter()
        .filter(|t| t.active)
        .map(|t| t.kind)
        .collect();
    kinds.sort();
    kinds.dedup();
    if kinds.is_empty() {
        return "none".to_string();
    }
    kinds
        .iter()
        .map(|kind| kind.to_string())
        .collect::<Vec<_>>()
        .join("+")
}

/// One path-change poller tick as a pure function (#58): decide the log
/// line (if any) and the next baseline from the previous baseline and a
/// fresh snapshot.
///
/// Encodes the poller's three quiet rules:
/// - a baseline that has never seen an active transport seeds silently —
///   the first active snapshot is a resolution, not a migration (the
///   established line logged `paths pending`);
/// - an all-inactive snapshot means the connection is tearing down — the
///   disconnect event covers that, and the OLD baseline is kept so the
///   transient snapshot can never surface later as a spurious `none→…`
///   line;
/// - otherwise [`diff_transports`] decides; a real change renders exactly
///   one line, `path changed <kinds>→<kinds> (now active: <transports>)`.
pub(crate) fn poller_step(
    last: &[TransportStatus],
    snapshot: &[TransportStatus],
) -> (Option<String>, Vec<TransportStatus>) {
    let has_active = |ts: &[TransportStatus]| ts.iter().any(|t| t.active);
    if !has_active(last) {
        return (None, snapshot.to_vec());
    }
    if !has_active(snapshot) {
        return (None, last.to_vec());
    }
    match diff_transports(last, snapshot) {
        Some(change) => {
            let line = format!(
                "path changed {}→{} (now active: {})",
                render_active_kinds(&change.before_active),
                render_active_kinds(&change.after_active),
                render_active_transports(&change.after_active),
            );
            (Some(line), snapshot.to_vec())
        }
        None => (None, snapshot.to_vec()),
    }
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

    // ---- diff_transports + render helpers (issue #58) ----

    fn status(kind: TransportKind, addr: &str, active: bool) -> TransportStatus {
        TransportStatus {
            kind,
            addr: addr.to_string(),
            active,
        }
    }

    #[test]
    fn diff_returns_none_for_no_active_change() {
        let before = vec![
            status(
                TransportKind::Relay,
                "https://use1-1.relay.iroh.network/",
                true,
            ),
            status(TransportKind::Direct, "192.168.1.10:52618", false),
        ];
        // Same active set — different order, different inactive candidates.
        let after = vec![
            status(TransportKind::Direct, "192.168.1.10:9999", false),
            status(
                TransportKind::Relay,
                "https://use1-1.relay.iroh.network/",
                true,
            ),
        ];
        assert_eq!(diff_transports(&before, &after), None);
        // Byte-identical snapshots are the trivial no-change case.
        assert_eq!(diff_transports(&before, &before), None);
    }

    #[test]
    fn diff_reports_kind_gaining_active_transport() {
        // The classic hole-punch-succeeded transition: relay → relay+direct.
        let before = vec![status(TransportKind::Relay, "https://relay/", true)];
        let after = vec![
            status(TransportKind::Relay, "https://relay/", true),
            status(TransportKind::Direct, "203.0.113.7:41641", true),
        ];
        let change = diff_transports(&before, &after).unwrap();
        assert_eq!(change.kinds_added, vec![TransportKind::Direct]);
        assert!(change.kinds_removed.is_empty());
        assert_eq!(change.before_active.len(), 1);
        assert_eq!(change.after_active.len(), 2);
    }

    #[test]
    fn diff_reports_kind_losing_last_active_transport() {
        // The direct-path-died fallback: relay+direct → relay.
        let before = vec![
            status(TransportKind::Relay, "https://relay/", true),
            status(TransportKind::Direct, "203.0.113.7:41641", true),
        ];
        let after = vec![status(TransportKind::Relay, "https://relay/", true)];
        let change = diff_transports(&before, &after).unwrap();
        assert!(change.kinds_added.is_empty());
        assert_eq!(change.kinds_removed, vec![TransportKind::Direct]);
        assert_eq!(change.after_active.len(), 1);
    }

    #[test]
    fn diff_counts_active_flip_as_removal() {
        // Same address, but iroh stopped sending on it — a real fallback,
        // not a no-op, even though the candidate list looks unchanged.
        let before = vec![
            status(TransportKind::Relay, "https://relay/", true),
            status(TransportKind::Direct, "10.0.0.2:52618", true),
        ];
        let after = vec![
            status(TransportKind::Relay, "https://relay/", true),
            status(TransportKind::Direct, "10.0.0.2:52618", false),
        ];
        let change = diff_transports(&before, &after).unwrap();
        assert!(change.kinds_added.is_empty());
        assert_eq!(change.kinds_removed, vec![TransportKind::Direct]);
        assert_eq!(change.after_active.len(), 1);
    }

    #[test]
    fn diff_counts_addr_swap_inside_one_kind() {
        // Relay failover: the kind stays but the address changed — it is
        // both "added" and "removed".
        let before = vec![status(TransportKind::Relay, "https://a.relay/", true)];
        let after = vec![status(TransportKind::Relay, "https://b.relay/", true)];
        let change = diff_transports(&before, &after).unwrap();
        assert_eq!(change.kinds_added, vec![TransportKind::Relay]);
        assert_eq!(change.kinds_removed, vec![TransportKind::Relay]);
    }

    #[test]
    fn render_active_transports_skips_inactive_and_joins_with_comma() {
        let transports = vec![
            status(
                TransportKind::Relay,
                "https://use1-1.relay.iroh.network/",
                true,
            ),
            status(TransportKind::Direct, "192.168.1.10:52618", true),
            status(TransportKind::Direct, "192.168.1.10:9999", false),
        ];
        assert_eq!(
            render_active_transports(&transports),
            "relay=https://use1-1.relay.iroh.network/, direct=192.168.1.10:52618"
        );
        assert_eq!(render_active_transports(&[]), "");
    }

    #[test]
    fn render_active_kinds_lists_unique_kinds_or_none() {
        let both = vec![
            status(TransportKind::Direct, "10.0.0.2:52618", true),
            status(TransportKind::Relay, "https://r/", true),
            status(TransportKind::Relay, "https://r2/", true),
        ];
        assert_eq!(render_active_kinds(&both), "relay+direct");
        let relay_only = vec![status(TransportKind::Relay, "https://r/", true)];
        assert_eq!(render_active_kinds(&relay_only), "relay");
        let all_inactive = vec![status(TransportKind::Direct, "10.0.0.2:1", false)];
        assert_eq!(render_active_kinds(&all_inactive), "none");
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

    // ---- poller_step (issue #58 poller tick) ----

    #[test]
    fn poller_step_seeds_silently_when_baseline_never_active() {
        // `paths pending` at connect time: the first active snapshot is a
        // resolution, not a migration — no line, baseline moves forward.
        let last = vec![status(TransportKind::Relay, "https://r/", false)];
        let snapshot = vec![status(TransportKind::Relay, "https://r/", true)];
        let (line, next) = poller_step(&last, &snapshot);
        assert_eq!(line, None);
        assert_eq!(next, snapshot);
    }

    #[test]
    fn poller_step_keeps_baseline_on_all_inactive_snapshot() {
        // Teardown blip: the disconnect event covers death; the baseline
        // stays on the last real path.
        let last = vec![status(TransportKind::Relay, "https://r/", true)];
        let snapshot = vec![status(TransportKind::Relay, "https://r/", false)];
        let (line, next) = poller_step(&last, &snapshot);
        assert_eq!(line, None);
        assert_eq!(next, last);
    }

    #[test]
    fn poller_step_transient_inactive_then_real_change_is_one_line() {
        // relay active → all-inactive blip → direct active: the blip is
        // absorbed, so the operator sees exactly one relay→direct line —
        // never a spurious `none→direct` in between.
        let relay = vec![status(TransportKind::Relay, "https://r/", true)];
        let blip = vec![status(TransportKind::Relay, "https://r/", false)];
        let (line, last) = poller_step(&relay, &blip);
        assert_eq!(line, None);
        assert_eq!(last, relay);

        let direct = vec![status(TransportKind::Direct, "203.0.113.7:41641", true)];
        let (line, next) = poller_step(&last, &direct);
        assert_eq!(
            line.as_deref(),
            Some("path changed relay→direct (now active: direct=203.0.113.7:41641)")
        );
        assert_eq!(next, direct);
    }

    #[test]
    fn poller_step_returns_none_and_forward_baseline_on_no_change() {
        let last = vec![status(TransportKind::Relay, "https://r/", true)];
        // Same active set, different inactive candidate — quiet, but the
        // baseline still moves so inactive churn never accumulates.
        let snapshot = vec![
            status(TransportKind::Relay, "https://r/", true),
            status(TransportKind::Direct, "10.0.0.2:1", false),
        ];
        let (line, next) = poller_step(&last, &snapshot);
        assert_eq!(line, None);
        assert_eq!(next, snapshot);
    }
}
