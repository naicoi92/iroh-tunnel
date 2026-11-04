//! UDP datagram framing shared by Access and Serve (#88).
//!
//! Wire contract: `docs/udp-integration-contract.md`. Each tunnel
//! datagram is `[flow id: u32 big-endian][client payload]`, carried as one
//! QUIC DATAGRAM. The payload is opaque and may be empty. There is no
//! fragmentation: a payload larger than the connection's datagram limit
//! (maximum datagram size minus the header) is never sent.

mod decode;
mod drain_queue;
mod encode;
mod is_transient_recv_error;
mod payload_limit;
mod sweep_interval;
mod types;

pub use decode::decode;
pub use drain_queue::drain_queue;
pub use encode::encode;
pub use is_transient_recv_error::is_transient_recv_error;
pub use payload_limit::payload_limit;
pub use sweep_interval::sweep_interval;
pub use types::{
    Admission, ClientFlows, DecodeError, DropReason, FlowId, FlowQueue, PeerFlows, Push, Refused,
    UdpStats, DEFAULT_MAX_FLOWS, DEFAULT_MAX_FLOWS_PER_PEER, DEFAULT_QUEUE_DEPTH, HEADER_LEN,
    IDLE_TIMEOUT, QUARANTINE, SWEEP_INTERVAL,
};
