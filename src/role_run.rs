//! Shared scaffolding for the serve and access role `run` paths.
//!
//! The two roles share the skeleton: load config → resolve key → build
//! endpoint → print NodeId → loop until shutdown → drain + close. The
//! genuinely different pieces (ALPN demux, listeners, retry and owned shutdown)
//! live behind the [`RoleStrategy`] trait. Roles finish draining and cleanup
//! before returning; the skeleton adds no second grace period.
//!
//! Everything here is `pub(crate)` — these are internal seams between two
//! modules of this crate, not a public extension point.

use std::future::Future;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use iroh::endpoint::Connection;
use tokio::task::JoinHandle;

use crate::config::{Protocol, RoleDoc};
use crate::ownership::RoleClaim;
use crate::status::StatusRole;

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

/// How long a role waits for its status flush task to stop at shutdown.
const STATUS_FLUSH_JOIN_BOUND: Duration = Duration::from_secs(5);

/// What a role's run loop reports to the run owner.
///
/// `status_quiesced` is `true` only when the status flush task joined within
/// [`STATUS_FLUSH_JOIN_BOUND`] and returned `Ok`, or when no flush task ever
/// ran. Only then may the run owner remove the status file: an unquiesced
/// writer may still have a save in flight that would recreate it.
pub(crate) struct RunCompletion {
    pub result: Result<()>,
    pub status_quiesced: bool,
}

impl RunCompletion {
    /// Completion of a loop body that yields `Ok(status_quiesced)`. A body
    /// that failed before starting a flush task has no writer to wait for.
    pub(crate) fn from_body(body: Result<bool>) -> Self {
        match body {
            Ok(status_quiesced) => Self {
                result: Ok(()),
                status_quiesced,
            },
            Err(error) => Self {
                result: Err(error),
                status_quiesced: true,
            },
        }
    }
}

/// Stop a status flush task cooperatively and join it within
/// [`STATUS_FLUSH_JOIN_BOUND`]; returns whether it quiesced.
///
/// Aborting would not cancel an in-flight `spawn_blocking` save
/// (write+fsync+rename runs to completion on the blocking pool and could
/// recreate the file AFTER a cleanup removal). The task exits only at an
/// await point and every save is awaited inside its loop, so a successful
/// join proves none of its saves is still running. On a pathological stall
/// the join is abandoned and the caller must keep the status file.
pub(crate) async fn stop_flush(
    stop_tx: tokio::sync::oneshot::Sender<()>,
    mut flush: JoinHandle<()>,
) -> bool {
    // Dropping the sender alone would also close the channel, but only at
    // scope end — far too late.
    let _ = stop_tx.send(());
    let flush_abort = flush.abort_handle();
    match tokio::time::timeout(STATUS_FLUSH_JOIN_BOUND, &mut flush).await {
        Ok(result) => {
            let clean = result.is_ok();
            crate::shutdown::observe(result);
            clean
        }
        Err(_) => {
            tracing::warn!("status flush task did not stop within 5s; keeping status file");
            flush_abort.abort();
            crate::shutdown::observe(flush.await);
            false
        }
    }
}

/// Finish a run: remove `writer`'s status file (only while it still belongs
/// to this process) when the writer quiesced, then yield the run result.
/// `state_dir` is the directory the run wrote to (`None` = env default).
pub(crate) async fn finish_run(
    completion: RunCompletion,
    writer: crate::status::StatusWriter,
    state_dir: Option<&Path>,
) -> Result<()> {
    if completion.status_quiesced {
        crate::status::remove_own_status_file(writer, state_dir).await;
    }
    completion.result
}

// ---------------------------------------------------------------------------
// Small display helpers (shared, no polymorphism)
// ---------------------------------------------------------------------------

/// Lowercase protocol name for display (matches the serde form in `config`).
///
/// Previously duplicated verbatim in `serve.rs` and `access.rs`.
pub(crate) fn protocol_str(p: Protocol) -> &'static str {
    match p {
        Protocol::Tcp => "tcp",
        Protocol::Udp => "udp",
    }
}

/// Future that logs a disconnect line when the peer's QUIC connection
/// closes.
///
/// The weak handle is registered while `conn` is still alive, so iroh
/// guarantees the close event is delivered even if `conn` drops before this
/// resolves. Previously duplicated (with cosmetic differences) in both roles:
/// serve logged `peer disconnected` with a `service` field, access logged
/// `disconnected from serve peer` with an `sname` field. Both call sites
/// now build a single `message` string and a `peer` field, preserving the
/// information without the structural drift.
pub(crate) fn disconnect_watcher(
    conn: &Connection,
    peer: String,
    message: String,
) -> impl Future<Output = ()> + Send + 'static {
    let weak = conn.weak_handle();
    async move {
        let _ = weak.closed().await;
        tracing::info!(%peer, "{message}");
    }
}

