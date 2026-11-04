//! Serve role host entry points.
//!
//! Thin wrappers that load the config, claim the role, map it to engine
//! settings and call [`iroh_tunnel_engine::run_serve`]; the accept loops and
//! tunnels live in the engine. See `role_run` for the orchestration.

use std::future::Future;
use std::path::Path;

use anyhow::Result;

use crate::config::ServeConfig;
use crate::role_run::RoleStrategy;
use iroh_tunnel_engine::observation::ServeObservation;
use iroh_tunnel_engine::settings::ServeSettings;
use iroh_tunnel_engine::{self as engine, RoleHooks, RunCompletion, RunContext};

/// Run the serve role until interrupted (Ctrl-C).
///
/// Thin wrapper over [`crate::role_run::run_with_shutdown`] that wires the
/// real signal handler and the [`ServeStrategy`] implementer.
pub async fn run(config_path: &Path, ctx: crate::shutdown::RunContext) -> Result<()> {
    crate::role_run::run_with_shutdown::<ServeStrategy>(
        config_path,
        crate::role_run::Ownership::Enforced(crate::role_run::StatusRole::Serve),
        ctx,
        crate::role_run::file_status_hooks::<ServeStrategy>(crate::render::print_ready(), None),
        crate::shutdown::wait_for_signal(),
    )
    .await
}

/// Run an in-memory quick configuration without persisting its identity.
pub async fn run_quick(cfg: ServeConfig, ctx: crate::shutdown::RunContext) -> Result<()> {
    cfg.validate()
        .map_err(|e| crate::error::CliError::Config(format!("serve quick mode: {e}")))?;
    let claim = Some(crate::ownership::claim(crate::role_run::StatusRole::Serve)?);
    crate::role_run::run_config::<ServeStrategy, _, _>(
        cfg,
        claim,
        ctx,
        None,
        crate::role_run::file_status_hooks::<ServeStrategy>(crate::render::print_ready(), None),
        |ep, settings, ctx, hooks| {
            ServeStrategy::run(ep, settings, crate::shutdown::wait_for_signal(), ctx, hooks)
        },
    )
    .await
}

/// Run the serve role until the caller-provided `shutdown` future resolves.
///
/// Same as [`run`], but the shutdown signal is injected. Production wires
/// `shutdown::wait_for_signal()` here; tests inject a `oneshot::Receiver` or
/// similar so the role can be driven end-to-end without sending real signals.
pub async fn run_with_shutdown(
    config_path: &Path,
    shutdown: impl Future<Output = ()>,
) -> Result<()> {
    crate::role_run::run_with_shutdown::<ServeStrategy>(
        config_path,
        crate::role_run::Ownership::Unenforced,
        crate::shutdown::RunContext::new(),
        crate::role_run::file_status_hooks::<ServeStrategy>(crate::render::print_ready(), None),
        shutdown,
    )
    .await
}

/// Run the serve role like [`run_with_shutdown`], but write the status file
/// into the explicitly given `state_dir` instead of the env-resolved default.
///
/// Advanced/testing seam: integration tests point a serve instance at an
/// isolated tempdir without touching the process-global
/// `IROH_TUNNEL_STATE_DIR` variable (which cannot be mutated safely while
/// other test threads exist). Production callers use [`run_with_shutdown`];
/// operators relocate the file via the env variable instead.
pub async fn run_with_shutdown_with_state_dir(
    state_dir: &Path,
    config_path: &Path,
    shutdown: impl Future<Output = ()>,
) -> Result<()> {
    crate::role_run::run_skeleton::<ServeStrategy, _, _>(
        config_path,
        crate::role_run::Ownership::Unenforced,
        crate::shutdown::RunContext::new(),
        Some(state_dir),
        crate::role_run::file_status_hooks::<ServeStrategy>(
            crate::render::print_ready(),
            Some(state_dir),
        ),
        |ep, settings, ctx, hooks| ServeStrategy::run(ep, settings, shutdown, ctx, hooks),
    )
    .await
}

/// Serve-role implementation of [`RoleStrategy`]: maps the config to engine
/// settings and delegates binding and the role loop to the engine.
pub(crate) struct ServeStrategy;

impl RoleStrategy for ServeStrategy {
    const STATUS_WRITER: crate::role_run::StatusWriter = crate::role_run::StatusWriter::serve();

    type Config = ServeConfig;
    type Settings = ServeSettings;
    type Observation = ServeObservation;

    fn settings(cfg: &Self::Config) -> Result<Self::Settings> {
        crate::config::serve_settings(cfg)
    }

    fn node(settings: &Self::Settings) -> &engine::settings::NodeSettings {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Seam: in-process serve (no client ever connects). The initial write
    /// happens in the run loop before `accept_loop` is spawned, so the file
    /// must already be present with no connections.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn initial_status_written_before_first_admission() {
        let (_relay_map, relay_url, _server) =
            iroh::test_utils::run_relay_server().await.expect("relay");
        let cfg_dir = tempfile::tempdir().unwrap();
        let state_dir = tempfile::tempdir().unwrap();
        let cfg = cfg_dir.path().join("serve.toml");
        std::fs::write(
            &cfg,
            format!(
                "[node]\nrelay_urls = [\"{relay_url}\"]\n\n[[services]]\nname = \"echo\"\nprotocol = \"tcp\"\nport = 9\n"
            ),
        )
        .unwrap();

        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let state = state_dir.path().to_path_buf();
        let role = tokio::spawn(async move {
            run_with_shutdown_with_state_dir(&state, &cfg, async move {
                let _ = stop_rx.await;
            })
            .await
        });

        let status_path = state_dir.path().join("serve-status.json");
        let mut parsed = None;
        for _ in 0..200 {
            if let Ok(text) = std::fs::read_to_string(&status_path) {
                parsed = serde_json::from_str::<serde_json::Value>(&text).ok();
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let status = parsed.expect("status file written without any client");
        assert_eq!(
            status["connections"],
            serde_json::json!([]),
            "no admission has happened yet"
        );
        assert_eq!(status["services"][0]["name"], "echo");

        let _ = stop_tx.send(());
        role.await.unwrap().unwrap();
    }
}
