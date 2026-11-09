//! Per-role differences behind one trait: config type, engine settings
//! mapping, bind and role loop.

use std::future::Future;

use anyhow::Result;

use crate::config::{AccessConfig, ServeConfig};
use crate::status::writer::StatusWriter;
use iroh_tunnel_engine::observation::{AccessObservation, ServeObservation};
use iroh_tunnel_engine::settings::{AccessSettings, NodeSettings, ServeSettings};
use iroh_tunnel_engine::{self as engine, RoleHooks, RunCompletion, RunContext};

/// Serve-role implementation of [`RoleStrategy`]: maps the config to engine
/// settings and delegates binding and the role loop to the engine.
pub(crate) struct ServeStrategy;

impl RoleStrategy for ServeStrategy {
    const STATUS_WRITER: StatusWriter = StatusWriter::serve();

    type Config = ServeConfig;
    type Settings = ServeSettings;
    type Observation = ServeObservation;

    fn settings(cfg: &Self::Config) -> Result<Self::Settings> {
        crate::config::serve_settings(cfg)
    }

    fn node(settings: &Self::Settings) -> &NodeSettings {
        &settings.node
    }

    async fn bind(settings: &Self::Settings) -> Result<iroh::Endpoint> {
        engine::bind_serve(settings).await
    }

    fn run(
        ep: iroh::Endpoint,
        settings: Self::Settings,
        shutdown: impl Future<Output = ()>,
        ctx: RunContext,
        hooks: RoleHooks<Self::Observation>,
    ) -> impl Future<Output = RunCompletion> {
        engine::run_serve(ep, settings, ctx, hooks, shutdown)
    }
}

/// Access-role implementation of [`RoleStrategy`].
pub(crate) struct AccessStrategy;

impl RoleStrategy for AccessStrategy {
    const STATUS_WRITER: StatusWriter = StatusWriter::access();

    type Config = AccessConfig;
    type Settings = AccessSettings;
    type Observation = AccessObservation;

    fn settings(cfg: &Self::Config) -> Result<Self::Settings> {
        crate::config::access_settings(cfg)
    }

    fn node(settings: &Self::Settings) -> &NodeSettings {
        &settings.node
    }

    async fn bind(settings: &Self::Settings) -> Result<iroh::Endpoint> {
        engine::bind_access(settings).await
    }

    fn run(
        ep: iroh::Endpoint,
        settings: Self::Settings,
        shutdown: impl Future<Output = ()>,
        ctx: RunContext,
        hooks: RoleHooks<Self::Observation>,
    ) -> impl Future<Output = RunCompletion> {
        engine::run_access(ep, settings, ctx, hooks, shutdown)
    }
}

/// The role's half of the `run` skeleton.
///
/// Each implementer carries the genuinely-different pieces (build the
/// endpoint with role-specific args, print the per-service status lines,
/// run the role-specific loop and finish owned shutdown). The shared skeleton
/// — load config, resolve key, print NodeId, dispatch role — lives in
/// `run_foreground` (via `run_skeleton`) so it has exactly one definition site.
///
/// The associated type [`RoleStrategy::Config`] is the role's TOML document
/// (`ServeConfig` / `AccessConfig`); it validates via
/// [`crate::config::RoleDoc`]; files are handled by the config store.
pub(crate) trait RoleStrategy {
    /// The TOML config document this role loads.
    type Config: crate::config::RoleDoc + Send + 'static;

    /// The engine settings this role runs with.
    type Settings: Send + 'static;

    /// What the role observes about itself for status publishing.
    type Observation: Clone + PartialEq + Send + 'static;

    /// The role's status file writer.
    const STATUS_WRITER: StatusWriter;

    /// Map the loaded config to engine settings (resolves the secret key).
    fn settings(cfg: &Self::Config) -> Result<Self::Settings>;

    /// The node-level settings inside `settings` (identity pre-check).
    fn node(settings: &Self::Settings) -> &NodeSettings;

    /// Bind the iroh endpoint for this role (serve registers ALPNs, access
    /// dials out only).
    fn bind(settings: &Self::Settings)
        -> impl std::future::Future<Output = Result<iroh::Endpoint>>;

    /// Run the engine role loop until `shutdown` resolves.
    ///
    /// The engine owns admission, drain, task joins and the endpoint close.
    /// The returned [`RunCompletion`] tells the run owner whether the status
    /// writer quiesced; the owner, not the role, removes the status file.
    fn run(
        ep: iroh::Endpoint,
        settings: Self::Settings,
        shutdown: impl Future<Output = ()>,
        ctx: RunContext,
        hooks: RoleHooks<Self::Observation>,
    ) -> impl std::future::Future<Output = RunCompletion>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::role::Role;

    #[test]
    fn strategies_pick_their_roles_status_writer() {
        assert_eq!(ServeStrategy::STATUS_WRITER.role(), Role::Serve);
        assert_eq!(AccessStrategy::STATUS_WRITER.role(), Role::Access);
    }
}
