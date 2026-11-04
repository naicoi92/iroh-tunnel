//! Runtime engine: endpoint binding, role loops, tunnels and their typed
//! settings/observations. Everything here is host-agnostic: the engine never
//! touches files, OS services, signals, `process::exit` or stdout/stderr.

pub mod access;
pub mod conn_path;
pub mod endpoint;
pub mod observation;
mod proto;
pub mod runtime;
pub mod serve;
pub mod settings;
mod shutdown;
mod tcp;
pub mod testing;
pub mod udp;

pub use access::{bind_access, run_access};
pub use runtime::{
    AccessReady, OnReady, ReadyReport, ReadyServices, RoleHooks, RunCompletion, RunContext,
    ServeReady, StatusSink,
};
pub use serve::{bind_serve, run_serve};
