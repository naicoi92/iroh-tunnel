//! Serve role: bind the endpoint and run the accept loops.

mod bind_serve;
mod run_serve;

pub use bind_serve::bind_serve;
pub use run_serve::run_serve;
