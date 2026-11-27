//! File-based action shutdown, scoped to the invocation's private workspace.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::io::AsyncWriteExt;

/// An action workspace is opted in explicitly; ordinary CLI runs are unaffected.
pub(super) fn workspace() -> Option<PathBuf> {
    std::env::var_os("IROH_TUNNEL_ACTION_SHUTDOWN_DIR").map(PathBuf::from)
}

/// Claim this single-use channel before running anything that could publish
/// completion. A duplicate invocation must not acknowledge another's shutdown.
pub(super) async fn claim(dir: &Path) -> std::io::Result<()> {
    let mut ready = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.join("shutdown-ready"))
        .await?;
    ready.write_all(b"ready").await
}

/// Publish runtime completion with the publisher's PID. The action must still
/// observe OS process exit before accepting this as shutdown confirmation:
/// returning from the runtime is not the same as exiting the process.
pub(super) async fn complete(dir: &Path) -> std::io::Result<()> {
    let record = format!(r#"{{"version":1,"pid":{}}}"#, std::process::id());
    tokio::fs::write(dir.join("shutdown-complete"), record).await
}

pub(super) async fn wait(dir: PathBuf) {
    loop {
        if tokio::fs::read(dir.join("shutdown-request"))
            .await
            .is_ok_and(|request| request == b"shutdown")
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
