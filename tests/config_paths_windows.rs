//! Issue #84 on Windows: LocalAppData config/state defaults and explicit
//! overrides (CFG-01; acceptance case CFG-PATH-01). Unix counterparts live in
//! `config_paths.rs`.
//!
//! The Windows known folder cannot be redirected per process, so every test
//! is read-only: it asks the real binary which path it would use (`config
//! path`, `status`) and compares it with the installing user's LocalAppData
//! folder, so the developer's real configuration is never written.
#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn tunnel(args: &[&str], envs: &[(&str, &Path)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_iroh-tunnel"));
    cmd.args(args)
        .env_remove("IROH_TUNNEL_TEST_CONFIG_DIR")
        .env_remove("IROH_TUNNEL_STATE_DIR");
    for (key, value) in envs {
        cmd.env(key, value);
    }
    cmd.output().expect("spawn iroh-tunnel")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn app_data_root() -> PathBuf {
    dirs::data_local_dir()
        .expect("LocalAppData known folder")
        .join("iroh-tunnel")
}

#[test]
fn config_default_is_local_app_data_ignoring_xdg() {
    let scratch_home = tempfile::tempdir().unwrap();
    let xdg = scratch_home.path().join("xdg-config");
    for role in ["serve", "access"] {
        let out = tunnel(
            &[role, "config", "path"],
            &[("XDG_CONFIG_HOME", &xdg), ("HOME", scratch_home.path())],
        );
        assert!(out.status.success(), "{}", stderr(&out));
        let expected = app_data_root().join("config").join(format!("{role}.toml"));
        assert_eq!(stdout(&out).trim(), expected.display().to_string());
    }
}

#[test]
fn state_default_is_local_app_data_state() {
    let scratch = tempfile::tempdir().unwrap();
    let out = tunnel(&["serve", "status"], &[("XDG_STATE_HOME", scratch.path())]);
    if out.status.success() {
        // A real status file exists on this host; the table does not print
        // its location, so there is nothing path-related left to assert.
        return;
    }
    // "not running" is a general error (exit 1) naming the file it looked for.
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let expected = app_data_root().join("state").join("serve-status.json");
    assert!(
        stderr(&out).contains(&expected.display().to_string()),
        "status must look under LocalAppData\\iroh-tunnel\\state: {}",
        stderr(&out)
    );
}

#[test]
fn explicit_config_override_wins() {
    let dir = tempfile::tempdir().unwrap();
    let custom = dir.path().join("custom.toml");
    let default = app_data_root().join("config").join("serve.toml");
    let existed = default.exists();
    let out = tunnel(
        &[
            "serve",
            "config",
            "keygen",
            "--config",
            custom.to_str().unwrap(),
        ],
        &[],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(custom.is_file());
    assert_eq!(
        default.exists(),
        existed,
        "the default config location must be left alone"
    );
}

#[test]
fn explicit_state_override_wins() {
    let dir = tempfile::tempdir().unwrap();
    let out = tunnel(
        &["access", "status"],
        &[("IROH_TUNNEL_STATE_DIR", dir.path())],
    );
    // The override directory is empty, so "not running" (exit 1) is certain.
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(
        stderr(&out).contains(&dir.path().join("access-status.json").display().to_string()),
        "{}",
        stderr(&out)
    );
}
