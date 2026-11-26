//! File-based action shutdown, scoped to the invocation's private workspace.

use std::path::PathBuf;
use std::time::Duration;

/// An action workspace is opted in explicitly; ordinary CLI runs are unaffected.
pub(super) fn workspace() -> Option<PathBuf> {
    std::env::var_os("IROH_TUNNEL_ACTION_SHUTDOWN_DIR").map(PathBuf::from)
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
