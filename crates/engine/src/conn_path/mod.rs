//! Peer connection-path reporting for status files and access logs.
//!
//! Shared seam between the serve status file (`serve-status.json`, issue
//! #57) and the access-side connection-path logs (issue #58): turns iroh's
//! live endpoint state into a small snapshot of *how* a peer
//! is currently reachable.
//!
//! Two layers, deliberately split:
//!
//! - [`peer_path_report`] — the async query against a live [`iroh::Endpoint`]
//!   (`Endpoint::remote_info` snapshot + `Endpoint::bound_sockets`).
//! - `classify_transport` / `sort_transports` / `diff_transports` /
//!   `poller_step` — pure mappers, unit-testable without an endpoint.
//!
//! ## Semantics
//!
//! iroh negotiates relay and direct paths concurrently, so the transport list
//! is a *list with usage*, not a single mode: a peer can be direct-only (hole
//! punching failed), relay-only, or have both active at once (direct known and
//! reachable, relay kept as fallback). `TransportObservation::active` mirrors
//! iroh's own `TransportAddrUsage` snapshot at query time.
//!
//! `local_bound_addrs` are the endpoint's local UDP socket *candidates* (one
//! per address family the endpoint bound) — iroh 1.0 does not expose a
//! local-address-per-path mapping, so these are NOT paired with remote
//! addresses and must not be read as "the local addr of this transport".

mod paths;
mod peer_path_report;
mod short_peer_id;

pub(crate) use paths::{poller_step, render_established_paths};
pub use peer_path_report::peer_path_report;
pub use short_peer_id::short_peer_id;
