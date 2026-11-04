//! iroh-tunnel library: P2P port-forwarding tunnel (TCP/UDP) over Iroh.
//!
//! The crate is shaped as a library + binary pair so that integration tests
//! (under `tests/`) can drive the serve/access roles end-to-end without
//! spawning a subprocess. The binary in [`main.rs`](../main.rs) is a thin
//! wrapper around [`serve::run`] / [`access::run`].
//!
//! Most modules carry `#![allow(dead_code)]` because the single-crate layout
//! would otherwise flag every symbol as unused by the binary alone. Exposing
//! them via the library target makes that suppression accurate: they ARE used,
//! by integration tests and by future downstream consumers.

pub mod access;
pub mod app;
pub mod cli;
pub(crate) mod cli_service;
pub mod config;
pub mod config_cmd;
pub(crate) mod config_store;
pub mod engine;
pub mod error;
pub(crate) mod ownership;
pub mod paths;
// Old-path re-exports so tests and downstream callers keep compiling until
// the engine becomes its own crate (Task 19b/20).
pub use crate::engine::{conn_path, endpoint, observation, pipe, proto, ready, udp};
pub(crate) mod render;
/// `pub(crate)` — internal seam between serve/access, not a public extension
/// point. See `src/role_run.rs` module docs.
pub(crate) mod role_run;
pub mod serve;
pub mod service;
pub mod shutdown;
pub mod status;
pub mod status_cmd;
pub mod update;
pub mod update_owner;
