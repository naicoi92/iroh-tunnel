//! Access role host entry points.
//!
//! Thin wrappers that load the config, claim the role, map it to engine
//! settings and call [`crate::engine::run_access`]; the accept loops and
//! tunnels live in the engine. See `role_run` for the orchestration.

use std::future::Future;
use std::path::Path;

use anyhow::Result;

use crate::config::AccessConfig;
use crate::engine::observation::{AccessObservation, RoleHooks};
use crate::engine::settings::AccessSettings;
use crate::engine::{self, RunCompletion, RunContext};
use crate::role_run::RoleStrategy;

/// Re-export of the engine's path-poller cadence for integration tests.
#[cfg(feature = "test-utils")]
pub use crate::engine::access::PATH_POLL_INTERVAL;

/// Run the access role until interrupted (Ctrl-C).
///
/// Thin wrapper over [`crate::role_run::run_with_shutdown`] that wires the
/// real signal handler and the [`AccessStrategy`] implementer.
pub async fn run(config_path: &Path, ctx: crate::shutdown::RunContext) -> Result<()> {
    crate::role_run::run_with_shutdown::<AccessStrategy>(
        config_path,
        crate::role_run::Ownership::Enforced(crate::role_run::StatusRole::Access),
        ctx,
        crate::role_run::file_status_hooks::<AccessStrategy>(crate::render::print_ready(), None),
        crate::shutdown::wait_for_signal(),
    )
    .await
}

/// Run an in-memory quick configuration without persisting its identity.
pub async fn run_quick(cfg: AccessConfig, ctx: crate::shutdown::RunContext) -> Result<()> {
    cfg.validate()
        .map_err(|e| crate::error::CliError::Config(format!("access quick mode: {e}")))?;
    let claim = Some(crate::ownership::claim(
        crate::role_run::StatusRole::Access,
    )?);
    crate::role_run::run_config::<AccessStrategy, _, _>(
        cfg,
        claim,
        ctx,
        None,
        crate::role_run::file_status_hooks::<AccessStrategy>(crate::render::print_ready(), None),
        |ep, settings, ctx, hooks| {
            AccessStrategy::run(ep, settings, crate::shutdown::wait_for_signal(), ctx, hooks)
        },
    )
    .await
}

/// Run the access role until the caller-provided `shutdown` future resolves.
///
/// Same as [`run`], but the shutdown signal is injected. Production wires
/// `shutdown::wait_for_signal()` here; tests inject a `oneshot::Receiver` or
/// similar so the role can be driven end-to-end without sending real signals.
pub async fn run_with_shutdown(
    config_path: &Path,
    shutdown: impl Future<Output = ()>,
) -> Result<()> {
    crate::role_run::run_with_shutdown::<AccessStrategy>(
        config_path,
        crate::role_run::Ownership::Unenforced,
        crate::shutdown::RunContext::new(),
        crate::role_run::file_status_hooks::<AccessStrategy>(crate::render::print_ready(), None),
        shutdown,
    )
    .await
}

/// Run the access role like [`run_with_shutdown`], but write its status
/// file into the explicitly given `state_dir` instead of the env-resolved
/// default.
///
/// Advanced/testing seam mirroring
/// [`crate::serve::run_with_shutdown_with_state_dir`]: integration tests
/// point an access instance at an isolated tempdir without touching the
/// process-global `IROH_TUNNEL_STATE_DIR` variable (which cannot be
/// mutated safely while other test threads exist). Production callers use
/// [`run_with_shutdown`]; operators relocate the file via the env variable
/// instead.
pub async fn run_with_shutdown_with_state_dir(
    state_dir: &Path,
    config_path: &Path,
    shutdown: impl Future<Output = ()>,
) -> Result<()> {
    crate::role_run::run_skeleton::<AccessStrategy, _, _>(
        config_path,
        crate::role_run::Ownership::Unenforced,
        crate::shutdown::RunContext::new(),
        Some(state_dir),
        crate::role_run::file_status_hooks::<AccessStrategy>(
            crate::render::print_ready(),
            Some(state_dir),
        ),
        |ep, settings, ctx, hooks| AccessStrategy::run(ep, settings, shutdown, ctx, hooks),
    )
    .await
}

/// Access-role implementation of [`RoleStrategy`]: maps the config to engine
/// settings and delegates binding and the role loop to the engine.
pub(crate) struct AccessStrategy;

impl RoleStrategy for AccessStrategy {
    const STATUS_WRITER: crate::role_run::StatusWriter = crate::role_run::StatusWriter::access();

    type Config = AccessConfig;
    type Settings = AccessSettings;
    type Observation = AccessObservation;

    fn settings(cfg: &Self::Config) -> Result<Self::Settings> {
        crate::config::access_settings(cfg)
    }

    fn node(settings: &Self::Settings) -> &engine::settings::NodeSettings {
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
