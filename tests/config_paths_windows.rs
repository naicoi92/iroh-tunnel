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

fn local_app_data() -> PathBuf {
    dirs::data_local_dir()
        .expect("LocalAppData known folder")
        .join("iroh-tunnel")
}

#[test]
fn config_default_is_local_app_data_ignoring_xdg() {
    let other = tempfile::tempdir().unwrap();
    let xdg = other.path().join("xdg-config");
    for role in ["serve", "access"] {
        let out = tunnel(
            &[role, "config", "path"],
            &[("XDG_CONFIG_HOME", &xdg), ("HOME", other.path())],
        );
        assert!(out.status.success(), "{}", stderr(&out));
        let expected = local_app_data().join("config").join(format!("{role}.toml"));
        assert_eq!(stdout(&out).trim(), expected.display().to_string());
    }
    assert!(!xdg.exists(), "XDG_CONFIG_HOME must be ignored");
}

#[test]
fn state_default_is_local_app_data_state() {
    let other = tempfile::tempdir().unwrap();
    let out = tunnel(&["serve", "status"], &[("XDG_STATE_HOME", other.path())]);
    assert!(!out.status.success());
    let expected = local_app_data().join("state").join("serve-status.json");
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
}

#[test]
fn explicit_state_override_wins() {
    let dir = tempfile::tempdir().unwrap();
    let out = tunnel(
        &["access", "status"],
        &[("IROH_TUNNEL_STATE_DIR", dir.path())],
    );
    assert!(
        stderr(&out).contains(&dir.path().join("access-status.json").display().to_string()),
        "{}",
        stderr(&out)
    );
}
