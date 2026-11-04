//! Runtime engine: endpoint binding, role loops, tunnels and their typed
//! settings/observations. Everything here is host-agnostic: the engine never
//! touches files, OS services, signals, `process::exit` or stdout/stderr.

pub mod access;
pub mod conn_path;
pub mod endpoint;
pub mod observation;
pub mod pipe;
pub mod proto;
pub mod ready;
pub mod runtime;
pub mod serve;
pub mod settings;
pub mod shutdown;
pub mod testing;
pub mod udp;

pub use access::run_access;
pub use endpoint::{bind_access, bind_serve};
pub use runtime::RunCompletion;
pub use serve::run_serve;
pub use shutdown::RunContext;
