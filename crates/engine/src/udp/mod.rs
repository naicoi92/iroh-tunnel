//! UDP datagram framing shared by Access and Serve (#88).
//!
//! Wire contract: `docs/udp-integration-contract.md`. Each tunnel
//! datagram is `[flow id: u32 big-endian][client payload]`, carried as one
//! QUIC DATAGRAM. The payload is opaque and may be empty. There is no
//! fragmentation: a payload larger than the connection's datagram limit
//! (maximum datagram size minus the header) is never sent.

mod carries_datagrams;
mod closed_for_unsupported_datagrams;
mod decode;
mod drain_queue;
mod encode;
mod is_transient_recv_error;
mod payload_limit;
mod redial_backoff;
mod send_decision;
mod sweep_interval;
mod types;

pub(crate) use carries_datagrams::{
    carries_datagrams, DATAGRAMS_UNSUPPORTED_CODE, DATAGRAMS_UNSUPPORTED_REASON,
};
pub(crate) use closed_for_unsupported_datagrams::closed_for_unsupported_datagrams;
pub use decode::decode;
pub use drain_queue::drain_queue;
pub use encode::encode;
pub use is_transient_recv_error::is_transient_recv_error;
pub use payload_limit::payload_limit;
pub(crate) use redial_backoff::{BackoffCheck, Incarnation, RedialBackoff};
pub(crate) use send_decision::{send_decision, SendDecision};
pub use sweep_interval::sweep_interval;
pub use types::{
    Admission, ClientFlows, DecodeError, DropReason, FlowId, FlowQueue, PeerFlows, Push, Refused,
    UdpStats, DEFAULT_MAX_FLOWS, DEFAULT_MAX_FLOWS_PER_PEER, DEFAULT_QUEUE_DEPTH, HEADER_LEN,
    IDLE_TIMEOUT, QUARANTINE, SWEEP_INTERVAL,
};
