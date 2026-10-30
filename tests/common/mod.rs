//! Helpers shared by integration tests.
// Each test crate uses a different subset of these helpers.
#![allow(dead_code)]

use std::path::Path;

/// Role-ownership runtime state (#87): lock and owner records in the default
/// state directory. They are neither configuration nor keys.
pub fn is_ownership_artifact(path: &Path) -> bool {
    // `machine` is the test-build stand-in for the machine-wide lock dir.
    let in_state_dir = path.parent().is_some_and(|dir| {
        dir.ends_with(".local/state/iroh-tunnel")
            || dir.ends_with(".local/state/iroh-tunnel/machine")
    });
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    in_state_dir && (name.ends_with(".lock") || name.contains(".owner.json"))
}
