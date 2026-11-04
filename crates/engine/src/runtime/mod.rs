//! Run-owner values (context, hooks, readiness, completion) and the
//! crate-private mechanics shared by the role loops.

mod run_context;
pub(crate) mod support;
mod types;

pub use run_context::RunContext;
pub use types::{
    AccessReady, BoxFuture, OnReady, ReadyReport, ReadyServices, RoleHooks, RunCompletion,
    ServeReady, StatusSink,
};
