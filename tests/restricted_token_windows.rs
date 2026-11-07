//! Self-test of the restricted-token helper: the child it launches from the
//! elevated runner must be Medium integrity and not elevated. A launch
//! failure is a hard failure, never a skip.
//!
//! Opt-in (`IROH_TUNNEL_E2E_SERVICE=1`).

#![cfg(windows)]

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
    assert_eq!(
        out.status.code(),
        Some(0),
        "{text}{}",
        String::from_utf8_lossy(&out.stderr)
    );
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
/// child: a `--system` install must be refused as a permission error before
/// anything else runs (Option A: the real CLI, no test-only probe).
#[test]
fn restricted_child_is_not_elevated_per_product() {
    if std::env::var_os("IROH_TUNNEL_E2E_SERVICE").is_none() {
        eprintln!("skipped: set IROH_TUNNEL_E2E_SERVICE=1 to launch restricted-token children");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let config = dir.path().join("serve.toml");
    let out = run_non_elevated(Command::new(env!("CARGO_BIN_EXE_iroh-tunnel")).args([
        "serve",
        "service",
        "install",
        "--system",
        "--config",
        config.to_str().unwrap(),
    ]))
    .expect("launch iroh-tunnel under a restricted token");
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(3), "{err}");
    assert!(err.contains("Run as administrator"), "{err}");
}
