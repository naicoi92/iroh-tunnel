//! #95: `update` replaces only the invoked standalone executable.
//!
//! A local release server (`tests/common/release_server.rs`) stands in for
//! GitHub Releases. The release archives contain a tiny shell script that
//! prints its version, so a run after the update proves which file is
//! installed. Real signed artifacts come from #100; until then the signature
//! step is covered through the verifier seam and the fail-closed default.
//!
//! Acceptance map:
//! - 95.CL1 (UPDATE-SELECT-01): latest and `--to VERSION`, only the invoked file changes.
//! - 95.AC1 / CL3 (UPDATE-VERIFY-01, UPDATE-BYPASS-01): verification failure
//!   keeps the old file; `--insecure` bypasses with a warning.
//! - 95.AC2 (UPDATE-RUNNING-01): a running process is not stopped.
#![cfg(unix)]

mod common;

use anyhow::{Context, Result};
use common::release_server::ReleaseServer;
use iroh_tunnel::cli::UpdateArgs;
use iroh_tunnel::update::{update, SignatureVerifier, UpdateContext};
use sha2::{Digest, Sha256};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SERVE: &str = env!("CARGO_BIN_EXE_iroh-tunnel-serve");
const EXE: &str = "iroh-tunnel-serve";
const OLD_BYTES: &[u8] = b"#!/bin/sh\necho OLD\n";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Publish `version` for `target` with a script printing `NEW-<version>`.
/// Returns the path of the checksums file.
fn publish(root: &Path, version: &str, target: &str, tamper_archive: bool) -> Result<()> {
    let dir = root.join(version);
    std::fs::create_dir_all(&dir)?;
    let staging = tempfile::tempdir()?;
    let script = staging.path().join(EXE);
    std::fs::write(&script, format!("#!/bin/sh\necho NEW-{version}\n"))?;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))?;
    let archive = dir.join(format!("{EXE}-{version}-{target}.tar.gz"));
    let status = Command::new("tar")
        .arg("-czf")
        .arg(&archive)
        .arg("-C")
        .arg(staging.path())
        .arg(EXE)
        .status()?;
    assert!(status.success());
    let sum = hex(&Sha256::digest(std::fs::read(&archive)?));
    let name = archive.file_name().unwrap().to_string_lossy().into_owned();
    let checksums = if target.starts_with("darwin-") {
        "checksums-darwin.txt"
    } else {
        "checksums.txt"
    };
    std::fs::write(dir.join(checksums), format!("{sum}  {name}\n"))?;
    std::fs::write(dir.join(format!("{checksums}.sigstore.json")), "{}")?;
    if tamper_archive {
        let mut bytes = std::fs::read(&archive)?;
        bytes.push(0);
        std::fs::write(&archive, bytes)?;
    }
    Ok(())
}

fn host_target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("darwin-arm64"),
        ("linux", "x86_64") if !cfg!(target_env = "musl") => Some("linux-amd64"),
        ("linux", "aarch64") if !cfg!(target_env = "musl") => Some("linux-arm64"),
        _ => None,
    }
}

/// Installed copy of the real serve binary, plus an unrelated sibling file.
struct Install {
    _dir: tempfile::TempDir,
    exe: PathBuf,
    sibling: PathBuf,
}

fn install_real_binary() -> Result<Install> {
    let dir = tempfile::tempdir()?;
    let exe = dir.path().join(EXE);
    std::fs::copy(SERVE, &exe)?;
    let sibling = dir.path().join("iroh-tunnel-access");
    std::fs::write(&sibling, b"unrelated")?;
    Ok(Install {
        _dir: dir,
        exe,
        sibling,
    })
}

fn run_update(exe: &Path, base_url: &str, args: &[&str]) -> Result<Output> {
    Command::new(exe)
        .arg("update")
        .args(args)
        .env("IROH_TUNNEL_BASE_URL", base_url)
        .output()
        .context("cannot run update")
}

fn run_installed(exe: &Path) -> Result<String> {
    let out = Command::new(exe).output()?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

#[test]
fn insecure_update_installs_latest_and_warns() -> Result<()> {
    let Some(target) = host_target() else {
        return Ok(());
    };
    let releases = tempfile::tempdir()?;
    publish(releases.path(), "9.9.9", target, false)?;
    let server = ReleaseServer::start(releases.path().to_owned(), "9.9.9")?;
    let install = install_real_binary()?;

    let out = run_update(&install.exe, &server.url, &["--insecure"])?;

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("--insecure"));
    assert_eq!(run_installed(&install.exe)?, "NEW-9.9.9");
    assert_eq!(std::fs::read(&install.sibling)?, b"unrelated");
    Ok(())
}

