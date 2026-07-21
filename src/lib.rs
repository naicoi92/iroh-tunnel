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
pub mod cli;
pub mod config;
pub mod config_cmd;
pub mod endpoint;
pub mod error;
pub mod pipe;
pub mod proto;
pub mod role_run;
pub mod serve;
pub mod service;
pub mod shutdown;
pub mod status;
