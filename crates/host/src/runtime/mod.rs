//! Host runtime: orchestration shared by the serve and access role `run` paths.
//!
//! claim → load config + persist key → publish identity → map to engine
//! settings → bind → engine role loop → remove own status file → release the
//! claim. The role loops live in [`iroh_tunnel_engine`]; this module owns
//! everything file- and OS-bound around them. Terminal and signal concerns
//! are the caller's: it supplies the readiness hook and the shutdown future.

mod policy;
mod remove_own_status_file_for_exit;
mod run_foreground;
mod run_quick_access;
mod run_quick_serve;
mod skeleton;
mod strategy;

#[cfg(feature = "test-utils")]
pub(crate) use policy::Ownership;
pub use remove_own_status_file_for_exit::remove_own_status_file_for_exit;
pub use run_foreground::run_foreground;
pub use run_quick_access::run_quick_access;
pub use run_quick_serve::run_quick_serve;
#[cfg(feature = "test-utils")]
pub(crate) use skeleton::run_role;
