//! #95: the Windows release format (`.zip`, `<exe>.exe`) and the locked
//! running executable (UPDATE-RECOVERY-01, policy #80).
//!
//! - The zip tests run on every OS: they feed the Windows archive layout to
//!   `update` through the in-process seam with a Windows target id.
//! - The locked-executable tests run on Windows only. Policy #80: replacing a
//!   file the OS keeps locked reports the OS error and keeps the old binary;
//!   no rollback is promised.

mod common;

use anyhow::Result;
use common::release_server::ReleaseServer;
use iroh_tunnel_host::update::{
    update, OverwritePolicy, SignatureVerifier, UpdateContext, UpdateRequest, Verification,
};
use sha2::{Digest, Sha256};
use std::io::Write as _;
use std::path::Path;

const EXE: &str = "iroh-tunnel-serve";
const OLD: &[u8] = b"old executable bytes";
const NEW: &[u8] = b"new executable bytes";

struct Accept;
impl SignatureVerifier for Accept {
    fn verify(&self, _: &[u8], _: &[u8]) -> Result<()> {
        Ok(())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Publish `version` for `target` as a `.zip` holding `entry` with `contents`.
fn publish_zip(
    root: &Path,
    version: &str,
    target: &str,
    entry: &str,
    contents: &[u8],
) -> Result<()> {
    let dir = root.join(version);
    std::fs::create_dir_all(&dir)?;
    let mut buf = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        zip.start_file(entry, zip::write::SimpleFileOptions::default())?;
        zip.write_all(contents)?;
        zip.finish()?;
    }
    let name = format!("{EXE}-{version}-{target}.zip");
    std::fs::write(dir.join(&name), &buf)?;
    std::fs::write(
        dir.join("checksums.txt"),
        format!("{}  {name}\n", hex(&Sha256::digest(&buf))),
    )?;
    std::fs::write(dir.join("checksums.txt.sigstore.json"), "{}")?;
    Ok(())
}

fn run_update(server: &ReleaseServer, target: &str, install: &Path) -> Result<()> {
    let ctx = UpdateContext {
        base_url: server.url.clone(),
        exe_name: EXE,
        target,
        current_version: "0.0.1",
        install_path: install.to_owned(),
        verifier: &Accept,
        owner: iroh_tunnel_host::update::Owner::Standalone,
    };
    let req = UpdateRequest {
        to: None,
        verification: Verification::Verified,
        overwrite: OverwritePolicy::RespectOwner,
    };
    tokio::runtime::Runtime::new()?.block_on(update(&req, &ctx))?;
    Ok(())
}

fn installed(dir: &Path, contents: &[u8]) -> Result<std::path::PathBuf> {
    let path = dir.join(format!("{EXE}.exe"));
    std::fs::write(&path, contents)?;
    Ok(path)
}

#[test]
fn windows_zip_release_replaces_the_installed_exe() -> Result<()> {
    for target in ["windows-amd64", "windows-arm64"] {
        let releases = tempfile::tempdir()?;
        publish_zip(releases.path(), "9.9.9", target, &format!("{EXE}.exe"), NEW)?;
        let server = ReleaseServer::start(releases.path().to_owned(), "9.9.9")?;
        let dir = tempfile::tempdir()?;
        let exe = installed(dir.path(), OLD)?;
        let sibling = dir.path().join("iroh-tunnel-access.exe");
        std::fs::write(&sibling, b"unrelated")?;

        run_update(&server, target, &exe)?;

        assert_eq!(std::fs::read(&exe)?, NEW, "{target}");
        assert_eq!(std::fs::read(&sibling)?, b"unrelated", "{target}");
    }
    Ok(())
}

#[test]
fn windows_zip_without_the_expected_entry_keeps_the_old_exe() -> Result<()> {
    let releases = tempfile::tempdir()?;
    publish_zip(releases.path(), "9.9.9", "windows-amd64", "other.exe", NEW)?;
    let server = ReleaseServer::start(releases.path().to_owned(), "9.9.9")?;
    let dir = tempfile::tempdir()?;
    let exe = installed(dir.path(), OLD)?;

    let err = run_update(&server, "windows-amd64", &exe).unwrap_err();

    assert!(format!("{err:#}").contains("does not contain"), "{err:#}");
    assert_eq!(std::fs::read(&exe)?, OLD);
    Ok(())
}

#[test]
fn windows_zip_with_a_traversal_entry_name_is_not_extracted() -> Result<()> {
    let releases = tempfile::tempdir()?;
    publish_zip(
        releases.path(),
        "9.9.9",
        "windows-amd64",
        &format!("..\\{EXE}.exe"),
        NEW,
    )?;
    let server = ReleaseServer::start(releases.path().to_owned(), "9.9.9")?;
    let dir = tempfile::tempdir()?;
    let exe = installed(dir.path(), OLD)?;

    assert!(run_update(&server, "windows-amd64", &exe).is_err());
    assert_eq!(std::fs::read(&exe)?, OLD);
    Ok(())
}

#[cfg(windows)]
mod locked {
    use super::*;
    use std::process::{Command, Stdio};