#[test]
fn short_flag_and_explicit_version_select_that_release() -> Result<()> {
    let Some(target) = host_target() else {
        return Ok(());
    };
    let releases = tempfile::tempdir()?;
    publish(releases.path(), "9.9.9", target, false)?;
    publish(releases.path(), "9.9.8", target, false)?;
    let server = ReleaseServer::start(releases.path().to_owned(), "9.9.9")?;
    let install = install_real_binary()?;

    let out = run_update(&install.exe, &server.url, &["-i", "--to", "9.9.8"])?;

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(run_installed(&install.exe)?, "NEW-9.9.8");
    Ok(())
}

#[test]
fn update_without_verification_material_keeps_installed_binary() -> Result<()> {
    let Some(target) = host_target() else {
        return Ok(());
    };
    let releases = tempfile::tempdir()?;
    publish(releases.path(), "9.9.9", target, false)?;
    let server = ReleaseServer::start(releases.path().to_owned(), "9.9.9")?;
    let install = install_real_binary()?;
    let before = std::fs::read(&install.exe)?;

    let out = run_update(&install.exe, &server.url, &[])?;

    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("signature"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(std::fs::read(&install.exe)?, before);
    Ok(())
}

#[test]
fn unknown_version_fails_and_keeps_installed_binary() -> Result<()> {
    let Some(target) = host_target() else {
        return Ok(());
    };
    let releases = tempfile::tempdir()?;
    publish(releases.path(), "9.9.9", target, false)?;
    let server = ReleaseServer::start(releases.path().to_owned(), "9.9.9")?;
    let install = install_real_binary()?;
    let before = std::fs::read(&install.exe)?;

    let out = run_update(&install.exe, &server.url, &["-i", "--to", "1.2.3"])?;

    assert!(!out.status.success());
    assert_eq!(std::fs::read(&install.exe)?, before);
    Ok(())
}

#[test]
fn invalid_version_is_rejected_before_any_request() -> Result<()> {
    let install = install_real_binary()?;
    let out = run_update(&install.exe, "http://127.0.0.1:9", &["--to", "../x"])?;
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("invalid version"));
    Ok(())
}

#[test]
fn update_does_not_stop_a_running_process_of_the_old_binary() -> Result<()> {
    let Some(target) = host_target() else {
        return Ok(());
    };
    let releases = tempfile::tempdir()?;
    publish(releases.path(), "9.9.9", target, false)?;
    let server = ReleaseServer::start(releases.path().to_owned(), "9.9.9")?;
    let install = install_real_binary()?;
    // A live tunnel started from the very file that is about to be replaced.
    let mut running = Command::new(&install.exe)
        .args(["--target", "127.0.0.1:9", "--name", "update-test"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    std::thread::sleep(std::time::Duration::from_secs(2));
    assert!(running.try_wait()?.is_none(), "tunnel did not start");

    let out = run_update(&install.exe, &server.url, &["-i"])?;

    let still_running = running.try_wait()?.is_none();
    let _ = running.kill();
    let _ = running.wait();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(still_running, "the running tunnel must not be stopped");
    assert!(String::from_utf8_lossy(&out.stdout).contains("restart"));
    assert_eq!(run_installed(&install.exe)?, "NEW-9.9.9");
    Ok(())
}

// ---- in-process: checksum and signature seams -----------------------------

struct Accept;
impl SignatureVerifier for Accept {
    fn verify(&self, _: &[u8], _: &[u8]) -> Result<()> {
        Ok(())
    }
}
struct Reject;
impl SignatureVerifier for Reject {
    fn verify(&self, _: &[u8], _: &[u8]) -> Result<()> {
        anyhow::bail!("wrong identity")
    }
}

fn in_process(
    server: &ReleaseServer,
    verifier: &dyn SignatureVerifier,
    install: &Path,
    args: &UpdateArgs,
) -> Result<()> {
    let ctx = UpdateContext {
        base_url: server.url.clone(),
        exe_name: EXE,
        target: "linux-amd64",
        current_version: "0.0.1",
        install_path: install.to_owned(),
        verifier,
    };
    tokio::runtime::Runtime::new()?.block_on(update(args, &ctx))
}

fn args(to: Option<&str>, insecure: bool) -> UpdateArgs {
    UpdateArgs {
        to: to.map(str::to_owned),
        insecure,
    }
}

fn old_install() -> Result<(tempfile::TempDir, PathBuf)> {
    let dir = tempfile::tempdir()?;
    let exe = dir.path().join(EXE);
    std::fs::write(&exe, OLD_BYTES)?;
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755))?;
    Ok((dir, exe))
}

#[test]
fn verified_release_replaces_only_the_invoked_file() -> Result<()> {
    let releases = tempfile::tempdir()?;
    publish(releases.path(), "9.9.9", "linux-amd64", false)?;
    let server = ReleaseServer::start(releases.path().to_owned(), "9.9.9")?;
    let (dir, exe) = old_install()?;
    let sibling = dir.path().join("iroh-tunnel-access");
    std::fs::write(&sibling, b"unrelated")?;

    in_process(&server, &Accept, &exe, &args(None, false))?;

    assert_eq!(run_installed(&exe)?, "NEW-9.9.9");
    assert_eq!(std::fs::read(&sibling)?, b"unrelated");
    Ok(())
}

