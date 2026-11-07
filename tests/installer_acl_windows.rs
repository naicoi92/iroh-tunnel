//! Task 12 (spec: install-release-packaging): the directory `install.ps1`
//! creates is writable only by SYSTEM, Administrators and TrustedInstaller,
//! so the installed binary passes the service binary trust check; a directory
//! that already exists is never touched.
//!
//! Opt-in (`IROH_TUNNEL_E2E_SERVICE=1`): it creates a directory under
//! `%ProgramFiles%` and needs an elevated session.

#![cfg(windows)]

mod common;

use anyhow::{ensure, Context, Result};
use common::release_server::ReleaseServer;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const EXES: [&str; 3] = ["iroh-tunnel", "iroh-tunnel-serve", "iroh-tunnel-access"];
const VERSION: &str = "9.9.9";

fn enabled() -> bool {
    if std::env::var_os("IROH_TUNNEL_E2E_SERVICE").is_none() {
        eprintln!(
            "skipped: set IROH_TUNNEL_E2E_SERVICE=1 (creates a directory under Program Files)"
        );
        return false;
    }
    true
}

fn target() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "windows-arm64"
    } else {
        "windows-amd64"
    }
}

/// Removes the install directory, whoever owns it, when the test ends.
struct DirGuard(PathBuf);
impl Drop for DirGuard {
    fn drop(&mut self) {
        let dir = self.0.to_string_lossy().into_owned();
        let _ = pwsh(&format!(
            "if (Test-Path -LiteralPath '{dir}') {{ takeown /f '{dir}' /r /d y | Out-Null; icacls '{dir}' /grant '*S-1-5-32-544:(OI)(CI)F' /t | Out-Null; Remove-Item -LiteralPath '{dir}' -Recurse -Force }}"
        ));
    }
}

fn pwsh(script: &str) -> Result<Output> {
    Command::new("pwsh")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .context("spawn pwsh")
}

fn sddl(dir: &Path) -> Result<String> {
    let out = pwsh(&format!("(Get-Acl -LiteralPath '{}').Sddl", dir.display()))?;
    ensure!(out.status.success(), "Get-Acl failed: {out:?}");
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn test_dir() -> PathBuf {
    let base = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
    PathBuf::from(base).join(format!("iroh-tunnel-test-{}", std::process::id()))
}

fn current_user_sid() -> Result<String> {
    let out = pwsh("[System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value")?;
    ensure!(out.status.success(), "{out:?}");
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Serve a release and run `install.ps1 -Yes -BinDir dir` against it.
fn run_installer(dir: &Path) -> Result<Output> {
    let releases = tempfile::tempdir()?;
    let exes: Vec<(&str, String, &[u8])> = EXES
        .iter()
        .map(|e| (*e, format!("{e}.exe"), b"installer test bytes".as_slice()))
        .collect();
    let refs: Vec<(&str, &str, &[u8])> =
        exes.iter().map(|(e, n, c)| (*e, n.as_str(), *c)).collect();
    common::publish_zips(releases.path(), VERSION, target(), &refs)?;
    let server = ReleaseServer::start(releases.path().to_owned(), VERSION)?;
    let scratch = tempfile::tempdir()?;
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("install.ps1");
    Command::new("pwsh")
        .args(["-NoProfile", "-NonInteractive", "-File"])
        .arg(script)
        .args(["-Yes", "-BinDir"])
        .arg(dir)
        .env("IROH_TUNNEL_BASE_URL", &server.url)
        .env("IROH_TUNNEL_TARGET", target())
        .env("IROH_TUNNEL_PATH_FILE", scratch.path().join("path.txt"))
        .output()
        .context("spawn install.ps1")
}

#[test]
fn installer_creates_a_directory_that_passes_the_binary_check() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let dir = test_dir();
    let _guard = DirGuard(dir.clone());

    let out = run_installer(&dir)?;
    ensure!(out.status.success(), "install.ps1 failed: {out:?}");

    iroh_tunnel_host::service::__test_verify_binary(&dir.join("iroh-tunnel.exe"))?;
    let sddl = sddl(&dir)?;
    assert!(
        sddl.starts_with("O:BA"),
        "owner must be Administrators: {sddl}"
    );
    let me = current_user_sid()?;
    assert!(
        !sddl.contains(&me),
        "an ACE names the installing user: {sddl}"
    );
    assert!(
        !sddl.contains(";;;CO)"),
        "an ACE names CREATOR OWNER: {sddl}"
    );
    Ok(())
}

#[test]
fn installer_leaves_an_existing_directory_untouched() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let dir = test_dir();
    let _guard = DirGuard(dir.clone());
    std::fs::create_dir(&dir)?;
    let before = sddl(&dir)?;

    let out = run_installer(&dir)?;
    ensure!(out.status.success(), "install.ps1 failed: {out:?}");

    assert_eq!(sddl(&dir)?, before);
    Ok(())
}
