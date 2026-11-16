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

/// Publish `version` for `target` as a `.zip` holding `entry` with `contents`.
fn publish_zip(
    root: &Path,
    version: &str,
    target: &str,
    entry: &str,
    contents: &[u8],
) -> Result<()> {
    common::publish_zips(root, version, target, &[(EXE, entry, contents)])
}

fn run_update(server: &ReleaseServer, target: &str, install: &Path) -> Result<()> {
    run_update_for(server, target, install, EXE)
}

fn run_update_for(
    server: &ReleaseServer,
    target: &str,
    install: &Path,
    exe_name: &str,
) -> Result<()> {
    let ctx = UpdateContext {
        base_url: server.url.clone(),
        exe_name,
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

    const CASES: [(&str, &str, Option<&str>, &[&str]); 3] = [
        (
            env!("CARGO_BIN_EXE_iroh-tunnel"),
            "iroh-tunnel",
            Some("serve"),
            &[
                "iroh-tunnel serve service stop --system",
                "iroh-tunnel access service stop --system",
            ],
        ),
        (
            env!("CARGO_BIN_EXE_iroh-tunnel-serve"),
            "iroh-tunnel-serve",
            None,
            &["iroh-tunnel-serve service stop --system"],
        ),
        (
            env!("CARGO_BIN_EXE_iroh-tunnel-access"),
            "iroh-tunnel-access",
            None,
            &["iroh-tunnel-access service stop --system"],
        ),
    ];

    struct Running(std::process::Child);

    impl Drop for Running {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn target() -> &'static str {
        if cfg!(target_arch = "aarch64") {
            "windows-arm64"
        } else {
            "windows-amd64"
        }
    }

    fn isolated_command(exe: &Path, home: &Path) -> Command {
        let mut command = Command::new(exe);
        command
            .env_remove("IROH_TUNNEL_CONTAINER")
            .env("HOME", home)
            .env("USERPROFILE", home)
            .env("APPDATA", home.join("config"))
            .env("LOCALAPPDATA", home.join("data"))
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("XDG_DATA_HOME", home.join("data"))
            .env("XDG_STATE_HOME", home.join("state"))
            .env("XDG_CACHE_HOME", home.join("cache"))
            .env("IROH_TUNNEL_TEST_CONFIG_DIR", home.join("config"))
            .env("IROH_TUNNEL_STATE_DIR", home.join("state"));
        command
    }

    /// Run a real role executable, with only a loopback relay, to lock its image.
    fn run_from(exe: &Path, prefix: Option<&str>, access: bool, home: &Path) -> Result<Running> {
        let mut command = isolated_command(exe, home);
        command.args(prefix);
        if access {
            let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
            let listen = listener.local_addr()?.to_string();
            drop(listener);
            let peer = iroh::SecretKey::generate().public().to_string();
            command.args(["--listen", &listen, "--node-id", &peer]);
        } else {
            command.args(["--target", "127.0.0.1:9"]);
        }
        let child = Running(
            command
                .args([
                    "--name",
                    "locked-test",
                    "--relay-url",
                    "https://127.0.0.1:9",
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?,
        );
        std::thread::sleep(std::time::Duration::from_secs(3));
        Ok(child)
    }

    fn assert_stop_guidance(message: &str, commands: &[&str]) {
        for command in commands {
            assert!(message.contains(command), "{message}");
        }
        for (_, _, _, other_commands) in CASES {
            for command in other_commands {
                if !commands.contains(command) {
                    assert!(!message.contains(command), "{message}");
                }
            }
        }
        assert!(message.contains("elevated"), "{message}");
        assert!(!message.contains("<role>"), "{message}");
        assert!(!message.contains("sudo"), "{message}");
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
        for (source, exe_name, prefix, commands) in CASES {
            let releases = tempfile::tempdir()?;
            let entry = format!("{exe_name}.exe");
            common::publish_zips(
                releases.path(),
                "9.9.9",
                target(),
                &[(exe_name, &entry, NEW)],
            )?;
            let server = ReleaseServer::start(releases.path().to_owned(), "9.9.9")?;
            let dir = tempfile::tempdir()?;
            let home = tempfile::tempdir()?;
            let exe = dir.path().join(&entry);
            std::fs::copy(source, &exe)?;
            let before = std::fs::read(&exe)?;
            let mut running =
                run_from(&exe, prefix, exe_name == "iroh-tunnel-access", home.path())?;
            assert!(running.0.try_wait()?.is_none(), "{exe_name} did not start");

            let result = run_update_for(&server, target(), &exe, exe_name);

            let alive = running.0.try_wait()?.is_none();
            let err = result.expect_err("a locked executable cannot be replaced");
            let message = format!("{err:#}");
            assert!(message.contains("cannot replace"), "{message}");
            assert!(message.contains("stop it"), "{message}");
            assert_stop_guidance(&message, commands);
            assert!(alive, "{exe_name}: the running process must not be stopped");
            assert_eq!(std::fs::read(&exe)?, before, "{exe_name}");
            assert_eq!(leftovers(dir.path()), Vec::<String>::new());
        }
        Ok(())
    }

    /// Each binary's `update` run on itself: the updater is the locked file.
    #[test]
    fn an_executable_updating_itself_fails_cleanly_and_stays_usable() -> Result<()> {
        for (source, exe_name, _, commands) in CASES {
            let releases = tempfile::tempdir()?;
            let entry = format!("{exe_name}.exe");
            common::publish_zips(
                releases.path(),
                "9.9.9",
                target(),
                &[(exe_name, &entry, NEW)],
            )?;
            let server = ReleaseServer::start(releases.path().to_owned(), "9.9.9")?;
            let dir = tempfile::tempdir()?;
            let home = tempfile::tempdir()?;
            let exe = dir.path().join(&entry);
            std::fs::copy(source, &exe)?;
            let before = std::fs::read(&exe)?;

            let out = isolated_command(&exe, home.path())
                .args(["update", "-i"])
                .env("IROH_TUNNEL_BASE_URL", &server.url)
                .output()?;

            let stderr = String::from_utf8_lossy(&out.stderr);
            assert_eq!(out.status.code(), Some(3), "{exe_name}: {stderr}");
            assert!(stderr.contains("cannot replace"), "{exe_name}: {stderr}");
            assert!(stderr.contains("stop it"), "{exe_name}: {stderr}");
            assert_stop_guidance(&stderr, commands);
            assert_eq!(std::fs::read(&exe)?, before, "{exe_name}");
            assert_eq!(leftovers(dir.path()), Vec::<String>::new());
            assert!(isolated_command(&exe, home.path())
                .arg("--version")
                .output()?
                .status
                .success());
        }
        Ok(())
    }
}
