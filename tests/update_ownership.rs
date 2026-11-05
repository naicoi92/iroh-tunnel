//! #96: managed installs are updated through their owner (AC-DIST-04/07).
//!
//! In-process cases use the `owner` of `UpdateContext`; the process-level case
//! runs the real binary with `IROH_TUNNEL_CONTAINER=1`, the variable the
//! project's images set. Real package managers and images are exercised by
//! `packaging/verify-dist.sh` and the final-artifact run (#97).
#![cfg(unix)]

mod common;

use anyhow::Result;
use common::release_server::ReleaseServer;
use iroh_tunnel_host::update::Owner;
use iroh_tunnel_host::update::{
    check_owner, update, OverwritePolicy, OwnerDecision, SignatureVerifier, UpdateContext,
    UpdateRequest, Verification,
};
use sha2::{Digest, Sha256};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const SERVE: &str = env!("CARGO_BIN_EXE_iroh-tunnel-serve");
const EXE: &str = "iroh-tunnel-serve";
const OLD: &[u8] = b"#!/bin/sh\necho OLD\n";

struct Accept;
impl SignatureVerifier for Accept {
    fn verify(&self, _: &[u8], _: &[u8]) -> Result<()> {
        Ok(())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn publish(root: &Path) -> Result<()> {
    let dir = root.join("9.9.9");
    std::fs::create_dir_all(&dir)?;
    let staging = tempfile::tempdir()?;
    let script = staging.path().join(EXE);
    std::fs::write(&script, "#!/bin/sh\necho NEW\n")?;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))?;
    let name = format!("{EXE}-9.9.9-linux-amd64.tar.gz");
    let archive = dir.join(&name);
    assert!(Command::new("tar")
        .arg("-czf")
        .arg(&archive)
        .arg("-C")
        .arg(staging.path())
        .arg(EXE)
        .status()?
        .success());
    std::fs::write(
        dir.join("checksums.txt"),
        format!(
            "{}  {name}\n",
            hex(&Sha256::digest(std::fs::read(&archive)?))
        ),
    )?;
    std::fs::write(dir.join("checksums.txt.sigstore.json"), "{}")?;
    Ok(())
}

struct Fixture {
    _releases: tempfile::TempDir,
    server: ReleaseServer,
    _dir: tempfile::TempDir,
    exe: PathBuf,
}

fn fixture() -> Result<Fixture> {
    let releases = tempfile::tempdir()?;
    publish(releases.path())?;
    let server = ReleaseServer::start(releases.path().to_owned(), "9.9.9")?;
    let dir = tempfile::tempdir()?;
    let exe = dir.path().join(EXE);
    std::fs::write(&exe, OLD)?;
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755))?;
    Ok(Fixture {
        _releases: releases,
        server,
        _dir: dir,
        exe,
    })
}

fn run(f: &Fixture, owner: Owner, insecure: bool, yes: bool) -> Result<()> {
    let ctx = UpdateContext {
        base_url: f.server.url.clone(),
        exe_name: EXE,
        target: "linux-amd64",
        current_version: "0.0.1",
        install_path: f.exe.clone(),
        verifier: &Accept,
        owner,
    };
    let req = UpdateRequest {
        to: None,
        verification: if insecure {
            Verification::Insecure
        } else {
            Verification::Verified
        },
        overwrite: if yes {
            OverwritePolicy::ReplacePackageOwned
        } else {
            OverwritePolicy::RespectOwner
        },
    };
    // The updater does not check ownership; the CLI wrapper does, in this order.
    if matches!(
        check_owner(&ctx, req.overwrite),
        OwnerDecision::Delegate { .. }
    ) {
        return Ok(());
    }
    tokio::runtime::Runtime::new()?.block_on(update(&req, &ctx))?;
    Ok(())
}

fn deb() -> Owner {
    Owner::LinuxPackage {
        package: EXE.into(),
    }
}

#[test]
fn package_owned_file_is_not_replaced_by_default() -> Result<()> {
    let f = fixture()?;
    run(&f, deb(), false, false)?;
    assert_eq!(std::fs::read(&f.exe)?, OLD);
    Ok(())
}

#[test]
fn insecure_does_not_override_package_ownership() -> Result<()> {
    let f = fixture()?;
    run(&f, deb(), true, false)?;
    assert_eq!(std::fs::read(&f.exe)?, OLD);
    Ok(())
}

#[test]
fn package_owned_file_is_replaced_only_with_yes() -> Result<()> {
    let f = fixture()?;
    run(&f, deb(), true, true)?;
    assert_ne!(std::fs::read(&f.exe)?, OLD);
    Ok(())
}

#[test]
fn homebrew_owned_file_follows_the_same_rule() -> Result<()> {
    let f = fixture()?;
    run(&f, Owner::Homebrew, true, false)?;
    assert_eq!(std::fs::read(&f.exe)?, OLD);
    run(&f, Owner::Homebrew, true, true)?;
    assert_ne!(std::fs::read(&f.exe)?, OLD);
    Ok(())
}

#[test]
fn container_image_is_never_overwritten_even_with_yes() -> Result<()> {
    let f = fixture()?;
    run(&f, Owner::Container, true, true)?;
    assert_eq!(std::fs::read(&f.exe)?, OLD);
    Ok(())
}

#[test]
fn only_the_invoked_file_changes_when_a_package_owner_is_overridden() -> Result<()> {
    let f = fixture()?;
    let sibling = f.exe.with_file_name("iroh-tunnel-access");
    std::fs::write(&sibling, b"unrelated")?;
    run(&f, deb(), true, true)?;
    assert_eq!(std::fs::read(&sibling)?, b"unrelated");
    Ok(())
}

/// The real binary inside a container (env set by the images): guidance, exit 0, file untouched.
#[test]
fn real_binary_in_a_container_prints_redeploy_guidance_and_changes_nothing() -> Result<()> {
    let f = fixture()?;
    let installed = f.exe.with_file_name("copy");
    std::fs::create_dir(&installed)?;
    let exe = installed.join(EXE);
    std::fs::copy(SERVE, &exe)?;
    let before = std::fs::read(&exe)?;

    let out = Command::new(&exe)
        .args(["update", "-i", "--yes"])
        .env("IROH_TUNNEL_BASE_URL", &f.server.url)
        .env("IROH_TUNNEL_CONTAINER", "1")
        .output()?;

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("redeploy"), "{stdout}");
    assert_eq!(std::fs::read(&exe)?, before);
    Ok(())
}
