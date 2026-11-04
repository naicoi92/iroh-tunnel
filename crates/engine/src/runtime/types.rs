//! Values exchanged between a host and a running role: readiness report,
//! status sink, hooks and the completion report.

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;

use crate::settings::Protocol;

/// Hook a role calls once it is ready. An `Err` aborts the start: the
/// endpoint is closed and the error is returned from the run.
pub type OnReady = Box<dyn FnOnce(&ReadyReport) -> anyhow::Result<()> + Send>;

/// What a role knows about itself once its endpoint is up.
#[derive(Debug, Clone, PartialEq)]
pub struct ReadyReport {
    /// Full endpoint id of the role.
    pub node_id: String,
    /// Home relay URL, if the endpoint has one.
    pub home_relay: Option<String>,
    /// Per-service readiness, by role.
    pub services: ReadyServices,
}

/// The per-service half of a [`ReadyReport`], by role.
#[derive(Debug, Clone, PartialEq)]
pub enum ReadyServices {
    /// Services published by the Serve role.
    Serve(Vec<ServeReady>),
    /// Services exposed by the Access role.
    Access(Vec<AccessReady>),
}

/// A service the serve role publishes.
#[derive(Debug, Clone, PartialEq)]
pub struct ServeReady {
    /// Service name.
    pub name: String,
    /// Tunnel protocol of the service.
    pub protocol: Protocol,
    /// Local `host:port` the service forwards to.
    pub local_addr: String,
}

/// A remote service the access role exposes locally.
#[derive(Debug, Clone, PartialEq)]
pub struct AccessReady {
    /// Service name.
    pub name: String,
    /// Local `host:port` the service listens on.
    pub listen_addr: String,
    /// The serve peer's node id as shown to the operator (the raw configured
    /// text when it does not parse).
    pub peer_display: String,
    /// Tunnel protocol of the service.
    pub protocol: Protocol,
    /// Number of per-service relay URLs overriding the node relays (count).
    pub relay_override_count: usize,
}

/// Boxed, sendable future (local alias; no `futures` dependency).
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Publishes observations of type `O`; the host decides where they go.
pub struct StatusSink<O> {
    save: Box<dyn FnMut(O) -> BoxFuture<'static, anyhow::Result<()>> + Send>,
}

impl<O> StatusSink<O> {
    /// Wrap a save callback that persists one observation.
    pub fn new(
        save: impl FnMut(O) -> BoxFuture<'static, anyhow::Result<()>> + Send + 'static,
    ) -> Self {
        Self {
            save: Box::new(save),
        }
    }

    /// Persist one observation; resolves once the save has finished.
    pub fn save(&mut self, observation: O) -> BoxFuture<'static, anyhow::Result<()>> {
        (self.save)(observation)
    }
}

/// Host-provided hooks of one role run.
pub struct RoleHooks<O> {
    /// Called once the role is ready; an `Err` aborts the start.
    pub on_ready: OnReady,
    /// `None` runs the role without publishing status.
    pub status: Option<StatusSink<O>>,
}

/// What a role's run loop reports to the run owner.
///
/// `status_quiesced` is `true` only when the status flush task joined within
/// `STATUS_FLUSH_JOIN_BOUND` and returned `Ok`, or when no flush task ever
/// ran (no sink, or the body failed before spawning one). Only then may the
/// run owner remove the status file: an unquiesced writer may still have a
/// save in flight that would recreate it. Every role-body path states it
/// explicitly; none is inferred from the result.
pub struct RunCompletion {
    /// Outcome of the role run.
    pub result: Result<()>,
    /// `true` when no status save can still be in flight, so the status file may be removed.
    pub status_quiesced: bool,
}
