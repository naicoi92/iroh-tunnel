//! Access role: bind the endpoint and run the local listeners.

mod bind_access;
mod run_access;

pub use bind_access::bind_access;
pub use run_access::run_access;
#[cfg(feature = "test-utils")]
pub use run_access::PATH_POLL_INTERVAL;
