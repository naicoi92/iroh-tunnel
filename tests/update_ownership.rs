//! #96: managed installs are updated through their owner (AC-DIST-04/07).
//!
//! In-process cases use the `owner` of `UpdateContext`; the process-level case
//! runs the real binary with `IROH_TUNNEL_CONTAINER=1`, the variable the
//! project's images set. Real package managers and images are not exercised by
//! the test suite.
#![cfg(unix)]

mod common;

use anyhow::Result;
use common::busy_exec;
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
const NEW: &[u8] = b"#!/bin/sh\necho NEW\n";
const HOMEBREW_CASES: [(&str, &str, &str); 3] = [
    (
        env!("CARGO_BIN_EXE_iroh-tunnel"),
        "iroh-tunnel",
        "brew upgrade --cask iroh-tunnel",
    ),
    (
        SERVE,
        "iroh-tunnel-serve",
        "brew upgrade --cask iroh-tunnel-serve",
    ),
    (
        env!("CARGO_BIN_EXE_iroh-tunnel-access"),
        "iroh-tunnel-access",
        "brew upgrade --cask iroh-tunnel-access",
    ),
];

struct Accept;
impl SignatureVerifier for Accept {
    fn verify(&self, _: &[u8], _: &[u8]) -> Result<()> {
        Ok(())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn publish(root: &Path, exe_name: &str, target: &str) -> Result<()> {
    let dir = root.join("9.9.9");
    std::fs::create_dir_all(&dir)?;
    let staging = tempfile::tempdir()?;
    let script = staging.path().join(exe_name);
    std::fs::write(&script, NEW)?;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))?;
    let name = format!("{exe_name}-9.9.9-{target}.tar.gz");
    let archive = dir.join(&name);
    assert!(Command::new("tar")
        .arg("-czf")
        .arg(&archive)
        .arg("-C")
        .arg(staging.path())
        .arg(exe_name)
        .status()?
        .success());
    let checksums = if target.starts_with("darwin-") {
        "checksums-darwin.txt"
    } else {
        "checksums.txt"
    };
    std::fs::write(
        dir.join(checksums),
        format!(
            "{}  {name}\n",
            hex(&Sha256::digest(std::fs::read(&archive)?))
        ),
    )?;
    std::fs::write(dir.join(format!("{checksums}.sigstore.json")), "{}")?;
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
    publish(releases.path(), EXE, "linux-amd64")?;
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

/// A real executable below the ownership detector's Homebrew prefix, with
/// unrelated siblings. No Homebrew installation or package database is touched.
struct HomebrewInstall {
    dir: tempfile::TempDir,
    exe: PathBuf,
    siblings: Vec<PathBuf>,
}

impl HomebrewInstall {
    fn new(source: &str, exe_name: &str) -> Result<Self> {
        let dir = tempfile::tempdir()?;
        let bin = dir.path().join("Caskroom").join(exe_name).join("0.0.1");
        std::fs::create_dir_all(&bin)?;
        let exe = bin.join(exe_name);
        std::fs::copy(source, &exe)?;
        let mut siblings = Vec::new();
        for (_, name, _) in HOMEBREW_CASES {
            if name != exe_name {
                let sibling = bin.join(name);
                std::fs::write(&sibling, b"unrelated")?;
                siblings.push(sibling);
            }
        }
        Ok(Self { dir, exe, siblings })
    }

    fn command(&self, base_url: &str) -> Command {
        let mut command = Command::new(&self.exe);
        command
            .arg("update")
            .env_remove("IROH_TUNNEL_CONTAINER")
            .env("IROH_TUNNEL_BASE_URL", base_url)
            .env("HOME", self.dir.path())
            .env("XDG_CONFIG_HOME", self.dir.path().join("config"))
            .env("XDG_DATA_HOME", self.dir.path().join("data"))
            .env("XDG_STATE_HOME", self.dir.path().join("state"))
            .env("XDG_CACHE_HOME", self.dir.path().join("cache"))
            .env("IROH_TUNNEL_STATE_DIR", self.dir.path().join("state"));
        command
    }

    fn assert_siblings_unchanged(&self) -> Result<()> {
        for sibling in &self.siblings {
            assert_eq!(std::fs::read(sibling)?, b"unrelated", "{sibling:?}");
        }
        Ok(())
    }
}

#[test]
fn real_homebrew_binaries_delegate_to_their_own_casks() -> Result<()> {
    for (source, exe_name, expected_command) in HOMEBREW_CASES {
        let install = HomebrewInstall::new(source, exe_name)?;
        let before = std::fs::read(&install.exe)?;
        for args in [vec![], vec!["--insecure"]] {
            // Delegation must happen before any release fetch, even insecurely.
            let out = busy_exec::output(install.command("http://127.0.0.1:9").args(&args))?;
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(out.status.success(), "{exe_name} {args:?}: {stderr}");
            assert!(
                stdout.contains(&format!("`{expected_command}`")),
                "{exe_name} {args:?}: {stdout}"
            );
            assert!(stdout.contains("--yes"), "{stdout}");
            assert_eq!(std::fs::read(&install.exe)?, before, "{exe_name} {args:?}");
            install.assert_siblings_unchanged()?;
        }
    }
    Ok(())
}

#[test]
fn confirmed_homebrew_updates_warn_before_replacing_only_the_invoked_file() -> Result<()> {
    let target = iroh_tunnel_host::update::current_target()?;
    for (source, exe_name, _) in HOMEBREW_CASES {
        let releases = tempfile::tempdir()?;
        publish(releases.path(), exe_name, target)?;
        let server = ReleaseServer::start(releases.path().to_owned(), "9.9.9")?;
        let install = HomebrewInstall::new(source, exe_name)?;

        // With confirmation, the warning must precede a failed release fetch too.
        let failed = busy_exec::output(
            install
                .command("http://127.0.0.1:9")
                .args(["--yes", "--insecure"]),
        )?;
        let stderr = String::from_utf8_lossy(&failed.stderr);
        assert!(!failed.status.success(), "{exe_name}: {stderr}");
        let warning = stderr.find("warning: replacing").expect("override warning");
        let error = stderr.find("cannot fetch").expect("release fetch error");
        assert!(warning < error, "{exe_name}: {stderr}");

        let out = busy_exec::output(install.command(&server.url).args(["--yes", "--insecure"]))?;
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{exe_name}: {stderr}");
        assert!(
            stderr.contains("warning: replacing"),
            "{exe_name}: {stderr}"
        );
        assert!(stderr.contains("package manager"), "{exe_name}: {stderr}");
        assert_eq!(std::fs::read(&install.exe)?, NEW, "{exe_name}");
        install.assert_siblings_unchanged()?;
    }
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

    let out = busy_exec::output(
        Command::new(&exe)
            .args(["update", "-i", "--yes"])
            .env("IROH_TUNNEL_BASE_URL", &f.server.url)
            .env("IROH_TUNNEL_CONTAINER", "1"),
    )?;

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
