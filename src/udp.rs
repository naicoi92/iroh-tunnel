//! UDP datagram framing shared by Access and Serve (#88).
//!
//! Wire contract: `docs/design/udp-integration-contract.md`. Each tunnel
//! datagram is `[flow id: u32 big-endian][client payload]`, carried as one
//! QUIC DATAGRAM. The payload is opaque and may be empty. There is no
//! fragmentation: a payload larger than the connection's datagram limit
//! (maximum datagram size minus the header) is never sent.

use bytes::{BufMut, Bytes, BytesMut};

/// Bytes of the flow identifier that prefix every tunnel datagram.
pub const HEADER_LEN: usize = 4;

/// How long a flow may stay silent in both directions before it ends.
///
/// Minimal lifecycle for #88: the flow ends and its upstream socket closes at
/// once. Quarantine, caps and a configurable value follow in #89 (UDP-06).
pub const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Default flows one service may hold (Serve: per service; Access: per
/// service). An untested estimate (UDP-06).
pub const DEFAULT_MAX_FLOWS: usize = 256;

/// Default flows one peer may hold on a Serve service. An untested estimate.
pub const DEFAULT_MAX_FLOWS_PER_PEER: usize = 64;

/// Default datagrams queued per flow before the newest is dropped. An untested
/// estimate.
pub const DEFAULT_QUEUE_DEPTH: usize = 64;

/// Why a datagram was dropped (one counter each; see the contract's failure
/// table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropReason {
    Oversize,
    Malformed,
    UnknownFlow,
    UpstreamError,
    Cap,
    QueueFull,
    LimitShrink,
    QuarantineReply,
    ForeignSource,
}

/// Live counters of one UDP service, shared between the data path and the
/// status writer. Relaxed ordering: these are diagnostics, not synchronization.
#[derive(Debug, Default)]
pub struct UdpStats {
    /// Slots in use: live flows plus quarantined ones (what the caps count).
    held_flows: std::sync::atomic::AtomicU64,
    /// Of those, the flows whose socket is only a sink.
    quarantined_flows: std::sync::atomic::AtomicU64,
    datagram_limit: std::sync::atomic::AtomicU64,
    oversize: std::sync::atomic::AtomicU64,
    malformed: std::sync::atomic::AtomicU64,
    unknown_flow: std::sync::atomic::AtomicU64,
    upstream_error: std::sync::atomic::AtomicU64,
    cap: std::sync::atomic::AtomicU64,
    queue_full: std::sync::atomic::AtomicU64,
    limit_shrink: std::sync::atomic::AtomicU64,
    quarantine_reply: std::sync::atomic::AtomicU64,
    foreign_source: std::sync::atomic::AtomicU64,
}

/// Subtract `n` from `value`, stopping at zero.
fn sub_saturating(value: &std::sync::atomic::AtomicU64, n: u64) {
    use std::sync::atomic::Ordering::Relaxed;
    let mut now = value.load(Relaxed);
    loop {
        match value.compare_exchange_weak(now, now.saturating_sub(n), Relaxed, Relaxed) {
            Ok(_) => return,
            Err(actual) => now = actual,
        }
    }
}

impl UdpStats {
    fn counter(&self, reason: DropReason) -> &std::sync::atomic::AtomicU64 {
        match reason {
            DropReason::Oversize => &self.oversize,
            DropReason::Malformed => &self.malformed,
            DropReason::UnknownFlow => &self.unknown_flow,
            DropReason::UpstreamError => &self.upstream_error,
            DropReason::Cap => &self.cap,
            DropReason::QueueFull => &self.queue_full,
            DropReason::LimitShrink => &self.limit_shrink,
            DropReason::QuarantineReply => &self.quarantine_reply,
            DropReason::ForeignSource => &self.foreign_source,
        }
    }

