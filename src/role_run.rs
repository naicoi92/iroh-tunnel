//! Host orchestration shared by the serve and access role `run` paths.
//!
//! claim → load config + persist key → publish identity → map to engine
//! settings → bind → engine role loop → remove own status file → release the
//! claim. The role loops live in [`crate::engine`]; this module owns
//! everything file- and OS-bound around them (ownership claim, config load,
//! status-file mapping and cleanup). The per-role differences sit behind the
//! [`RoleStrategy`] trait.
//!
//! Everything here is `pub(crate)` — these are internal seams between two
//! modules of this crate, not a public extension point.

use std::future::Future;
use std::path::Path;

use anyhow::{Context, Result};

use crate::config::RoleDoc;
use crate::engine::observation::{RoleHooks, StatusSink};
use crate::engine::settings::NodeSettings;
use crate::engine::{RunCompletion, RunContext};
use crate::ownership::RoleClaim;
/// Role identity and writer: the host side picks the writer by role; the
/// engine never sees them.
pub(crate) use crate::status::{StatusRole, StatusWriter};

/// Whether a run enforces role ownership (RO-01..RO-03).
///
/// Production entry points enforce it; the library seams that tests drive
/// in-process (`run_with_shutdown*`) do not, so several in-process roles of
/// one kind can share a test binary. Real-process tests cover enforcement.
#[derive(Clone, Copy)]
pub(crate) enum Ownership {
    Enforced(StatusRole),
    Unenforced,
}

impl Ownership {
    /// Take the role's locks, refusing while the role already runs.
    pub(crate) fn claim(self) -> Result<Option<RoleClaim>> {
        match self {
            Self::Enforced(role) => crate::ownership::claim(role).map(Some),
            Self::Unenforced => Ok(None),
        }
    }
}

/// Finish a run: remove `writer`'s status file (only while it still belongs
/// to this process) when the writer quiesced, then yield the run result.
/// `state_dir` is the directory the run wrote to (`None` = env default).
pub(crate) async fn finish_run(
    completion: RunCompletion,
    writer: StatusWriter,
    state_dir: Option<&Path>,
) -> Result<()> {
    if completion.status_quiesced {
        crate::status::remove_own_status_file(writer, state_dir).await;
    }
    completion.result
}

/// Hooks whose status sink writes the role's status file via
/// [`crate::status::StatusWriter::save_with_state_dir`] (`None` = env
/// default dir). The save runs on the blocking pool (the atomic save fsyncs;
/// RS-07) and is awaited to completion by the flush loop, never detached.
pub(crate) fn file_status_hooks<S: RoleStrategy>(
    on_ready: crate::engine::ready::OnReady,
    state_dir: Option<&Path>,
) -> RoleHooks<S::Observation>
where
    for<'a> crate::status::StatusPayload: From<&'a S::Observation>,
{
    let state_dir = state_dir.map(Path::to_path_buf);
    let writer = S::STATUS_WRITER;
    let status = StatusSink::new(move |observation: S::Observation| {
        let payload = crate::status::StatusPayload::from(&observation);
        let state_dir = state_dir.clone();
        Box::pin(async move {
            let path = tokio::task::spawn_blocking(move || {
                writer.save_with_state_dir(state_dir.as_deref(), &payload)
            })
            .await
            .context("status save task failed")??;
            tracing::debug!(path = %path.display(), "wrote status file");
            Ok(())
        }) as crate::engine::observation::BoxFuture<'static, Result<()>>
    });
    RoleHooks {
        on_ready,
        status: Some(status),
    }
}

// ---------------------------------------------------------------------------
// RoleStrategy trait + shared run skeleton
// ---------------------------------------------------------------------------

/// The role's half of the `run` skeleton.
///
/// Each implementer carries the genuinely-different pieces (build the
/// endpoint with role-specific args, print the per-service status lines,
/// run the role-specific loop and finish owned shutdown). The shared skeleton
/// — load config, resolve key, print NodeId, dispatch role — lives in
/// [`run_with_shutdown`] so it has exactly one definition site.
///
/// The associated type [`RoleStrategy::Config`] is the role's TOML document
/// (`ServeConfig` / `AccessConfig`); it must load+validate+persist its own
/// key via the [`crate::config::RoleDoc`] trait.
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

