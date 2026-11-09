#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links, rustdoc::private_intra_doc_links)]
//! Host operations: everything file-, OS- and process-bound that a frontend
//! (the CLI today) drives — config editing, status files, role ownership,
//! service management, updating and the role runtime.
//!
//! Nothing here reads stdin, prints, installs signal handlers or exits the
//! process; frontends own the terminal and map [`error::CliError`] to exit
//! codes. The only package below is `iroh-tunnel-engine`.

pub mod config;
pub mod error;
pub(crate) mod ownership;
pub mod paths;
pub mod role;
pub mod runtime;
pub mod service;
pub mod status;
pub mod update;

pub use role::Role;

pub use iroh_tunnel_engine::settings::Protocol;
/// Bridge re-exports (deviation R3): the runtime types the CLI hands to or
/// receives from the engine, reachable through the host so the root package
/// needs no direct engine dependency. `OnReady` is the hook type
/// [`runtime::run_foreground`] accepts.
pub use iroh_tunnel_engine::{
    AccessReady, OnReady, ReadyReport, ReadyServices, RunContext, ServeReady,
};

#[cfg(feature = "test-utils")]
pub mod testing;