    /// Count one dropped datagram.
    pub fn record_drop(&self, reason: DropReason) {
        self.counter(reason)
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Set the live flow count (Access owns one table per service).
    pub fn set_active_flows(&self, count: usize) {
        self.held_flows
            .store(count as u64, std::sync::atomic::Ordering::Relaxed);
    }

    /// Reserve one flow slot if fewer than `max` are in use.
    ///
    /// Serve calls this for every peer of a service, so the cap is shared and
    /// two peers cannot both take the last slot. Release with
    /// [`UdpStats::flows_closed`].
    pub fn try_open_flow(&self, max: usize) -> bool {
        use std::sync::atomic::Ordering::Relaxed;
        let mut now = self.held_flows.load(Relaxed);
        loop {
            if now >= max as u64 {
                return false;
            }
            match self
                .held_flows
                .compare_exchange_weak(now, now + 1, Relaxed, Relaxed)
            {
                Ok(_) => return true,
                Err(actual) => now = actual,
            }
        }
    }

    /// `count` flows ended and release their slots.
    pub fn flows_closed(&self, count: usize) {
        sub_saturating(&self.held_flows, count as u64);
    }

    /// A live flow expired: its socket stays open as a sink and keeps its slot.
    pub fn quarantine_started(&self) {
        self.quarantined_flows
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// A quarantined socket closed: its slot is free again.
    pub fn quarantine_ended(&self) {
        sub_saturating(&self.quarantined_flows, 1);
        sub_saturating(&self.held_flows, 1);
    }

    /// Record the connection's current payload limit; `None` when datagrams
    /// are unusable.
    pub fn set_datagram_limit(&self, limit: Option<usize>) {
        self.datagram_limit.store(
            limit.map_or(0, |l| l as u64),
            std::sync::atomic::Ordering::Relaxed,
        );
    }

    /// A point-in-time copy for the status file.
    pub fn snapshot(&self) -> crate::status::UdpStatus {
        use std::sync::atomic::Ordering::Relaxed;
        let limit = self.datagram_limit.load(Relaxed);
        crate::status::UdpStatus {
            active_flows: self
                .held_flows
                .load(Relaxed)
                .saturating_sub(self.quarantined_flows.load(Relaxed)),
            quarantined_flows: self.quarantined_flows.load(Relaxed),
            datagram_limit: (limit > 0).then_some(limit),
            drops: crate::status::UdpDrops {
                oversize: self.oversize.load(Relaxed),
                malformed: self.malformed.load(Relaxed),
                unknown_flow: self.unknown_flow.load(Relaxed),
                upstream_error: self.upstream_error.load(Relaxed),
                cap: self.cap.load(Relaxed),
                queue_full: self.queue_full.load(Relaxed),
                limit_shrink: self.limit_shrink.load(Relaxed),
                quarantine_reply: self.quarantine_reply.load(Relaxed),
                foreign_source: self.foreign_source.load(Relaxed),
            },
        }
    }
}

/// What happened to a datagram offered to a [`FlowQueue`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Push {
    /// Queued for the flow's writer.
    Queued,
    /// The queue was full; the newest datagram (this one) was dropped.
    Full,
    /// The flow's writer has gone; the flow is ending.
    Closed,
}

/// The bounded queue between the shared receive loop and one flow's writer.
///
/// A flow whose writer is slow fills only its own queue, so it cannot delay
/// the receive loop or any other flow: [`FlowQueue::push`] never waits. When
/// full the newest datagram is dropped, which keeps arrival order for what is
/// delivered (UDP-06).
#[derive(Debug, Clone)]
pub struct FlowQueue {
    tx: tokio::sync::mpsc::Sender<Bytes>,
}

impl FlowQueue {
    /// A queue holding up to `depth` datagrams, and the receiving end for the
    /// flow's writer task.
    pub fn new(depth: usize) -> (Self, tokio::sync::mpsc::Receiver<Bytes>) {
        let (tx, rx) = tokio::sync::mpsc::channel(depth.max(1));
        (Self { tx }, rx)
    }

