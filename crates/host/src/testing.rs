//! Test-only seams (`test-utils`): drive a role in-process without taking
//! role ownership, so several in-process roles of one kind can share a test
//! binary. Real-process tests cover ownership enforcement.

use std::future::Future;
use std::path::Path;

use anyhow::Result;

use crate::runtime::{run_role, Ownership};
use crate::status::Role;
use crate::RunContext;

/// Run `role` from `config` until `shutdown` resolves, without claiming the
/// role. With `state_dir` the status file is written there instead of the
/// env-resolved default (no process-global env mutation needed). No readiness
/// line is reported.
///
/// # Errors
///
/// The config, bind and run failures of the role, as for a foreground run.
pub async fn run_unowned(
    role: Role,
    config: &Path,
    state_dir: Option<&Path>,
    shutdown: impl Future<Output = ()>,
) -> Result<()> {
    run_role(
        role,
        config,
        Ownership::Unenforced,
        RunContext::new(),
        state_dir,
        Box::new(|_| Ok(())),
        shutdown,
    )
    .await
}