// ---------------------------------------------------------------------------
// connect_with_retry + backoff schedule (access role)
// ---------------------------------------------------------------------------

/// Initial backoff (ms) for the access dial retry loop.
const INITIAL_BACKOFF_MS: u64 = 1_000;
/// First per-attempt dial bound; later attempts get more room (see
/// [`attempt_timeout`]).
const DIAL_ATTEMPT_STEP: Duration = Duration::from_secs(5);
/// Ceiling of the per-attempt dial bound.
const DIAL_ATTEMPT_MAX: Duration = Duration::from_secs(20);
/// Maximum backoff (ms) — the schedule caps here and stays flat.
const MAX_BACKOFF_MS: u64 = 30_000;

/// Compute the next backoff in the exponential schedule:
/// `1s → 2s → 4s → 8s → 16s → 30s → 30s → …` (capped).
///
/// Factored out of [`connect_with_retry`] so the schedule can be unit-tested
/// without driving a real iroh endpoint. The schedule is the load-bearing
/// piece — it bounds worst-case reconnect latency and matches Page 04 v2 §1.3.
pub(crate) fn next_backoff_ms(prev_ms: u64) -> u64 {
    (prev_ms.saturating_mul(2)).min(MAX_BACKOFF_MS)
}

/// Per-attempt dial bound: `5s → 10s → 15s → 20s (cap)`.
///
/// A stalled first attempt still leaves room for backoff and a retry inside
/// the caller's total budget, while a slow-but-working relay or hole-punch
/// dial gets progressively longer to finish. The caller's deadline, not this,
/// is the real bound.
pub(crate) fn attempt_timeout(attempt: u32) -> Duration {
    DIAL_ATTEMPT_STEP
        .saturating_mul(attempt)
        .min(DIAL_ATTEMPT_MAX)
}

/// Dial a peer with exponential backoff until success or `deadline`.
///
/// Retries [`iroh::Endpoint::connect`] on failure with the schedule
/// [`INITIAL_BACKOFF_MS`] → [`next_backoff_ms`] → … → [`MAX_BACKOFF_MS`]:
/// `1s → 2s → 4s → 8s → 16s → 30s (cap)`. On the first success after one or
/// more failures, logs `reconnected after N attempts`. Per-service
/// independent: an unreachable peer never affects another service.
///
/// # Errors
///
/// Returns an error once `deadline` passes: attempts and backoff sleeps are
/// both truncated at it, so the loop can never run unbounded (RS-09). Callers
/// may additionally cancel the future (for shutdown); dropping it cancels the
/// in-flight dial.
pub(crate) async fn connect_with_retry(
    ep: &iroh::Endpoint,
    addr: &iroh::EndpointAddr,
    alpn: &[u8],
    deadline: tokio::time::Instant,
) -> Result<Connection> {
    let mut backoff_ms = INITIAL_BACKOFF_MS;
    let mut attempt = 1u32;
    loop {
        let now = tokio::time::Instant::now();
        anyhow::ensure!(now < deadline, "dial deadline expired");
        // Dropping a timed-out attempt cancels that dial future.
        let limit = (now + attempt_timeout(attempt)).min(deadline);
        let dial = tokio::time::timeout_at(limit, ep.connect(addr.clone(), alpn)).await;
        match dial {
            Ok(Ok(conn)) => {
                if attempt > 1 {
                    tracing::info!("reconnected after {attempt} attempts");
                }
                return Ok(conn);
            }
            Ok(Err(error)) => {
                tracing::warn!(
                    "connect attempt {attempt} failed: {error}, retrying in {backoff_ms}ms"
                );
            }
            Err(_) if limit == deadline => anyhow::bail!("dial deadline expired"),
            Err(_) => {
                tracing::warn!("connect attempt {attempt} timed out, retrying in {backoff_ms}ms");
            }
        }
        tokio::time::sleep_until(
            (tokio::time::Instant::now() + Duration::from_millis(backoff_ms)).min(deadline),
        )
        .await;
        backoff_ms = next_backoff_ms(backoff_ms);
        attempt += 1;
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

    /// The role's status file writer.
    const STATUS_WRITER: crate::status::StatusWriter;

    /// Build the iroh endpoint for this role (serve registers ALPNs, access
    /// dials out only).
    fn build_endpoint(
        cfg: &Self::Config,
    ) -> impl std::future::Future<Output = Result<iroh::Endpoint>>;

    /// The per-service half of the ready report (serve: published services;
    /// access: exposed services). Also warns when no services are configured.
    fn ready_services(cfg: &Self::Config) -> crate::ready::ReadyServices;

    /// Run the role's accept/listen loop until `shutdown` resolves.
    ///
    /// The loop owns the endpoint and all descendant tasks. On `shutdown`, it
    /// seals admission, drains established pipes within one bounded deadline,
    /// joins/aborts remaining work and closes the endpoint before returning.
    /// The returned [`RunCompletion`] tells the run owner whether the status
    /// writer quiesced; the owner, not the role, removes the status file.
    fn run_loop(
        ep: iroh::Endpoint,
        cfg: Self::Config,
        shutdown: impl Future<Output = ()>,
        ctx: crate::shutdown::RunContext,
    ) -> impl std::future::Future<Output = RunCompletion>;
}

/// Drive a role end-to-end: load → resolve key → endpoint → print → loop →
/// owned shutdown, with the role's run-loop supplied as a parameter.
///
/// This is the single definition site of the orchestration; the plain
/// [`run_with_shutdown`] wraps it with the strategy's stock `run_loop`, and
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
    ctx: crate::shutdown::RunContext,
    state_dir: Option<&Path>,
    on_ready: crate::ready::OnReady,
    run_loop: F,
) -> Result<()>
where
    S: RoleStrategy,
    F: FnOnce(iroh::Endpoint, S::Config, crate::shutdown::RunContext) -> Fut,
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

    run_config::<S, _, _>(cfg, claim, ctx, state_dir, on_ready, run_loop).await
}