/// Drive a role end-to-end: load → resolve key → endpoint → print → loop →
/// owned shutdown, with the role's run-loop supplied as a parameter.
///
/// This is the single definition site of the orchestration; the plain
/// [`run_with_shutdown`] wraps it with the engine's stock role loop, and
/// the serve role's state-dir injection seam
/// (`serve::run_with_shutdown_with_state_dir`) wraps it with a closure that
/// carries the injected dir — neither duplicates the skeleton.
///
/// Production callers wrap this with [`shutdown::wait_for_signal`]; tests
/// inject a `oneshot::Receiver` so the role can be driven without sending
/// real signals.
///
/// The status-file write (serve-only) and any other post-build hooks live
/// inside the run-loop closure — that's where the role has access to both
/// the loaded config and the live endpoint, and where the timing fits
/// between "endpoint ready" and "shutdown received".
pub(crate) async fn run_skeleton<S, F, Fut>(
    config_path: &Path,
    ownership: Ownership,
    ctx: RunContext,
    state_dir: Option<&Path>,
    hooks: RoleHooks<S::Observation>,
    run_loop: F,
) -> Result<()>
where
    S: RoleStrategy,
    F: FnOnce(iroh::Endpoint, S::Settings, RunContext, RoleHooks<S::Observation>) -> Fut,
    Fut: Future<Output = RunCompletion>,
{
    // Claim the role before touching the config: a refused start must not
    // generate or persist a key.
    let claim = ownership.claim()?;
    // Load and validate the config, then resolve+persist the secret key. Both
    // steps are shared across roles via the RoleDoc trait.
    // The save takes a file lock and fsyncs, so it runs off the async workers
    // (RS-07). It is awaited to completion, never detached.
    let path = config_path.to_path_buf();
    let cfg = tokio::task::spawn_blocking(move || -> Result<S::Config> {
        let mut cfg = crate::service::load_config::<S::Config>(&path)?;
        cfg.resolve_and_save_key(&path)?;
        Ok(cfg)
    })
    .await
    .context("config load task failed")??;

    run_config::<S, _, _>(cfg, claim, ctx, state_dir, hooks, run_loop).await
}

/// Map the config, bind the endpoint and run the role while holding `claim`.
///
/// The public identity is checked against the other role before the endpoint
/// binds, so a rejected duplicate never reaches the relay (RO-02). An
/// ephemeral key only exists after the endpoint is built and cannot clash.
pub(crate) async fn run_config<S, F, Fut>(
    cfg: S::Config,
    claim: Option<RoleClaim>,
    ctx: RunContext,
    state_dir: Option<&Path>,
    hooks: RoleHooks<S::Observation>,
    run_loop: F,
) -> Result<()>
where
    S: RoleStrategy,
    F: FnOnce(iroh::Endpoint, S::Settings, RunContext, RoleHooks<S::Observation>) -> Fut,
    Fut: Future<Output = RunCompletion>,
{
    // Resolving the key here keeps an invalid `secret_key` a pre-bind error.
    let settings = S::settings(&cfg)?;
    // A configured key is known before binding, so a duplicate identity is
    // rejected without touching the relay; an ephemeral key only exists
    // after the endpoint is built and cannot clash.
    let configured_id = S::node(&settings)
        .secret_key
        .as_ref()
        .map(|key| key.public().to_string());
    if let (Some(claim), Some(id)) = (&claim, &configured_id) {
        claim.publish_identity(id)?;
    }

    // Build the role-specific endpoint (serve registers ALPNs, access is
    // outbound-only).
    let ep = S::bind(&settings).await?;
    if let (Some(claim), None) = (&claim, &configured_id) {
        claim.publish_identity(&crate::engine::endpoint::node_id_string(&ep))?;
    }

    // The engine reports readiness through `hooks.on_ready`, then owns
    // admission, drain, task joins and endpoint close; this owner removes
    // the status file once the writer has quiesced.
    let completion = run_loop(ep, settings, ctx, hooks).await;
    let result = finish_run(completion, S::STATUS_WRITER, state_dir).await;
    // Held to here so the role stays owned until it has fully stopped.
    drop(claim);
    result
}

