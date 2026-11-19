//! Self-test of the restricted-token helper: the child it launches from the
//! elevated runner must be Medium integrity and not elevated. A launch
//! failure is a hard failure, never a skip.
//!
//! Opt-in (`IROH_TUNNEL_E2E_SERVICE=1`).

#![cfg(windows)]
#![cfg(test)]

mod common;

use common::restricted::{run_non_elevated, LAUNCH_API};
use std::process::Command;

#[test]
fn restricted_child_is_not_elevated() {
    if std::env::var_os("IROH_TUNNEL_E2E_SERVICE").is_none() {
        eprintln!("skipped: set IROH_TUNNEL_E2E_SERVICE=1 to launch restricted-token children");
        return;
    }
    let out = run_non_elevated(Command::new("whoami").args(["/groups", "/fo", "csv"]))
        .expect("launch whoami under a restricted token");
    eprintln!("restricted launch API: {}", LAUNCH_API.lock().unwrap());
    let text = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if out.status.code() != Some(0) {
        eprintln!(
            "whoami failed: status={:?}\nLAUNCH_API={}\nstdout:\n{text}\nstderr:\n{stderr}",
            out.status,
            LAUNCH_API.lock().unwrap()
        );
    }
    assert_eq!(out.status.code(), Some(0), "{text}{stderr}");
    assert!(
        text.contains("S-1-16-8192"),
        "not Medium integrity:\n{text}"
    );
    assert!(
        !text.contains("S-1-16-12288"),
        "still High integrity:\n{text}"
    );
    // The deny-only attribute text of the Administrators line is localized,
    // so it is not asserted; integrity level is the locale-independent proof.
}

/// The product's own elevation decision (`is_elevated`) for the restricted
/// child: it is the user tier, so its default config path is the user's own,
/// never `%ProgramData%` (Option A: the real CLI, no test-only probe).
#[test]
fn restricted_child_is_not_elevated_per_product() {
    if std::env::var_os("IROH_TUNNEL_E2E_SERVICE").is_none() {
        eprintln!("skipped: set IROH_TUNNEL_E2E_SERVICE=1 to launch restricted-token children");
        return;
    }
    let out = run_non_elevated(
        Command::new(env!("CARGO_BIN_EXE_iroh-tunnel"))
            .args(["serve", "config", "path"])
            .env_remove("IROH_TUNNEL_TEST_CONFIG_DIR")
            .env_remove("IROH_TUNNEL_TEST_TIER"),
    )
    .expect("launch iroh-tunnel under a restricted token");
    let path = String::from_utf8_lossy(&out.stdout).to_lowercase();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{err}");
    let program_data = std::env::var("ProgramData")
        .expect("ProgramData must be set on Windows")
        .to_lowercase();
    assert!(!path.contains(&program_data), "{path}");
    assert!(path.contains(r"iroh-tunnel\config\serve.toml"), "{path}");
}
