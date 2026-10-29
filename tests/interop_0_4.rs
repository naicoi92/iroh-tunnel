//! TCP-COMPAT-01 / #85: opt-in actual release/current executable evidence.
//! See tests/evidence/issue-85/README.md for artifact preparation and limits.
//! This test starts no Iroh endpoint: only digest-pinned production executables
//! in Python contact relay. Never pass Cargo's feature-unified executable.

use anyhow::{Context, Result};
use std::process::Command;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires authentic 0.4 and separately built production executables; see evidence README"]
async fn released_0_4_and_current_exchange_tcp_bytes_in_both_directions() -> Result<()> {
    for variable in [
        "IROH_INTEROP_OLD_BIN",
        "IROH_INTEROP_CURRENT_BIN",
        "IROH_INTEROP_OLD_SHA256",
        "IROH_INTEROP_CURRENT_SHA256",
        "IROH_INTEROP_RELAY",
        "IROH_INTEROP_OUTPUT",
    ] {
        std::env::var_os(variable)
            .with_context(|| format!("set {variable}; see evidence README"))?;
    }
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/evidence/issue-85/tcp_compat.py");
    // Python bounds each operation and owns role cleanup. Wait off Tokio workers;
    // do not kill the runner ahead of its children.
    let outcome =
        tokio::task::spawn_blocking(move || Command::new("python3").arg(script).status()).await;
    anyhow::ensure!(
        outcome.context("join compatibility runner")??.success(),
        "TCP compatibility runner failed; inspect evidence output"
    );
    Ok(())
}
