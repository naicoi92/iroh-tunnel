//! #90: install and manage a user service, driven through the real binary.
//!
//! Acceptance map (cases from `docs/design/redesign-acceptance.md`):
//! - SM-SCOPE-01 (90.CL1): the non-elevated interactive install offers
//!   user/system; `system` without privilege stops with rerun guidance that
//!   keeps the intended arguments and absolute paths, never elevates, never
//!   falls back to a user service and registers nothing.
//! - SM-PERMISSION-01 (90.AC2): a denied registration fails with guidance and
//!   leaves scope and account unchanged.
//! - SM-USER-01 / SM-PATH-01 (90.AC1, 90.CL2): the real user-service manager
//!   (launchd on macOS, `systemctl --user` on Linux) installs, starts, reports,
//!   stops, restarts and uninstalls a service that runs with the pinned
//!   absolute config and state paths from a different working directory.
//!
//! The lifecycle test drives the host's real service manager, so it is opt-in:
//! set `IROH_TUNNEL_E2E_SERVICE=1`. It registers under a unique test suffix
//! (the `test-utils` seam `IROH_TUNNEL_TEST_SERVICE_SUFFIX`) and never touches
//! a real `iroh-tunnel` service, and it removes its registration on exit.
//! The `test-utils` feature disables relay TLS verification: only the isolated
//! in-process relay is used, never public relays or real secrets.

#![cfg(unix)]

use anyhow::Result;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_iroh-tunnel");

fn is_root() -> bool {
    // SAFETY: geteuid(2) has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// An isolated user: `HOME`, a config file and a state directory.
struct Fixture {
    root: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Result<Self> {
        let root = tempfile::tempdir()?;
        // Canonical so macOS /var -> /private/var does not break comparisons.
        let canonical = root.path().canonicalize()?;
        let fixture = Self {
            root: tempfile::TempDir::new_in(canonical.parent().unwrap())?,
        };
        drop(root);
        std::fs::create_dir_all(fixture.home())?;
        Ok(fixture)
    }
    fn base(&self) -> PathBuf {
        self.root.path().canonicalize().expect("canonical root")
    }
    fn home(&self) -> PathBuf {
        self.root.path().join("home")
    }
    fn config(&self) -> PathBuf {
        self.base().join("cfg dir").join("serve.toml")
    }
    fn state(&self) -> PathBuf {
        self.base().join("state dir")
    }
    fn write_config(&self, relay: &str) -> Result<()> {
        let key = iroh::SecretKey::generate();
        std::fs::create_dir_all(self.config().parent().unwrap())?;
        std::fs::write(
            self.config(),
            format!(
                "[node]\nsecret_key = {:?}\nrelay_urls = [{relay:?}]\n\n\
                 [[services]]\nname = \"echo\"\nprotocol = \"tcp\"\nport = 9\n",
                iroh_tunnel::config::encode_secret_key(&key)
            ),
        )?;
        Ok(())
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(BIN);
        command
            .args(args)
            .env("HOME", self.home())
            .env_remove("IROH_TUNNEL_TEST_CONFIG_DIR")
            .env("IROH_TUNNEL_STATE_DIR", self.state())
            .env_remove("IROH_TUNNEL_TEST_INTERACTIVE")
            // Keep any `iroh-tunnel` on the developer's PATH out of the unit.
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin");
        command
    }
    fn run(&self, args: &[&str]) -> Result<Output> {
        Ok(self.command(args).output()?)
    }
    fn run_with_stdin(&self, args: &[&str], stdin: &str) -> Result<Output> {
        let mut child = self
            .command(args)
            .env("IROH_TUNNEL_TEST_INTERACTIVE", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child.stdin.take().unwrap().write_all(stdin.as_bytes())?;
        Ok(child.wait_with_output()?)
    }
    /// Everything a service manager could have been told, under `HOME`.
    fn registrations(&self) -> Vec<PathBuf> {
        fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        walk(&path, out);
                    } else {
                        out.push(path);
                    }
                }
            }
        }
        let mut found = Vec::new();
        walk(&self.home(), &mut found);
        found
    }
}