    const SERVE: &str = env!("CARGO_BIN_EXE_iroh-tunnel-serve");

    fn target() -> &'static str {
        if cfg!(target_arch = "aarch64") {
            "windows-arm64"
        } else {
            "windows-amd64"
        }
    }

    /// Start the real serve binary from `exe` so the OS locks that file.
    fn run_from(exe: &Path) -> Result<std::process::Child> {
        let child = Command::new(exe)
            .args(["--target", "127.0.0.1:9", "--name", "locked-test"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        std::thread::sleep(std::time::Duration::from_secs(3));
        Ok(child)
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".iroh-tunnel-update-"))
            .collect()
    }

    /// Another process holds the target open as its running image.
    #[test]
    fn replacing_an_exe_that_another_process_runs_fails_and_keeps_it() -> Result<()> {
        let releases = tempfile::tempdir()?;
        publish_zip(
            releases.path(),
            "9.9.9",
            target(),
            &format!("{EXE}.exe"),
            NEW,
        )?;
        let server = ReleaseServer::start(releases.path().to_owned(), "9.9.9")?;
        let dir = tempfile::tempdir()?;
        let exe = dir.path().join(format!("{EXE}.exe"));
        std::fs::copy(SERVE, &exe)?;
        let before = std::fs::read(&exe)?;
        let mut running = run_from(&exe)?;
        assert!(running.try_wait()?.is_none(), "tunnel did not start");

        let result = run_update(&server, target(), &exe);

        let alive = running.try_wait()?.is_none();
        let _ = running.kill();
        let _ = running.wait();
        let err = result.expect_err("a locked executable cannot be replaced");
        eprintln!("locked replace error: {err:#}");
        assert!(format!("{err:#}").contains("cannot replace"), "{err:#}");
        assert!(format!("{err:#}").contains("stop it"), "{err:#}");
        assert!(
            format!("{err:#}").contains("service stop --system"),
            "{err:#}"
        );
        assert!(!format!("{err:#}").contains("sudo"), "{err:#}");
        assert!(alive, "the running process must not be stopped");
        assert_eq!(std::fs::read(&exe)?, before);
        assert_eq!(leftovers(dir.path()), Vec::<String>::new());
        Ok(())
    }

    /// `iroh-tunnel-serve update` run on itself: the updater is the locked file.
    #[test]
    fn an_executable_updating_itself_fails_cleanly_and_stays_usable() -> Result<()> {
        let releases = tempfile::tempdir()?;
        publish_zip(
            releases.path(),
            "9.9.9",
            target(),
            &format!("{EXE}.exe"),
            NEW,
        )?;
        let server = ReleaseServer::start(releases.path().to_owned(), "9.9.9")?;
        let dir = tempfile::tempdir()?;
        let exe = dir.path().join(format!("{EXE}.exe"));
        std::fs::copy(SERVE, &exe)?;
        let before = std::fs::read(&exe)?;

        let out = Command::new(&exe)
            .args(["update", "-i"])
            .env("IROH_TUNNEL_BASE_URL", &server.url)
            .output()?;

        let stderr = String::from_utf8_lossy(&out.stderr);
        eprintln!("self-update exit={:?} stderr={stderr}", out.status.code());
        assert!(!out.status.success(), "{stderr}");
        assert!(stderr.contains("cannot replace"), "{stderr}");
        assert!(
            stderr.contains("stop it") && !stderr.contains("sudo"),
            "{stderr}"
        );
        assert_eq!(std::fs::read(&exe)?, before);
        assert_eq!(leftovers(dir.path()), Vec::<String>::new());
        assert!(Command::new(&exe)
            .arg("--version")
            .output()?
            .status
            .success());
        Ok(())
    }
}
