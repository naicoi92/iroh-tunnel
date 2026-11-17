//! Crate-wide fixtures of the unit tests.

#[cfg(unix)]
use std::path::{Path, PathBuf};

/// Re-run the named test in a child process whose `HOME` is a fresh
/// directory. Use it for tests that touch process-wide state (`HOME`-keyed
/// registration paths and role locks, the umask): this process is
/// multi-threaded and runs other tests in parallel.
#[cfg(unix)]
pub(crate) fn in_isolated_home(name: &str, body: impl FnOnce(&Path)) {
    const CHILD_MARKER: &str = "IROH_TUNNEL_TEST_CHILD";
    if std::env::var_os(CHILD_MARKER).is_none() {
        let home = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--nocapture"])
            .env(CHILD_MARKER, "1")
            .env("HOME", home.path())
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "child failed:\n{stdout}\n{stderr}");
        assert!(
            stdout.contains(&format!("IROH_CHILD_BODY_RAN {name}")),
            "child did not run the test body (0 tests matched?):\n{stdout}\n{stderr}"
        );
        return;
    }
    body(&PathBuf::from(std::env::var_os("HOME").unwrap()));
    println!("IROH_CHILD_BODY_RAN {name}");
}