#[test]
fn changed_payload_fails_checksum_and_keeps_old_binary() -> Result<()> {
    let releases = tempfile::tempdir()?;
    publish(releases.path(), "9.9.9", "linux-amd64", true)?;
    let server = ReleaseServer::start(releases.path().to_owned(), "9.9.9")?;
    let (_dir, exe) = old_install()?;

    let err = in_process(&server, &Accept, &exe, &args(None, false)).unwrap_err();

    assert!(format!("{err:#}").contains("checksum mismatch"), "{err:#}");
    assert_eq!(std::fs::read(&exe)?, OLD_BYTES);
    Ok(())
}

#[test]
fn wrong_identity_fails_and_keeps_old_binary() -> Result<()> {
    let releases = tempfile::tempdir()?;
    publish(releases.path(), "9.9.9", "linux-amd64", false)?;
    let server = ReleaseServer::start(releases.path().to_owned(), "9.9.9")?;
    let (_dir, exe) = old_install()?;

    let err = in_process(&server, &Reject, &exe, &args(None, false)).unwrap_err();

    assert!(format!("{err:#}").contains("wrong identity"), "{err:#}");
    assert_eq!(std::fs::read(&exe)?, OLD_BYTES);
    Ok(())
}

#[test]
fn missing_signature_bundle_fails_and_keeps_old_binary() -> Result<()> {
    let releases = tempfile::tempdir()?;
    publish(releases.path(), "9.9.9", "linux-amd64", false)?;
    std::fs::remove_file(releases.path().join("9.9.9/checksums.txt.sigstore.json"))?;
    let server = ReleaseServer::start(releases.path().to_owned(), "9.9.9")?;
    let (_dir, exe) = old_install()?;

    let err = in_process(&server, &Accept, &exe, &args(None, false)).unwrap_err();

    assert!(format!("{err:#}").contains("signature bundle"), "{err:#}");
    assert_eq!(std::fs::read(&exe)?, OLD_BYTES);
    Ok(())
}

#[test]
fn insecure_skips_checksum_of_a_changed_payload() -> Result<()> {
    let releases = tempfile::tempdir()?;
    publish(releases.path(), "9.9.9", "linux-amd64", true)?;
    let server = ReleaseServer::start(releases.path().to_owned(), "9.9.9")?;
    let (_dir, exe) = old_install()?;

    in_process(&server, &Reject, &exe, &args(None, true))?;

    assert_eq!(run_installed(&exe)?, "NEW-9.9.9");
    Ok(())
}

#[test]
fn latest_never_downgrades_a_newer_installed_version() -> Result<()> {
    let releases = tempfile::tempdir()?;
    publish(releases.path(), "0.0.5", "linux-amd64", false)?;
    let server = ReleaseServer::start(releases.path().to_owned(), "0.0.5")?;
    let (_dir, exe) = old_install()?;
    let ctx = UpdateContext {
        base_url: server.url.clone(),
        exe_name: EXE,
        target: "linux-amd64",
        current_version: "0.1.0",
        install_path: exe.clone(),
        verifier: &Accept,
    };

    tokio::runtime::Runtime::new()?.block_on(update(&args(None, false), &ctx))?;

    assert_eq!(std::fs::read(&exe)?, OLD_BYTES);
    Ok(())
}

#[test]
fn insecure_with_explicit_version_needs_no_checksums_file() -> Result<()> {
    let releases = tempfile::tempdir()?;
    publish(releases.path(), "9.9.9", "linux-amd64", false)?;
    std::fs::remove_file(releases.path().join("9.9.9/checksums.txt"))?;
    let server = ReleaseServer::start(releases.path().to_owned(), "9.9.9")?;
    let (_dir, exe) = old_install()?;

    in_process(&server, &Reject, &exe, &args(Some("9.9.9"), true))?;

    assert_eq!(run_installed(&exe)?, "NEW-9.9.9");
    Ok(())
}

#[test]
fn unwritable_install_dir_exits_with_permission_code_and_keeps_binary() -> Result<()> {
    let Some(target) = host_target() else {
        return Ok(());
    };
    let releases = tempfile::tempdir()?;
    publish(releases.path(), "9.9.9", target, false)?;
    let server = ReleaseServer::start(releases.path().to_owned(), "9.9.9")?;
    let install = install_real_binary()?;
    let dir = install.exe.parent().unwrap().to_owned();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555))?;
    let probe_ok = std::fs::File::create(dir.join("probe")).is_ok();
    let before = std::fs::read(&install.exe)?;

    let out = run_update(&install.exe, &server.url, &["-i"])?;

    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755))?;
    if probe_ok {
        return Ok(()); // running as root: permission cannot be denied
    }
    assert_eq!(
        out.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(std::fs::read(&install.exe)?, before);
    Ok(())
}