    /// Offer a datagram without waiting.
    pub fn push(&self, datagram: Bytes) -> Push {
        use tokio::sync::mpsc::error::TrySendError;
        match self.tx.try_send(datagram) {
            Ok(()) => Push::Queued,
            Err(TrySendError::Full(_)) => Push::Full,
            Err(TrySendError::Closed(_)) => Push::Closed,
        }
    }
}

/// Run one flow's writer: hand each queued datagram to `sink` in order, until
/// every [`FlowQueue`] handle is dropped.
///
/// `sink` may wait (a slow target); only this flow's queue backs up behind it.
pub async fn drain_queue<F, Fut>(mut rx: tokio::sync::mpsc::Receiver<Bytes>, mut sink: F)
where
    F: FnMut(Bytes) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    while let Some(datagram) = rx.recv().await {
        sink(datagram).await;
    }
}

/// How long an expired flow's upstream socket stays open as a sink before it
/// closes and its slot is released (UDP-06). A fixed constant, not configurable.
pub const QUARANTINE: std::time::Duration = std::time::Duration::from_secs(30);

/// How often each side looks for idle flows.
pub const SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

/// The sweep period for a service with this idle timeout: never longer than
/// the timeout itself, so a short timeout is honoured promptly.
pub fn sweep_interval(idle: std::time::Duration) -> std::time::Duration {
    SWEEP_INTERVAL
        .min(idle)
        .max(std::time::Duration::from_millis(100))
}

/// Identifier of one flow, assigned by Access within a session incarnation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FlowId(pub u32);

/// Why a tunnel datagram could not be decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// Shorter than the 4-byte header.
    Malformed,
}

/// Frame `payload` for `flow`.
pub fn encode(flow: FlowId, payload: &[u8]) -> Bytes {
    let mut out = BytesMut::with_capacity(HEADER_LEN + payload.len());
    out.put_u32(flow.0);
    out.put_slice(payload);
    out.freeze()
}

/// Split a tunnel datagram into its flow and payload.
pub fn decode(datagram: &Bytes) -> Result<(FlowId, Bytes), DecodeError> {
    if datagram.len() < HEADER_LEN {
        return Err(DecodeError::Malformed);
    }
    let flow = u32::from_be_bytes([datagram[0], datagram[1], datagram[2], datagram[3]]);
    Ok((FlowId(flow), datagram.slice(HEADER_LEN..)))
}

/// Whether a receive error on a UDP socket only reports a lost datagram.
///
/// A connected socket reports the target's ICMP answer (port unreachable) on a
/// later receive. Windows reports it on unconnected sockets too, as
/// `WSAECONNRESET` (os error 10054), after a send to a port nobody listens on.
/// Neither means the socket is dead.
pub fn is_transient_recv_error(error: &std::io::Error) -> bool {
    use std::io::ErrorKind::{
        ConnectionAborted, ConnectionRefused, ConnectionReset, Interrupted, WouldBlock,
    };
    matches!(
        error.kind(),
        ConnectionRefused | ConnectionReset | ConnectionAborted | Interrupted | WouldBlock
    )
}

/// Largest client payload a connection can carry now, or `None` when the
/// connection cannot carry datagrams at all (unsupported, disabled, or a
/// maximum no larger than the header, per the contract).
pub fn payload_limit(max_datagram_size: Option<usize>) -> Option<usize> {
    max_datagram_size
        .and_then(|max| max.checked_sub(HEADER_LEN))
        .filter(|limit| *limit > 0)
}

/// Client endpoints and the flow of each, for one session incarnation.
///
/// Identifiers only increase and are never reused, so a datagram for an
/// expired flow can never be mistaken for a new one (contract).
#[derive(Default)]
pub struct ClientFlows {
    by_client: std::collections::HashMap<std::net::SocketAddr, FlowId>,
    by_flow: std::collections::HashMap<FlowId, (std::net::SocketAddr, std::time::Instant)>,
    next: u32,
}

/// Why Access could not give a client a flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// The service already holds `max_flows` flows. Nothing running is evicted.
    Cap,
    /// Every 32-bit flow identifier of this incarnation is spent. The caller
    /// must end the incarnation; identifiers are never reused inside it.
    Exhausted,
}