#[test]
fn system_install_without_privilege_stops_with_rerun_guidance() -> Result<()> {
    if is_root() {
        return Ok(());
    }
    let fx = Fixture::new()?;
    fx.write_config("https://relay.invalid")?;
    let config = fx.config();

    let out = fx.run(&[
        "serve",
        "service",
        "install",
        "--system",
        "--config",
        config.to_str().unwrap(),
    ])?;
    let err = stderr(&out);

    assert_eq!(out.status.code(), Some(3), "stderr: {err}");
    assert!(err.contains("sudo"), "no elevation guidance: {err}");
    assert!(
        err.contains("serve service install --system"),
        "intended arguments not preserved: {err}"
    );
    assert!(
        err.contains(config.to_str().unwrap()),
        "absolute config path not preserved: {err}"
    );
    assert!(
        fx.registrations().is_empty(),
        "nothing may be registered or silently fall back to a user service: {:?}",
        fx.registrations()
    );
    Ok(())
}

#[test]
fn system_scope_operations_without_privilege_are_refused_with_guidance() -> Result<()> {
    if is_root() {
        return Ok(());
    }
    let fx = Fixture::new()?;
    for verb in ["uninstall", "start", "stop", "restart"] {
        let out = fx.run(&["serve", "service", verb, "--system"])?;
        let err = stderr(&out);
        assert_eq!(out.status.code(), Some(3), "{verb}: {err}");
        assert!(
            err.contains(&format!("sudo iroh-tunnel serve service {verb} --system")),
            "{verb}: {err}"
        );
    }
    Ok(())
}

#[test]
fn interactive_install_offers_user_or_system_and_system_is_refused_unprivileged() -> Result<()> {
    if is_root() {
        return Ok(());
    }
    let fx = Fixture::new()?;
    fx.write_config("https://relay.invalid")?;
    let config = fx.config();
    let config = config.to_str().unwrap();

    let out = fx.run_with_stdin(
        &["serve", "service", "install", "--config", config],
        "system\n",
    )?;
    let err = stderr(&out);
    assert!(
        err.contains("user") && err.contains("system"),
        "no choice offered: {err}"
    );
    assert_eq!(out.status.code(), Some(3), "stderr: {err}");
    assert!(err.contains("sudo") && err.contains(config), "{err}");
    assert!(fx.registrations().is_empty(), "{:?}", fx.registrations());

    // An unrecognised answer is an error, never a guess.
    let out = fx.run_with_stdin(
        &["serve", "service", "install", "--config", config],
        "maybe\n",
    )?;
    assert_eq!(out.status.code(), Some(2), "stderr: {}", stderr(&out));
    assert!(fx.registrations().is_empty());
    Ok(())
}

#[test]
fn install_without_config_fails_with_guidance_and_registers_nothing() -> Result<()> {
    let fx = Fixture::new()?;
    let config = fx.config();
    let out = fx.run(&[
        "serve",
        "service",
        "install",
        "--user",
        "--config",
        config.to_str().unwrap(),
    ])?;
    let err = stderr(&out);
    assert_eq!(out.status.code(), Some(2), "stderr: {err}");
    assert!(err.contains(config.to_str().unwrap()), "{err}");
    assert!(err.contains("config"), "{err}");
    assert!(fx.registrations().is_empty(), "{:?}", fx.registrations());
    Ok(())
}

#[test]
fn denied_registration_fails_with_guidance_and_keeps_scope() -> Result<()> {
    if is_root() {
        return Ok(());
    }
    let fx = Fixture::new()?;
    fx.write_config("https://relay.invalid")?;
    std::fs::set_permissions(fx.home(), std::fs::Permissions::from_mode(0o500))?;
    let config = fx.config();

    let out = fx.run(&[
        "serve",
        "service",
        "install",
        "--user",
        "--config",
        config.to_str().unwrap(),
    ]);
    std::fs::set_permissions(fx.home(), std::fs::Permissions::from_mode(0o700))?;
    let out = out?;
    let err = stderr(&out);

    assert_eq!(out.status.code(), Some(3), "stderr: {err}");
    assert!(err.contains("permission"), "no permission guidance: {err}");
    assert!(
        err.contains("--system") && err.contains("not"),
        "guidance must say scope is not changed automatically: {err}"
    );
    assert!(fx.registrations().is_empty(), "{:?}", fx.registrations());
    Ok(())
}

// ---------------------------------------------------------------------------
// Real lifecycle through the host's user-service manager.
// ---------------------------------------------------------------------------

