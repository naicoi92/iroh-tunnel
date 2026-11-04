//! Host operations: everything file-, OS- and process-bound that a frontend
//! (the CLI today) drives — config editing, status files, role ownership,
//! service management, updating and the role runtime.
//!
//! Gathered in one module so the whole tree can move verbatim into its own
//! package: nothing here may reach outside `crate::host::` except the engine
//! package, and nothing here reads stdin, prints, installs signal handlers or
//! exits the process.

pub mod config;
pub mod config_ops;
pub(crate) mod config_store;
pub mod error;
pub(crate) mod ownership;
pub mod paths;
pub mod runtime;
pub mod service;
pub mod status;
pub mod status_read;
pub mod update;
pub mod update_owner;

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
