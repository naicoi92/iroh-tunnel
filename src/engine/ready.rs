//! Typed description of a role that has finished starting up.
//!
//! A role reports readiness once, after its endpoint is bound and its
//! identity is published, and before it admits any forwarding. Callers
//! decide how to present it (the CLI renders text; see the host renderer).

use crate::engine::settings::Protocol;

/// Hook a role calls once it is ready. An `Err` aborts the start: the
/// endpoint is closed and the error is returned from the run.
pub type OnReady = Box<dyn FnOnce(&ReadyReport) -> anyhow::Result<()> + Send>;

/// What a role knows about itself once its endpoint is up.
#[derive(Debug, Clone, PartialEq)]
pub struct ReadyReport {
    pub node_id: String,
    pub home_relay: Option<String>,
    pub services: ReadyServices,
}

/// The per-service half of a [`ReadyReport`], by role.
#[derive(Debug, Clone, PartialEq)]
pub enum ReadyServices {
    Serve(Vec<ServeReady>),
    Access(Vec<AccessReady>),
}

/// A service the serve role publishes.
#[derive(Debug, Clone, PartialEq)]
pub struct ServeReady {
    pub name: String,
    pub protocol: Protocol,
    pub local_addr: String,
}

/// A remote service the access role exposes locally.
#[derive(Debug, Clone, PartialEq)]
pub struct AccessReady {
    pub name: String,
    pub listen_addr: String,
    /// The serve peer's node id as shown to the operator (the raw configured
    /// text when it does not parse).
    pub peer_display: String,
    pub protocol: Protocol,
    pub relay_override_count: usize,
}