/// Build the endpoint and run the role while holding `claim`.
///
/// The public identity is checked against the other role before the endpoint
/// binds, so a rejected duplicate never reaches the relay (RO-02). An
/// ephemeral key only exists after the endpoint is built and cannot clash.
pub(crate) async fn run_config<S, F, Fut>(
    mut cfg: S::Config,
    claim: Option<RoleClaim>,
    ctx: crate::shutdown::RunContext,
    state_dir: Option<&Path>,
    on_ready: crate::ready::OnReady,
    run_loop: F,
) -> Result<()>
where
    S: RoleStrategy,
    F: FnOnce(iroh::Endpoint, S::Config, crate::shutdown::RunContext) -> Fut,
    Fut: Future<Output = RunCompletion>,
{
    // A configured key is known before binding, so a duplicate identity is
    // rejected without touching the relay; an ephemeral key only exists
    // after the endpoint is built and cannot clash.
    let secret_key = cfg.node_mut().secret_key.clone();
    let configured_id = if secret_key.is_empty() {
        None
    } else {
        let (key, _) = crate::config::resolve_secret_key(&secret_key)?;
        Some(key.public().to_string())
    };
    if let (Some(claim), Some(id)) = (&claim, &configured_id) {
        claim.publish_identity(id)?;
    }

    // Build the role-specific endpoint (serve registers ALPNs, access is
    // outbound-only).
    let ep = S::build_endpoint(&cfg).await?;
    if let (Some(claim), None) = (&claim, &configured_id) {
        claim.publish_identity(&crate::endpoint::node_id_string(&ep))?;
    }

    // Per-service entries diverge (Serving vs Exposed), so they sit behind
    // the strategy, which also warns when no services are configured.
    let report = crate::ready::ReadyReport {
        node_id: crate::endpoint::node_id_string(&ep),
        home_relay: crate::endpoint::home_relay(&ep).map(|relay| relay.to_string()),
        services: S::ready_services(&cfg),
    };
    if let Err(error) = on_ready(&report) {
        // `claim` drops on return; run_loop never ran, so no status file.
        ep.close().await;
        return Err(error);
    }

    tracing::info!("endpoint ready");
    // The role owns admission, drain, task joins and endpoint close; this
    // owner removes the status file once the writer has quiesced.
    let completion = run_loop(ep, cfg, ctx).await;
    let result = finish_run(completion, S::STATUS_WRITER, state_dir).await;
    // Held to here so the role stays owned until it has fully stopped.
    drop(claim);
    result
}

