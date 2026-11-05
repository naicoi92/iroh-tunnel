//! Run a role from its config file until shutdown.

use std::future::Future;
use std::path::Path;

use anyhow::Result;

use super::policy::Ownership;
use super::skeleton::run_role;
use crate::role::Role;
use iroh_tunnel_engine::{OnReady, RunContext};

/// Run `role` from the config file at `config` until `shutdown` resolves.
///
/// Order: claim the role → load the config and persist its key (blocking
/// pool) → publish the configured identity → bind → publish an ephemeral
/// identity → engine role loop → remove the role's own status file only if
/// the status writer quiesced → release the claim. `on_ready` receives the
/// startup report; its error aborts the start. Dropping the future cancels
/// the run (owned tasks and the endpoint are dropped with it).
///
/// # Errors
///
/// [`crate::error::CliError`]-classified config, ownership and iroh
/// failures, or whatever `on_ready` returned.
pub async fn run_foreground(
    role: Role,
    config: &Path,
    ctx: RunContext,
    on_ready: OnReady,
    shutdown: impl Future<Output = ()>,
) -> Result<()> {
    run_role(
        role,
        config,
        Ownership::Enforced(role),
        ctx,
        None,
        on_ready,
        shutdown,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_missing_config_fails_before_any_bind() {
        // HOME-keyed role locks make a real claim unsuitable here; the
        // ownership-enforcing path is covered by the role_ownership tests.
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().to_path_buf();
        let result = run_role(
            Role::Serve,
            &dir.path().join("missing.toml"),
            Ownership::Enforced(Role::Serve),
            RunContext::new(),
            Some(&state),
            Box::new(|_| Ok(())),
            std::future::ready(()),
        )
        .await;
        assert!(result.is_err());
    }
}
