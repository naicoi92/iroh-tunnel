//! iroh-tunnel command-line frontend: argument parsing, prompts, rendering,
//! signals and exit-code mapping around the `iroh-tunnel-host` operations.
//!
//! The library target exists so the three binaries share one compiled copy of
//! the CLI and so `tests/access_status_file.rs` can check the status
//! renderers; it re-exports nothing from the host or engine packages.

pub mod app;
pub mod cli;
pub mod cli_args;
pub(crate) mod cli_service;
pub mod config_cmd;
pub(crate) mod lifecycle;
pub(crate) mod render;
pub mod status_cmd;
pub mod usage;
