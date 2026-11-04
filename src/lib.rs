//! iroh-tunnel library: P2P port-forwarding tunnel (TCP/UDP) over Iroh.
//!
//! The crate is shaped as a library + binary pair so that integration tests
//! (under `tests/`) can drive the serve/access roles end-to-end without
//! spawning a subprocess. File-, OS- and process-bound operations live in the
//! [`host`] module (see its docs); the root keeps only the CLI around it.

pub mod app;
pub mod cli;
pub(crate) mod cli_service;
pub mod config_cmd;
pub mod host;
pub(crate) mod lifecycle;
pub(crate) mod render;
pub mod status_cmd;

// Old-path re-exports so tests and downstream callers keep compiling until
// the host and engine are their own crates (Task 20b rewires the tests).
pub use host::{config, error, paths, service, status, update, update_owner};
pub use iroh_tunnel_engine::{access, conn_path, endpoint, observation, udp};
