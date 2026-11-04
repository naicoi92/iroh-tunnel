//! Pure path-change diffing and log rendering (access logs, issue #58).

use crate::observation::{PeerPathObservation, TransportKind, TransportObservation};

// ---------------------------------------------------------------------------
// Path-change diff + log rendering (access logs, issue #58)
// ---------------------------------------------------------------------------

/// What changed between two transport snapshots, as computed by
/// [`diff_transports`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PathChangeSummary {
    /// Active transports before the change, in snapshot order.
    pub before_active: Vec<TransportObservation>,
    /// Active transports after the change, in snapshot order.
    pub after_active: Vec<TransportObservation>,
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
    before: &[TransportObservation],
    after: &[TransportObservation],
) -> Option<PathChangeSummary> {
    // A nested fn (not a closure): the borrowed-tuple return needs true
    // for<'a> universality, which closure lifetime elision cannot express.
    fn active_set(
        ts: &[TransportObservation],
    ) -> std::collections::HashSet<(&TransportKind, &str)> {
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
pub(crate) fn render_active_transports(transports: &[TransportObservation]) -> String {
    transports
        .iter()
        .filter(|t| t.active)
        .map(|t| format!("{}={}", t.kind, t.addr))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Render a fresh connection's active transports for its established log
/// line — or `paths pending` when iroh has no active-path snapshot yet
/// (queried immediately after the handshake).
///
/// Shared by both roles' connect lines: access dials in
/// (`connected to serve peer … via <this>`) and serve accepts
/// (`peer connected via <this>`).
pub(crate) fn render_established_paths(report: Option<&PeerPathObservation>) -> String {
    report
        .map(|r| render_active_transports(&r.transports))
        .filter(|rendered| !rendered.is_empty())
        .unwrap_or_else(|| "paths pending".to_string())
}

/// The active kind names of a snapshot, `+`-joined for the from/to clauses
/// of a path-change line: `relay`, `direct`, or `relay+direct`. An empty
/// active set renders as `none`.
pub(crate) fn render_active_kinds(transports: &[TransportObservation]) -> String {
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
///
/// `snapshot` is taken by value (the caller owns `report.transports` and
/// never reuses it), so every branch except the teardown keep returns the
/// moved vec intact — no per-tick clones.
pub(crate) fn poller_step(
    last: &[TransportObservation],
    snapshot: Vec<TransportObservation>,
) -> (Option<String>, Vec<TransportObservation>) {
    let has_active = |ts: &[TransportObservation]| ts.iter().any(|t| t.active);
    if !has_active(last) {
        return (None, snapshot);
    }
    if !has_active(&snapshot) {
        return (None, last.to_vec());
    }
    match diff_transports(last, &snapshot) {
        Some(change) => {
            let line = format!(
                "path changed {}→{} (now active: {})",
                render_active_kinds(&change.before_active),
                render_active_kinds(&change.after_active),
                render_active_transports(&change.after_active),
            );
            (Some(line), snapshot)
        }
        None => (None, snapshot),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- diff_transports + render helpers (issue #58) ----

    fn status(kind: TransportKind, addr: &str, active: bool) -> TransportObservation {
        TransportObservation {
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
    fn transport_kind_display_is_lowercase() {
        assert_eq!(TransportKind::Relay.to_string(), "relay");
        assert_eq!(TransportKind::Direct.to_string(), "direct");
    }

    // ---- poller_step (issue #58 poller tick) ----

    #[test]
    fn poller_step_seeds_silently_when_baseline_never_active() {
        // `paths pending` at connect time: the first active snapshot is a
        // resolution, not a migration — no line, baseline moves forward
        // (the moved snapshot, no clone).
        let last = vec![status(TransportKind::Relay, "https://r/", false)];
        let snapshot = vec![status(TransportKind::Relay, "https://r/", true)];
        let (line, next) = poller_step(&last, snapshot.clone());
        assert_eq!(line, None);
        assert_eq!(next, snapshot);
    }

    #[test]
    fn poller_step_keeps_baseline_on_all_inactive_snapshot() {
        // Teardown blip: the disconnect event covers death; the baseline
        // stays on the last real path (the only branch that clones).
        let last = vec![status(TransportKind::Relay, "https://r/", true)];
        let snapshot = vec![status(TransportKind::Relay, "https://r/", false)];
        let (line, next) = poller_step(&last, snapshot);
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
        let (line, last) = poller_step(&relay, blip);
        assert_eq!(line, None);
        assert_eq!(last, relay);

        let direct = vec![status(TransportKind::Direct, "203.0.113.7:41641", true)];
        let (line, next) = poller_step(&last, direct.clone());
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
        let (line, next) = poller_step(&last, snapshot.clone());
        assert_eq!(line, None);
        assert_eq!(next, snapshot);
    }
}