impl ClientFlows {
    /// The flow of `client`, created on its first datagram, and refresh its
    /// idle timer. A client that already has a flow is never refused; a new
    /// one is refused at `max_flows` live flows ([`Refused::Cap`]) or once the
    /// identifier space is spent ([`Refused::Exhausted`]).
    pub fn touch(
        &mut self,
        client: std::net::SocketAddr,
        now: std::time::Instant,
        max_flows: usize,
    ) -> Result<FlowId, Refused> {
        let flow = match self.by_client.get(&client) {
            Some(flow) => *flow,
            None => {
                if self.by_flow.len() >= max_flows {
                    return Err(Refused::Cap);
                }
                let flow = FlowId(self.next);
                self.next = self.next.checked_add(1).ok_or(Refused::Exhausted)?;
                self.by_client.insert(client, flow);
                flow
            }
        };
        self.by_flow.insert(flow, (client, now));
        Ok(flow)
    }

    /// The client of `flow`, refreshing its idle timer; `None` if unknown or
    /// already expired.
    pub fn client_of(
        &mut self,
        flow: FlowId,
        now: std::time::Instant,
    ) -> Option<std::net::SocketAddr> {
        let entry = self.by_flow.get_mut(&flow)?;
        entry.1 = now;
        Some(entry.0)
    }

    /// Flows currently live.
    pub fn len(&self) -> usize {
        self.by_flow.len()
    }

    /// Whether no flow is live.
    pub fn is_empty(&self) -> bool {
        self.by_flow.is_empty()
    }

    /// End every flow idle for at least `idle`; returns how many ended.
    pub fn expire(&mut self, now: std::time::Instant, idle: std::time::Duration) -> usize {
        let expired: Vec<FlowId> = self
            .by_flow
            .iter()
            .filter(|(_, (_, seen))| now.duration_since(*seen) >= idle)
            .map(|(flow, _)| *flow)
            .collect();
        for flow in &expired {
            if let Some((client, _)) = self.by_flow.remove(flow) {
                self.by_client.remove(&client);
            }
        }
        expired.len()
    }
}

/// Serve's live flows for one peer connection, holding one `T` (the upstream
/// socket) per flow.
///
/// A flow is admitted only if its identifier is higher than every identifier
/// already seen from this peer in this incarnation. An identifier at or below
/// that mark with no live flow (expired or never admitted) is refused, so an
/// expired flow is never rebound even if the two idle timers drift apart.
pub struct PeerFlows<T> {
    live: std::collections::HashMap<FlowId, (T, std::time::Instant)>,
    highest: Option<FlowId>,
}

/// What Serve should do with a datagram for a flow.
#[derive(Debug, PartialEq, Eq)]
pub enum Admission {
    /// A live flow.
    Live,
    /// A new, admittable flow: the caller opens its upstream and `insert`s it.
    New,
    /// Not live and not admittable.
    Unknown,
}

impl<T> Default for PeerFlows<T> {
    fn default() -> Self {
        Self {
            live: std::collections::HashMap::new(),
            highest: None,
        }
    }
}

impl<T> PeerFlows<T> {
    /// Classify `flow`; a live flow also has its idle timer refreshed.
    pub fn admit(&mut self, flow: FlowId, now: std::time::Instant) -> Admission {
        if let Some(entry) = self.live.get_mut(&flow) {
            entry.1 = now;
            return Admission::Live;
        }
        if self.highest.is_some_and(|mark| flow <= mark) {
            return Admission::Unknown;
        }
        Admission::New
    }

    /// Record that `flow` was refused (cap reached or no socket could be
    /// opened). Its identifier is spent: later datagrams for it are
    /// [`Admission::Unknown`], never admitted mid-flow.
    pub fn refuse(&mut self, flow: FlowId) {
        self.highest = Some(self.highest.map_or(flow, |mark| mark.max(flow)));
    }

    /// Register the upstream of a flow that `admit` returned `New` for.
    pub fn insert(&mut self, flow: FlowId, upstream: T, now: std::time::Instant) {
        self.highest = Some(self.highest.map_or(flow, |mark| mark.max(flow)));
        self.live.insert(flow, (upstream, now));
    }