/// Drive a role end-to-end with its stock [`RoleStrategy::run_loop`].
pub(crate) async fn run_with_shutdown<S: RoleStrategy>(
    config_path: &Path,
    ownership: Ownership,
    ctx: crate::shutdown::RunContext,
    on_ready: crate::ready::OnReady,
    shutdown: impl Future<Output = ()>,
) -> Result<()> {
    run_skeleton::<S, _, _>(
        config_path,
        ownership,
        ctx,
        None,
        on_ready,
        |ep, cfg, ctx| S::run_loop(ep, cfg, shutdown, ctx),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_str_matches_serde_form() {
        // Must match the lowercase serde rename on the Protocol enum so the
        // displayed form round-trips through config.
        assert_eq!(protocol_str(Protocol::Tcp), "tcp");
        assert_eq!(protocol_str(Protocol::Udp), "udp");
    }

    #[test]
    fn attempt_timeout_grows_then_caps() {
        let secs: Vec<u64> = (1..=6).map(|n| attempt_timeout(n).as_secs()).collect();
        assert_eq!(secs, [5, 10, 15, 20, 20, 20]);
    }

    #[test]
    fn backoff_schedule_doubles_then_caps_at_max() {
        // Page 04 v2 §1.3 contract: 1s → 2s → 4s → 8s → 16s → 30s → 30s → …
        let steps = [
            (1_000u64, 2_000u64),
            (2_000, 4_000),
            (4_000, 8_000),
            (8_000, 16_000),
            (16_000, 30_000), // doubles to 32_000 but capped at MAX_BACKOFF_MS
            (30_000, 30_000), // already at cap, stays at cap
            (30_000, 30_000),
        ];
        for (prev, expected_next) in steps {
            let got = next_backoff_ms(prev);
            assert_eq!(
                got, expected_next,
                "next_backoff_ms({prev}ms) should be {expected_next}ms, got {got}ms"
            );
        }
    }

    #[test]
    fn backoff_schedule_starts_at_1s_and_caps_at_30s() {
        // Regression guards for the two load-bearing constants.
        assert_eq!(INITIAL_BACKOFF_MS, 1_000, "initial backoff must be 1s");
        assert_eq!(MAX_BACKOFF_MS, 30_000, "max backoff must be 30s");
        // The cap is reachable from the initial value within 5 doublings.
        let mut ms = INITIAL_BACKOFF_MS;
        for _ in 0..5 {
            ms = next_backoff_ms(ms);
        }
        assert_eq!(ms, MAX_BACKOFF_MS, "should reach cap after 5 doublings");
    }

    #[test]
    fn backoff_does_not_overflow_on_huge_input() {
        // saturating_mul must keep this finite even for absurd inputs.
        let prev = u64::MAX;
        let next = next_backoff_ms(prev);
        assert_eq!(next, MAX_BACKOFF_MS, "huge input must cap, not overflow");
    }

    /// The retry loop is bounded by the caller's deadline by construction:
    /// an unreachable peer yields an error at the deadline, not a hang.
    #[tokio::test]
    async fn connect_with_retry_stops_at_the_deadline() {
        use iroh::endpoint::presets::Minimal;
        let ep = iroh::Endpoint::builder(Minimal)
            .relay_mode(iroh::endpoint::RelayMode::Disabled)
            .bind()
            .await
            .unwrap();
        // Nobody listens on this id: every attempt fails or stalls.
        let addr = iroh::EndpointAddr::new(iroh::SecretKey::generate().public())
            .with_ip_addr((std::net::Ipv4Addr::LOCALHOST, 9).into());
        let started = std::time::Instant::now();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(1500);
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            connect_with_retry(&ep, &addr, b"t", deadline),
        )
        .await
        .expect("connect_with_retry must return by itself at the deadline");
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(4));
    }

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
            assert!(
                output.status.success(),
                "child failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        let state_dir = tempfile::tempdir().unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let claim = crate::ownership::claim(StatusRole::Serve).unwrap();
            let on_ready: crate::ready::OnReady =
                Box::new(|_| Err(anyhow::anyhow!("ready hook refused")));
            let error = run_config::<crate::serve::ServeStrategy, _, _>(
                crate::config::ServeConfig::default(),
                Some(claim),
                crate::shutdown::RunContext::new(),
                Some(state_dir.path()),
                on_ready,
                |ep, cfg, ctx| {
                    crate::serve::ServeStrategy::run_loop(
                        ep,
                        cfg,
                        std::future::pending::<()>(),
                        ctx,
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