/// Drive a role end-to-end with the engine's stock role loop.
pub(crate) async fn run_with_shutdown<S: RoleStrategy>(
    config_path: &Path,
    ownership: Ownership,
    ctx: RunContext,
    hooks: RoleHooks<S::Observation>,
    shutdown: impl Future<Output = ()>,
) -> Result<()> {
    run_skeleton::<S, _, _>(
        config_path,
        ownership,
        ctx,
        None,
        hooks,
        |ep, settings, ctx, hooks| S::run(ep, settings, shutdown, ctx, hooks),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A failing `on_ready` aborts the start: the run returns that error, the
    /// claim is released and no status file was written. Role locks are keyed
    /// by `HOME`, so the body re-runs in a child process with an isolated
    /// `HOME` instead of mutating this multi-threaded process's environment.
    #[test]
    fn ready_hook_failure_closes_endpoint_and_releases_claim() {
        const CHILD_MARKER: &str = "IROH_TUNNEL_READY_HOOK_CHILD";
        const NAME: &str = "role_run::tests::ready_hook_failure_closes_endpoint_and_releases_claim";
        if std::env::var_os(CHILD_MARKER).is_none() {
            let home = tempfile::tempdir().unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", NAME, "--nocapture"])
                .env(CHILD_MARKER, "1")
                .env("HOME", home.path())
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(output.status.success(), "child failed:\n{stdout}\n{stderr}");
            assert!(
                stdout.contains(&format!("IROH_CHILD_BODY_RAN {NAME}")),
                "child did not run the test body (0 tests matched?):\n{stdout}\n{stderr}"
            );
            return;
        }

        let state_dir = tempfile::tempdir().unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let claim = crate::ownership::claim(StatusRole::Serve).unwrap();
            let on_ready: crate::engine::ready::OnReady =
                Box::new(|_| Err(anyhow::anyhow!("ready hook refused")));
            let hooks =
                file_status_hooks::<crate::serve::ServeStrategy>(on_ready, Some(state_dir.path()));
            let error = run_config::<crate::serve::ServeStrategy, _, _>(
                crate::config::ServeConfig::default(),
                Some(claim),
                RunContext::new(),
                Some(state_dir.path()),
                hooks,
                |ep, settings, ctx, hooks| {
                    crate::serve::ServeStrategy::run(
                        ep,
                        settings,
                        std::future::pending::<()>(),
                        ctx,
                        hooks,
                    )
                },
            )
            .await
            .unwrap_err();
            assert_eq!(error.to_string(), "ready hook refused");

            let reclaimed = crate::ownership::claim(StatusRole::Serve);
            assert!(
                reclaimed.is_ok(),
                "claim must be released after the failure"
            );
            let leftovers: Vec<_> = std::fs::read_dir(state_dir.path()).unwrap().collect();
            assert!(
                leftovers.is_empty(),
                "no status file may exist: {leftovers:?}"
            );
        });
        println!("IROH_CHILD_BODY_RAN {NAME}");
    }

    fn write_own_serve_status(dir: &Path) -> std::path::PathBuf {
        let status = crate::status::StatusFile {
            node_id: "node".into(),
            home_relay: None,
            pid: std::process::id(),
            started_at: 0,
            services: Vec::new(),
            connections: Vec::new(),
        };
        crate::status::StatusWriter::serve()
            .save_to(dir, &crate::status::StatusPayload::Serve(status))
            .unwrap()
    }

    #[tokio::test]
    async fn status_file_kept_when_flush_join_times_out() {
        let tmp = tempfile::tempdir().unwrap();
        let file = write_own_serve_status(tmp.path());
        let writer = crate::status::StatusWriter::serve();

        let unquiesced = RunCompletion {
            result: Ok(()),
            status_quiesced: false,
        };
        finish_run(unquiesced, writer, Some(tmp.path()))
            .await
            .unwrap();
        assert!(file.exists(), "unquiesced writer: file must be kept");

        let quiesced = RunCompletion {
            result: Ok(()),
            status_quiesced: true,
        };
        finish_run(quiesced, writer, Some(tmp.path()))
            .await
            .unwrap();
        assert!(!file.exists(), "quiesced writer: own file is removed");
    }
}