    /// Flows currently live.
    pub fn len(&self) -> usize {
        self.live.len()
    }

    /// Whether no flow is live.
    pub fn is_empty(&self) -> bool {
        self.live.is_empty()
    }

    /// The upstream of a live flow.
    pub fn get(&self, flow: FlowId) -> Option<&T> {
        self.live.get(&flow).map(|(upstream, _)| upstream)
    }

    /// Refresh the idle timer of a live flow (a reply counts as traffic).
    pub fn refresh(&mut self, flow: FlowId, now: std::time::Instant) {
        if let Some(entry) = self.live.get_mut(&flow) {
            entry.1 = now;
        }
    }

    /// Take every live flow's upstream, ending them all (the connection is
    /// gone).
    pub fn drain_all(&mut self) -> Vec<T> {
        self.live
            .drain()
            .map(|(_, (upstream, _))| upstream)
            .collect()
    }

    /// End every flow idle for at least `idle`, returning their upstreams so
    /// the caller can close them.
    pub fn expire(&mut self, now: std::time::Instant, idle: std::time::Duration) -> Vec<T> {
        let expired: Vec<FlowId> = self
            .live
            .iter()
            .filter(|(_, (_, seen))| now.duration_since(*seen) >= idle)
            .map(|(flow, _)| *flow)
            .collect();
        expired
            .into_iter()
            .filter_map(|flow| self.live.remove(&flow).map(|(upstream, _)| upstream))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use std::time::{Duration, Instant};

    fn client(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }

    #[test]
    fn encode_then_decode_returns_flow_and_unchanged_payload() {
        let framed = encode(FlowId(0x0102_0304), b"payload");
        assert_eq!(&framed[..HEADER_LEN], &[1, 2, 3, 4], "big-endian header");
        let (flow, payload) = decode(&framed).unwrap();
        assert_eq!(flow, FlowId(0x0102_0304));
        assert_eq!(payload.as_ref(), b"payload");
    }

    #[test]
    fn an_empty_payload_is_valid_and_stays_empty() {
        let (flow, payload) = decode(&encode(FlowId(7), b"")).unwrap();
        assert_eq!(flow, FlowId(7));
        assert!(payload.is_empty());
    }

    #[test]
    fn a_datagram_shorter_than_the_header_is_malformed() {
        for len in 0..HEADER_LEN {
            let short = Bytes::from(vec![0u8; len]);
            assert_eq!(decode(&short), Err(DecodeError::Malformed), "len {len}");
        }
    }

    #[test]
    fn payload_limit_subtracts_the_header() {
        assert_eq!(payload_limit(Some(1162)), Some(1158));
        assert_eq!(payload_limit(Some(HEADER_LEN + 1)), Some(1));
    }

    #[test]
    fn payload_limit_is_none_when_datagrams_are_unusable() {
        assert_eq!(payload_limit(None), None);
        assert_eq!(payload_limit(Some(HEADER_LEN - 1)), None);
        assert_eq!(payload_limit(Some(HEADER_LEN)), None, "contract: 4 or less");
    }

    #[test]
    fn a_client_keeps_its_flow_while_active() {
        let t0 = Instant::now();
        let mut flows = ClientFlows::default();
        let first = flows.touch(client(1), t0, usize::MAX).unwrap();
        assert_eq!(
            flows.touch(client(1), t0 + Duration::from_secs(30), usize::MAX),
            Ok(first)
        );
        assert_eq!(flows.expire(t0 + Duration::from_secs(80), IDLE_TIMEOUT), 0);
    }

    #[test]
    fn traffic_in_either_direction_refreshes_the_idle_timer() {
        let t0 = Instant::now();
        let mut flows = ClientFlows::default();
        let flow = flows.touch(client(1), t0, usize::MAX).unwrap();
        // A reply from the target at t0+50s refreshes it.
        assert!(flows
            .client_of(flow, t0 + Duration::from_secs(50))
            .is_some());
        assert_eq!(flows.expire(t0 + Duration::from_secs(100), IDLE_TIMEOUT), 0);
        assert_eq!(flows.expire(t0 + Duration::from_secs(110), IDLE_TIMEOUT), 1);
    }

    #[test]
    fn an_expired_flow_is_gone_and_its_identifier_is_never_reused() {
        let t0 = Instant::now();
        let mut flows = ClientFlows::default();
        let old = flows.touch(client(1), t0, usize::MAX).unwrap();
        assert_eq!(flows.expire(t0 + IDLE_TIMEOUT, IDLE_TIMEOUT), 1);
        assert_eq!(flows.client_of(old, t0 + IDLE_TIMEOUT), None);
        let renewed = flows
            .touch(client(1), t0 + IDLE_TIMEOUT, usize::MAX)
            .unwrap();
        assert!(
            renewed > old,
            "a returning client gets a new, higher flow id"
        );
    }

    #[test]
    fn a_new_client_is_refused_at_the_cap_but_a_known_client_is_not() {
        let t0 = Instant::now();
        let mut flows = ClientFlows::default();
        let a = flows.touch(client(1), t0, 2).unwrap();
        flows.touch(client(2), t0, 2).unwrap();
        assert_eq!(flows.touch(client(3), t0, 2), Err(Refused::Cap));
        // Existing flows keep working, and keep their identifier.
        assert_eq!(flows.touch(client(1), t0, 2), Ok(a));
        assert_eq!(flows.len(), 2);
    }

    #[test]
    fn an_expired_flow_frees_its_slot_under_the_cap() {
        let t0 = Instant::now();
        let mut flows = ClientFlows::default();
        flows.touch(client(1), t0, 1).unwrap();
        assert_eq!(flows.touch(client(2), t0, 1), Err(Refused::Cap));
        assert_eq!(flows.expire(t0 + IDLE_TIMEOUT, IDLE_TIMEOUT), 1);
        assert!(flows.touch(client(2), t0 + IDLE_TIMEOUT, 1).is_ok());
    }

    #[test]
    fn identifier_exhaustion_is_reported_not_wrapped() {
        let mut flows = ClientFlows {
            next: u32::MAX,
            ..Default::default()
        };
        let t0 = Instant::now();
        // The counter cannot advance past u32::MAX, so no identifier is issued
        // and the caller must end the incarnation.
        assert_eq!(
            flows.touch(client(1), t0, usize::MAX),
            Err(Refused::Exhausted)
        );
    }

    #[test]
    fn serve_admits_only_increasing_identifiers() {
        let t0 = Instant::now();
        let mut flows: PeerFlows<&str> = PeerFlows::default();
        assert_eq!(flows.admit(FlowId(5), t0), Admission::New);
        flows.insert(FlowId(5), "five", t0);
        assert_eq!(flows.admit(FlowId(5), t0), Admission::Live);
        // Lower and equal-to-mark ids without a live flow are never admitted.
        assert_eq!(flows.admit(FlowId(3), t0), Admission::Unknown);
        assert_eq!(flows.admit(FlowId(6), t0), Admission::New);
    }

    #[test]
    fn serve_never_rebinds_an_expired_identifier() {
        let t0 = Instant::now();
        let mut flows: PeerFlows<&str> = PeerFlows::default();
        flows.insert(FlowId(1), "one", t0);
        let closed = flows.expire(t0 + IDLE_TIMEOUT, IDLE_TIMEOUT);
        assert_eq!(closed, vec!["one"]);
        assert_eq!(flows.get(FlowId(1)), None);
        // Access still sends id 1 because its own timer has not fired yet.
        assert_eq!(
            flows.admit(FlowId(1), t0 + IDLE_TIMEOUT),
            Admission::Unknown
        );
    }

    #[test]
    fn a_reply_refreshes_serve_idle_timer() {
        let t0 = Instant::now();
        let mut flows: PeerFlows<&str> = PeerFlows::default();
        flows.insert(FlowId(1), "one", t0);
        flows.refresh(FlowId(1), t0 + Duration::from_secs(50));
        assert!(flows
            .expire(t0 + Duration::from_secs(100), IDLE_TIMEOUT)
            .is_empty());
        assert_eq!(
            flows
                .expire(t0 + Duration::from_secs(110), IDLE_TIMEOUT)
                .len(),
            1
        );
    }

    #[test]
    fn icmp_style_receive_errors_are_transient() {
        use std::io::{Error, ErrorKind};
        for kind in [
            ErrorKind::ConnectionRefused,
            ErrorKind::ConnectionReset,
            ErrorKind::ConnectionAborted,
        ] {
            assert!(is_transient_recv_error(&Error::from(kind)), "{kind:?}");
        }
        assert!(!is_transient_recv_error(&Error::from(
            ErrorKind::PermissionDenied
        )));
        assert!(!is_transient_recv_error(&Error::from(
            ErrorKind::InvalidInput
        )));
    }

    #[test]
    fn stats_count_each_reason_separately_and_report_the_limit() {
        let stats = UdpStats::default();
        stats.record_drop(DropReason::Oversize);
        stats.record_drop(DropReason::Oversize);
        stats.record_drop(DropReason::Cap);
        stats.set_datagram_limit(Some(1158));
        assert!(stats.try_open_flow(10));
        assert!(stats.try_open_flow(10));
        stats.flows_closed(1);
        let snap = stats.snapshot();
        assert_eq!(
            (snap.drops.oversize, snap.drops.cap, snap.drops.malformed),
            (2, 1, 0)
        );
        assert_eq!(snap.datagram_limit, Some(1158));
        assert_eq!(snap.active_flows, 1);
    }

    #[test]
    fn closing_more_flows_than_are_open_does_not_underflow() {
        let stats = UdpStats::default();
        assert!(stats.try_open_flow(10));
        stats.flows_closed(5);
        assert_eq!(stats.snapshot().active_flows, 0);
    }

    #[test]
    fn an_unusable_connection_reports_no_datagram_limit() {
        let stats = UdpStats::default();
        stats.set_datagram_limit(Some(1158));
        stats.set_datagram_limit(None);
        assert_eq!(stats.snapshot().datagram_limit, None);
    }

    #[test]
    fn a_refused_flow_is_never_admitted_later() {
        let t0 = Instant::now();
        let mut flows: PeerFlows<&str> = PeerFlows::default();
        assert_eq!(flows.admit(FlowId(4), t0), Admission::New);
        flows.refuse(FlowId(4));
        assert_eq!(flows.admit(FlowId(4), t0), Admission::Unknown);
        assert_eq!(flows.admit(FlowId(5), t0), Admission::New);
    }

    #[test]
    fn the_service_cap_is_shared_and_released() {
        let stats = UdpStats::default();
        assert!(stats.try_open_flow(2));
        assert!(stats.try_open_flow(2));
        assert!(!stats.try_open_flow(2), "third slot must be refused");
        stats.flows_closed(1);
        assert!(stats.try_open_flow(2), "a freed slot can be taken again");
    }

    #[test]
    fn a_quarantined_flow_keeps_its_slot_until_the_quarantine_ends() {
        let stats = UdpStats::default();
        assert!(stats.try_open_flow(1));
        stats.quarantine_started();
        let snap = stats.snapshot();
        assert_eq!((snap.active_flows, snap.quarantined_flows), (0, 1));
        assert!(!stats.try_open_flow(1), "the slot is still held");
        stats.quarantine_ended();
        assert_eq!(stats.snapshot().quarantined_flows, 0);
        assert!(
            stats.try_open_flow(1),
            "the slot is free once quarantine ends"
        );
    }

    #[test]
    fn drain_all_returns_every_live_upstream() {
        let t0 = Instant::now();
        let mut flows: PeerFlows<&str> = PeerFlows::default();
        flows.insert(FlowId(1), "one", t0);
        flows.insert(FlowId(2), "two", t0);
        let mut all = flows.drain_all();
        all.sort();
        assert_eq!(all, vec!["one", "two"]);
        assert!(flows.is_empty());
        assert_eq!(
            flows.admit(FlowId(2), t0),
            Admission::Unknown,
            "ids stay spent"
        );
    }

    #[test]
    fn the_sweep_never_waits_longer_than_the_idle_timeout() {
        assert_eq!(sweep_interval(Duration::from_secs(60)), SWEEP_INTERVAL);
        assert_eq!(
            sweep_interval(Duration::from_secs(1)),
            Duration::from_secs(1)
        );
        assert_eq!(
            sweep_interval(Duration::from_millis(1)),
            Duration::from_millis(100)
        );
    }

    #[tokio::test]
    async fn a_full_flow_queue_drops_the_newest_and_keeps_the_oldest() {
        let (queue, mut rx) = FlowQueue::new(2);
        assert_eq!(queue.push(Bytes::from_static(b"one")), Push::Queued);
        assert_eq!(queue.push(Bytes::from_static(b"two")), Push::Queued);
        assert_eq!(queue.push(Bytes::from_static(b"three")), Push::Full);
        assert_eq!(rx.recv().await.unwrap().as_ref(), b"one");
        assert_eq!(rx.recv().await.unwrap().as_ref(), b"two");
        // Space is free again once the consumer has caught up.
        assert_eq!(queue.push(Bytes::from_static(b"four")), Push::Queued);
    }

    #[tokio::test]
    async fn a_stuck_flow_does_not_block_the_queue_of_another_flow() {
        let (slow, _never_read) = FlowQueue::new(1);
        let (fast, mut fast_rx) = FlowQueue::new(1);
        assert_eq!(slow.push(Bytes::from_static(b"a")), Push::Queued);
        // The slow flow is full and refuses more, without waiting.
        assert_eq!(slow.push(Bytes::from_static(b"b")), Push::Full);
        // The other flow is unaffected.
        assert_eq!(fast.push(Bytes::from_static(b"x")), Push::Queued);
        assert_eq!(fast_rx.recv().await.unwrap().as_ref(), b"x");
    }

    #[tokio::test]
    async fn pushing_to_a_flow_whose_writer_is_gone_reports_closed() {
        let (queue, rx) = FlowQueue::new(1);
        drop(rx);
        assert_eq!(queue.push(Bytes::from_static(b"x")), Push::Closed);
    }

    #[tokio::test]
    async fn the_flow_writer_drains_its_queue_in_order_through_the_sink() {
        let (queue, rx) = FlowQueue::new(4);
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink_seen = seen.clone();
        let writer = tokio::spawn(drain_queue(rx, move |datagram: Bytes| {
            let sink_seen = sink_seen.clone();
            async move { sink_seen.lock().unwrap().push(datagram) }
        }));
        for word in [&b"a"[..], b"b", b"c"] {
            assert_eq!(queue.push(Bytes::copy_from_slice(word)), Push::Queued);
        }
        drop(queue);
        writer.await.unwrap();
        let got: Vec<_> = seen.lock().unwrap().iter().map(|b| b.to_vec()).collect();
        assert_eq!(got, vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]);
    }

    #[tokio::test]
    async fn a_stuck_sink_fills_only_its_own_queue_and_never_blocks_the_producer() {
        let gate = std::sync::Arc::new(tokio::sync::Notify::new());
        let (stuck, rx) = FlowQueue::new(2);
        let sink_gate = gate.clone();
        let writer = tokio::spawn(drain_queue(rx, move |_datagram: Bytes| {
            let sink_gate = sink_gate.clone();
            async move { sink_gate.notified().await }
        }));
        // The sink holds the first datagram, two more fill the queue, the rest
        // are refused at once: the producer never waits on the stuck flow.
        let mut full = 0;
        for _ in 0..10 {
            if stuck.push(Bytes::from_static(b"x")) == Push::Full {
                full += 1;
            }
            tokio::task::yield_now().await;
        }
        assert!(full >= 7, "expected most pushes refused, got {full}");
        // Another flow's queue is untouched by the stuck one.
        let (other, mut other_rx) = FlowQueue::new(2);
        assert_eq!(other.push(Bytes::from_static(b"y")), Push::Queued);
        assert_eq!(other_rx.recv().await.unwrap().as_ref(), b"y");
        writer.abort();
    }
}