struct Registered<'a> {
    fx: &'a Fixture,
    suffix: String,
}

impl Registered<'_> {
    fn cmd(&self, args: &[&str]) -> Command {
        let mut command = self.fx.command(args);
        command
            .env("HOME", real_home())
            .env("IROH_TUNNEL_TEST_SERVICE_SUFFIX", &self.suffix)
            // A different working directory than the pinned paths assume.
            .current_dir("/");
        command
    }
    fn registration(&self) -> PathBuf {
        let home = real_home();
        if cfg!(target_os = "macos") {
            home.join(format!(
                "Library/LaunchAgents/dev.iroh-tunnel.serve{}.plist",
                self.suffix
            ))
        } else {
            home.join(format!(
                ".config/systemd/user/iroh-tunnel-serve{}.service",
                self.suffix
            ))
        }
    }
}

impl Drop for Registered<'_> {
    fn drop(&mut self) {
        let _ = self
            .cmd(&["serve", "service", "uninstall", "--user"])
            .output();
    }
}

fn real_home() -> PathBuf {
    dirs::home_dir().expect("home directory")
}

fn wait_for(what: &str, mut ready: impl FnMut() -> bool) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if ready() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    anyhow::bail!("timeout waiting for {what}")
}

fn node_id_of(status_file: &Path) -> Option<String> {
    let bytes = std::fs::read(status_file).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    json.get("node_id")?.as_str().map(str::to_owned)
}

#[tokio::test]
async fn user_service_lifecycle_uses_the_pinned_absolute_paths() -> Result<()> {
    if std::env::var_os("IROH_TUNNEL_E2E_SERVICE").is_none() || is_root() {
        eprintln!("skipped: set IROH_TUNNEL_E2E_SERVICE=1 as a non-root user to drive the real service manager");
        return Ok(());
    }
    let (_map, relay, _server) = iroh::test_utils::run_relay_server().await?;
    let fx = Fixture::new()?;
    fx.write_config(relay.as_ref())?;
    let svc = Registered {
        fx: &fx,
        suffix: format!("-e2e{}", std::process::id()),
    };
    let config = fx.config();
    let status_file = fx.state().join("serve-status.json");
    let ok = |args: &[&str]| -> Result<String> {
        let out = svc.cmd(args).output()?;
        anyhow::ensure!(
            out.status.success(),
            "{args:?} failed: {}{}",
            String::from_utf8_lossy(&out.stdout),
            stderr(&out)
        );
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    };

    // install (relative config path, odd characters: it must be pinned absolute).
    ok(&[
        "serve",
        "service",
        "install",
        "--user",
        "--config",
        config.to_str().unwrap(),
    ])?;
    let registration = std::fs::read_to_string(svc.registration())?;
    assert!(
        registration.contains(config.to_str().unwrap()),
        "{registration}"
    );
    assert!(
        registration.contains(fx.state().to_str().unwrap()),
        "state dir not pinned: {registration}"
    );

    // The service really runs from the pinned paths and writes status there.
    wait_for("status file from the installed service", || {
        node_id_of(&status_file).is_some()
    })?;
    let first_id = node_id_of(&status_file).unwrap();

    let status = ok(&["serve", "service", "status", "--user"])?;
    assert!(
        status.to_lowercase().contains("running") || status.contains("state = running"),
        "{status}"
    );

    // stop: the process exits and the owned status file is cleaned up.
    ok(&["serve", "service", "stop", "--user"])?;
    wait_for("stopped service to remove its status file", || {
        !status_file.exists()
    })?;

    // start again and restart: identity is the configured one throughout.
    ok(&["serve", "service", "start", "--user"])?;
    wait_for("restarted status file", || {
        node_id_of(&status_file).is_some()
    })?;
    assert_eq!(node_id_of(&status_file).unwrap(), first_id);
    ok(&["serve", "service", "restart", "--user"])?;
    wait_for("status after restart", || {
        node_id_of(&status_file).is_some()
    })?;
    assert_eq!(node_id_of(&status_file).unwrap(), first_id);

    // uninstall removes the registration and the process.
    ok(&["serve", "service", "uninstall", "--user"])?;
    assert!(!svc.registration().exists());
    wait_for("uninstalled service to stop", || !status_file.exists())?;
    Ok(())
}
