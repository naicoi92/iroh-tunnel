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

const TRUSTED_INSTALLER: &str = "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";
const WRITE_BITS: [i64; 8] = [
    0x2, 0x4, 0x40, 0x10000, 0x40000, 0x80000, 0x10000000, 0x40000000,
];

fn sddl(dir: &Path) -> Result<String> {
    let out = pwsh(&format!("(Get-Acl -LiteralPath '{}').Sddl", dir.display()))?;
    ensure!(out.status.success(), "Get-Acl failed: {out:?}");
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Per-test directory under `base` (default `%ProgramFiles%`).
fn test_dir_in(base: Option<&Path>, name: &str) -> PathBuf {
    let base = base.map(Path::to_path_buf).unwrap_or_else(|| {
        PathBuf::from(std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into()))
    });
    base.join(format!("iroh-tunnel-test-{}-{name}", std::process::id()))
}

fn test_dir(name: &str) -> PathBuf {
    test_dir_in(None, name)
}

fn current_user_sid() -> Result<String> {
    let out = pwsh("[System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value")?;
    ensure!(out.status.success(), "{out:?}");
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// `(sid, rights, type)` for every explicit and inherited rule.
fn rules(dir: &Path) -> Result<Vec<(String, i64, String)>> {
    let out = pwsh(&format!(
        "(Get-Acl -LiteralPath '{}').GetAccessRules($true,$true,[Security.Principal.SecurityIdentifier]) | ForEach-Object {{ \"$($_.IdentityReference.Value)|$([int]$_.FileSystemRights)|$($_.AccessControlType)\" }}",
        dir.display()
    ))?;
    ensure!(out.status.success(), "Get-Acl failed: {out:?}");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let mut p = l.trim().split('|');
            let (sid, rights, kind) = (p.next(), p.next(), p.next());
            match (sid, rights, kind) {
                (Some(s), Some(r), Some(k)) => Ok((s.to_owned(), r.parse()?, k.to_owned())),
                _ => anyhow::bail!("bad rule line: {l}"),
            }
        })
        .collect()
}

fn owner_sid(dir: &Path) -> Result<String> {
    let out = pwsh(&format!(
        "(Get-Acl -LiteralPath '{}').GetOwner([Security.Principal.SecurityIdentifier]).Value",
        dir.display()
    ))?;
    ensure!(out.status.success(), "Get-Acl failed: {out:?}");
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// The whitelist DACL: only the four known SIDs, Users read-only, owner BA,
/// and never the installing user.
fn assert_protected(dir: &Path) -> Result<()> {
    let allowed = [
        "S-1-5-18",
        "S-1-5-32-544",
        TRUSTED_INSTALLER,
        "S-1-5-32-545",
    ];
    let me = current_user_sid()?;
    let rules = rules(dir)?;
    ensure!(!rules.is_empty(), "no rules read for {}", dir.display());
    for (sid, rights, kind) in &rules {
        assert_ne!(*sid, me, "an ACE names the installing user");
        if kind == "Allow" {
            assert!(
                allowed.contains(&sid.as_str()),
                "unexpected Allow SID {sid}"
            );
        }
        if sid == "S-1-5-32-545" {
            for bit in WRITE_BITS {
                assert_eq!(
                    rights & bit,
                    0,
                    "Users rule {rights:#x} has write bit {bit:#x}"
                );
            }
        }
    }
    assert_eq!(
        owner_sid(dir)?,
        "S-1-5-32-544",
        "owner must be Administrators"
    );
    Ok(())
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
    let dir = test_dir("creates");
    let _guard = DirGuard(dir.clone());

    let out = run_installer(&dir)?;
    ensure!(out.status.success(), "install.ps1 failed: {out:?}");

    iroh_tunnel_host::service::__test_verify_binary(&dir.join("iroh-tunnel.exe"))?;
    assert_protected(&dir)
}

/// I-1 regression: a parent that grants Users modify must not leak into the
/// new directory.
#[test]
fn installer_ignores_a_parent_that_grants_users_modify() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let pd = std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".into());
    let parent = test_dir_in(Some(Path::new(&pd)), "parent");
    let _parent_guard = DirGuard(parent.clone());
    std::fs::create_dir(&parent)?;
    let out = pwsh(&format!(
        "icacls '{}' /grant '*S-1-5-32-545:(OI)(CI)M'",
        parent.display()
    ))?;
    ensure!(out.status.success(), "icacls failed: {out:?}");
    let dir = parent.join("bin");

    let out = run_installer(&dir)?;
    ensure!(out.status.success(), "install.ps1 failed: {out:?}");

    assert_protected(&dir)
}

#[test]
fn installer_refuses_a_missing_parent_and_creates_nothing() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let parent = test_dir("missing-parent");
    let _guard = DirGuard(parent.clone());
    let dir = parent.join("bin");

    let out = run_installer(&dir)?;
    assert!(!out.status.success(), "must fail: {out:?}");
    assert!(!parent.exists(), "parent was created");
    assert!(!dir.exists(), "directory was created");
    Ok(())
}

#[test]
fn installer_leaves_an_existing_directory_untouched() -> Result<()> {
    if !enabled() {
        return Ok(());
    }
    let dir = test_dir("existing");
    let _guard = DirGuard(dir.clone());
    std::fs::create_dir(&dir)?;
    let before = sddl(&dir)?;

    let out = run_installer(&dir)?;
    ensure!(out.status.success(), "install.ps1 failed: {out:?}");

    assert_eq!(sddl(&dir)?, before);
    Ok(())
}
