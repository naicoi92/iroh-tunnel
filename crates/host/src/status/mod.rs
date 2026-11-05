//! Status files: the per-role JSON snapshot a running node writes, and the
//! read-only classification of it (`<role> status`).
//!
//! `<state dir>/<role>-status.json`, where the state dir is
//! [`crate::paths::default_state_dir`] unless `IROH_TUNNEL_STATE_DIR` names
//! another directory. The write mechanics (`writer`) are role-agnostic; the
//! schemas (`schema`) are role-specific.

#[cfg(test)]
mod fixtures;
mod read_role_status;
mod schema;
mod types;
pub(crate) mod writer;

pub use crate::ownership::{describe_owner, Liveness, OwnerRecord, Tier};
pub use iroh_tunnel_engine::conn_path::short_peer_id;
pub use read_role_status::read_role_status;
pub use schema::{
    AccessServiceStatus, AccessStatusFile, PeerConnectionStatus, PeerPathReport, ServiceStatus,
    StatusFile, StatusPayload, TransportKind, TransportStatus, UdpDrops, UdpStatus,
};
pub use types::{Freshness, RoleStatus, Snapshot};
