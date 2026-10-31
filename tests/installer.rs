//! #93: shell installer (`install.sh`) for the three role binaries.
//!
//! Every case runs the real script under `sh`, downloads real archives over
//! real HTTP from a local server that stands in for GitHub Releases
//! (`IROH_TUNNEL_BASE_URL`), and installs into a temporary directory. The
//! archives contain the cargo-built executables, so the installed files are
//! run, not just inspected. Prompts are answered through
//! `IROH_TUNNEL_PROMPT_INPUT`, the same read path the script uses for a
//! terminal.
//!
//! Limits: the release platform is forced with `IROH_TUNNEL_TARGET` (Linux
//! `linux-amd64`, macOS `darwin-arm64`); the archives hold executables built
//! for the test host. Platform detection itself and the default
//! `/etc/paths.d` are therefore not exercised. The PowerShell installer is in
//! `installer_ps.rs`.
//!
//! Acceptance map:
//! - 93.AC1 / CL1 (INSTALL-DEFAULT-01): default selection is all three
//!   binaries system-wide (`/usr/local/bin`); installed files equal the
//!   intended archive members; `it`/`its`/`ita` resolve to their roles.
//! - 93.AC2 / CL1 (INSTALL-CONSENT-01): PATH change and file conflicts need
//!   confirmation; declining leaves the previous state unchanged.
//! - macOS cases use `checksums-darwin.txt` and a `paths.d` entry.
//! - 93.CL2: the installer never registers or changes a service.
#![cfg(unix)]

mod common;

use anyhow::{Context, Result};
use common::release_server::ReleaseServer;
use std::collections::BTreeMap;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/install.sh");
const TARGET: &str = "linux-amd64";
const VERSION: &str = "9.9.9";
const ROLES: [(&str, &str, &str); 3] = [
    ("iroh-tunnel", "it", env!("CARGO_BIN_EXE_iroh-tunnel")),
    (
        "iroh-tunnel-serve",
        "its",
        env!("CARGO_BIN_EXE_iroh-tunnel-serve"),
    ),
    (
        "iroh-tunnel-access",
        "ita",
        env!("CARGO_BIN_EXE_iroh-tunnel-access"),
    ),
];

fn sha256(path: &Path) -> Result<String> {
    let out = Command::new("shasum")
        .args(["-a", "256"])
        .arg(path)
        .output()
        .or_else(|_| Command::new("sha256sum").arg(path).output())?;
    Ok(String::from_utf8(out.stdout)?
        .split_whitespace()
        .next()
        .context("no digest")?
        .to_owned())
}

/// A release directory with one archive per role and a checksum file.
fn build_release(root: &Path, skip_role: Option<&str>) -> Result<()> {
    build_release_for(root, skip_role, TARGET)
}

/// macOS checksums are published in their own file, like the release workflow.
fn checksum_file(target: &str) -> &'static str {
    if target.starts_with("darwin-") {
        "checksums-darwin.txt"
    } else {
        "checksums.txt"
    }
}

fn build_release_for(root: &Path, skip_role: Option<&str>, target: &str) -> Result<()> {
    let dir = root.join(VERSION);
    std::fs::create_dir_all(&dir)?;
    let mut sums = String::new();
    for (exe, _, source) in ROLES {
        if skip_role == Some(exe) {
            continue;
        }
        let staging = tempfile::tempdir()?;
        std::fs::copy(source, staging.path().join(exe))?;
        std::fs::write(staging.path().join("README.md"), "readme")?;
        let archive = dir.join(format!("{exe}-{VERSION}-{target}.tar.gz"));
        let status = Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(staging.path())
            .args([exe, "README.md"])
            .status()?;
        assert!(status.success());
        sums.push_str(&format!(
            "{}  {}\n",
            sha256(&archive)?,
            archive.file_name().unwrap().to_string_lossy()
        ));
    }
    std::fs::write(dir.join(checksum_file(target)), sums)?;
    Ok(())
}

struct Env {
    _root: tempfile::TempDir,
    release: PathBuf,
    bin: PathBuf,
    profile: PathBuf,
    home: PathBuf,
    answers: PathBuf,
    server: ReleaseServer,
    target: &'static str,
}

impl Env {
    fn new() -> Result<Self> {
        let root = tempfile::tempdir()?;
        let release = root.path().join("release");
        let profile = root.path().join("profile.d");
        let home = root.path().join("home");
        std::fs::create_dir_all(&profile)?;
        std::fs::create_dir_all(&home)?;
        build_release(&release, None)?;
        let server = ReleaseServer::start(release.clone(), VERSION)?;
        Ok(Self {
            bin: root.path().join("usr-local-bin"),
            answers: root.path().join("answers"),
            release,
            profile,
            home,
            server,
            target: TARGET,
            _root: root,
        })
    }

    /// Run the installer. `answers` are the lines typed at the prompts, in
    /// order; `None` means no terminal is available. `on_path` puts the install
    /// directory on PATH.
    fn run(&self, args: &[&str], answers: Option<&str>, on_path: bool) -> Result<Output> {
        let input = match answers {
            Some(text) => {
                std::fs::write(&self.answers, text)?;
                self.answers.clone()
            }
            None => PathBuf::from("/nonexistent/no-terminal"),
        };
        let path = if on_path {
            format!("{}:/usr/bin:/bin", self.bin.display())
        } else {
            "/usr/bin:/bin".to_owned()
        };
        Ok(Command::new("sh")
            .arg(SCRIPT)
            .arg("--bin-dir")
            .arg(&self.bin)
            .arg("--profile-dir")
            .arg(&self.profile)
            .args(args)
            .env_clear()
            .env("PATH", path)
            .env("HOME", &self.home)
            .env("IROH_TUNNEL_BASE_URL", &self.server.url)
            .env("IROH_TUNNEL_TARGET", self.target)
            .env("IROH_TUNNEL_PROMPT_INPUT", input)
            .stdin(Stdio::null())
            .output()?)
    }

    /// Everything the installer may have created, with digests.
    fn snapshot(&self) -> Result<BTreeMap<String, String>> {
        let mut map = BTreeMap::new();
        for dir in [&self.bin, &self.profile, &self.home] {
            if !dir.exists() {
                continue;
            }
            for entry in std::fs::read_dir(dir)? {
                let path = entry?.path();
                let value = if path.is_symlink() {
                    format!("-> {}", std::fs::read_link(&path)?.display())
                } else if path.is_file() {
                    sha256(&path)?
                } else {
                    "dir".to_owned()
                };
                map.insert(path.display().to_string(), value);
            }
        }
        Ok(map)
    }
}

fn text(out: &Output) -> String {
    format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn assert_all_installed(env: &Env) -> Result<()> {
    for (exe, alias, source) in ROLES {
        let installed = env.bin.join(exe);
        assert_eq!(
            sha256(&installed)?,
            sha256(Path::new(source))?,
            "{exe} is not the archive member"
        );
        let mode = std::fs::metadata(&installed)?.permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "{exe} is not executable");
        let link = env.bin.join(alias);
        assert!(link.is_symlink(), "{alias} is not a symlink");
        assert_eq!(std::fs::read_link(&link)?, Path::new(exe));
    }
    Ok(())
}

fn help(path: &Path) -> Result<String> {
    let out = Command::new(path).arg("--help").output()?;
    assert!(out.status.success(), "{}: {}", path.display(), text(&out));
    Ok(String::from_utf8(out.stdout)?)
}

// 93.AC1 / INSTALL-DEFAULT-01
#[test]
fn default_flow_installs_all_three_binaries_and_working_aliases() -> Result<()> {
    let env = Env::new()?;
    let out = env.run(&[], Some("y\n"), false)?;
    assert!(out.status.success(), "{}", text(&out));
    assert_all_installed(&env)?;
    // Aliases resolve to, and run, their intended roles.
    for (exe, alias, _) in ROLES {
        assert_eq!(
            help(&env.bin.join(alias))?.replace(alias, exe),
            help(&env.bin.join(exe))?.replace(alias, exe),
            "{alias} runs a different role than {exe}"
        );
    }
    let serve = help(&env.bin.join("its"))?;
    let access = help(&env.bin.join("ita"))?;
    assert_ne!(serve, access, "its and ita must be different roles");
    Ok(())
}

// 93.AC1: the default directory is system-wide, not per-user.
#[test]
fn dry_run_defaults_to_system_wide_directory_and_all_roles() -> Result<()> {
    let out = Command::new("sh")
        .args([SCRIPT, "--dry-run"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("IROH_TUNNEL_TARGET", TARGET)
        .output()?;
    assert!(out.status.success(), "{}", text(&out));
    let stdout = String::from_utf8(out.stdout)?;
    assert!(stdout.contains("/usr/local/bin"), "{stdout}");
    for (exe, alias, _) in ROLES {
        assert!(stdout.contains(exe) && stdout.contains(alias), "{stdout}");
    }
    Ok(())
}

// 93.AC1: latest stable by default, explicit version when asked.
#[test]
fn latest_and_explicit_version_both_resolve() -> Result<()> {
    let env = Env::new()?;
    let out = env.run(&["--version", &format!("v{VERSION}"), "--yes"], None, true)?;
    assert!(out.status.success(), "{}", text(&out));
    assert_all_installed(&env)?;

    let env = Env::new()?;
    let out = env.run(&["--version", "1.2.3", "--yes"], None, true)?;
    assert!(!out.status.success(), "{}", text(&out));
    assert!(!env.bin.exists(), "a missing version must install nothing");
    Ok(())
}

// 93.AC2: PATH change needs confirmation; decline keeps PATH files unchanged.
#[test]
fn declining_the_path_change_installs_binaries_but_leaves_path_alone() -> Result<()> {
    let env = Env::new()?;
    let before = env.snapshot()?;
    let out = env.run(&[], Some("n\n"), false)?;
    assert!(out.status.success(), "{}", text(&out));
    assert_all_installed(&env)?;
    assert!(!env.profile.join("iroh-tunnel.sh").exists());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("PATH was not changed"), "{}", text(&out));
    let after = env.snapshot()?;
    let added: Vec<_> = after.keys().filter(|k| !before.contains_key(*k)).collect();
    assert!(
        added
            .iter()
            .all(|k| k.starts_with(&env.bin.display().to_string())),
        "installer touched files outside the install directory: {added:?}"
    );
    Ok(())
}

#[test]
fn accepting_the_path_change_writes_the_profile_snippet() -> Result<()> {
    let env = Env::new()?;
    let out = env.run(&[], Some("y\n"), false)?;
    assert!(out.status.success(), "{}", text(&out));
    let snippet = std::fs::read_to_string(env.profile.join("iroh-tunnel.sh"))?;
    assert!(
        snippet.contains(&env.bin.display().to_string()),
        "{snippet}"
    );
    Ok(())
}

#[test]
fn no_prompt_is_shown_when_the_directory_is_already_on_path() -> Result<()> {
    let env = Env::new()?;
    let out = env.run(&[], None, true)?;
    assert!(out.status.success(), "{}", text(&out));
    assert!(!env.profile.join("iroh-tunnel.sh").exists());
    assert_all_installed(&env)
}

#[test]
fn without_a_terminal_the_answer_is_no() -> Result<()> {
    let env = Env::new()?;
    let out = env.run(&[], None, false)?;
    assert!(out.status.success(), "{}", text(&out));
    assert!(!env.profile.join("iroh-tunnel.sh").exists());
    assert_all_installed(&env)
}

// 93.AC2: conflicting file replacement needs confirmation.
fn seed_conflicts(env: &Env) -> Result<()> {
    std::fs::create_dir_all(&env.bin)?;
    std::fs::write(env.bin.join("iroh-tunnel-serve"), "someone else's program")?;
    symlink("/bin/ls", env.bin.join("ita"))?;
    Ok(())
}

#[test]
fn declining_a_file_conflict_stops_with_nothing_changed() -> Result<()> {
    let env = Env::new()?;
    seed_conflicts(&env)?;
    let before = env.snapshot()?;
    let out = env.run(&[], Some("n\n"), true)?;
    assert!(!out.status.success(), "{}", text(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("iroh-tunnel-serve"), "{}", text(&out));
    assert!(stdout.contains("/ita"), "{}", text(&out));
    assert_eq!(
        before,
        env.snapshot()?,
        "decline must leave state unchanged"
    );
    Ok(())
}

#[test]
fn accepting_a_file_conflict_replaces_only_the_conflicting_files() -> Result<()> {
    let env = Env::new()?;
    seed_conflicts(&env)?;
    std::fs::write(env.bin.join("unrelated"), "keep me")?;
    let out = env.run(&[], Some("y\n"), true)?;
    assert!(out.status.success(), "{}", text(&out));
    assert_all_installed(&env)?;
    assert_eq!(
        std::fs::read_to_string(env.bin.join("unrelated"))?,
        "keep me"
    );
    Ok(())
}

#[test]
fn reinstalling_the_same_release_asks_nothing() -> Result<()> {
    let env = Env::new()?;
    let first = env.run(&[], None, true)?;
    assert!(first.status.success(), "{}", text(&first));
    let before = env.snapshot()?;
    let again = env.run(&[], None, true)?;
    assert!(again.status.success(), "{}", text(&again));
    assert!(!String::from_utf8_lossy(&again.stderr).contains("[y/N]"));
    assert_eq!(before, env.snapshot()?);
    Ok(())
}

#[test]
fn yes_accepts_conflicts_without_a_terminal() -> Result<()> {
    let env = Env::new()?;
    seed_conflicts(&env)?;
    let out = env.run(&["--yes"], None, true)?;
    assert!(out.status.success(), "{}", text(&out));
    assert_all_installed(&env)
}

// Integrity: a bad download must never replace an installed binary.
#[test]
fn checksum_mismatch_installs_nothing_and_keeps_existing_files() -> Result<()> {
    let env = Env::new()?;
    seed_conflicts(&env)?;
    let archive = env
        .release
        .join(VERSION)
        .join(format!("iroh-tunnel-access-{VERSION}-{TARGET}.tar.gz"));
    std::fs::write(&archive, "tampered")?;
    let before = env.snapshot()?;
    let out = env.run(&["--yes"], None, true)?;
    assert!(!out.status.success(), "{}", text(&out));
    assert!(String::from_utf8_lossy(&out.stderr).contains("checksum mismatch"));
    assert_eq!(before, env.snapshot()?);
    Ok(())
}

#[test]
fn a_release_missing_one_role_installs_nothing() -> Result<()> {
    let env = Env::new()?;
    std::fs::remove_dir_all(env.release.join(VERSION))?;
    build_release(&env.release, Some("iroh-tunnel-serve"))?;
    let out = env.run(&["--yes"], None, true)?;
    assert!(!out.status.success(), "{}", text(&out));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("iroh-tunnel-serve"),
        "{}",
        text(&out)
    );
    assert!(!env.bin.exists(), "no partial install");
    Ok(())
}

#[test]
fn unwritable_directory_fails_before_downloading() -> Result<()> {
    let env = Env::new()?;
    std::fs::create_dir_all(&env.bin)?;
    std::fs::set_permissions(&env.bin, std::fs::Permissions::from_mode(0o555))?;
    // root ignores directory permissions: skip when the directory stays writable.
    if std::fs::write(env.bin.join(".probe"), "x").is_ok() {
        return Ok(());
    }
    let out = env.run(&["--yes"], None, true);
    std::fs::set_permissions(&env.bin, std::fs::Permissions::from_mode(0o755))?;
    let out = out?;
    assert!(!out.status.success(), "{}", text(&out));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("sudo"), "guidance missing: {stderr}");
    assert!(
        std::fs::read_dir(&env.bin)?.next().is_none(),
        "files were written to an unwritable directory"
    );
    Ok(())
}

// 93.CL2: binary publication implies no service policy.
#[test]
fn installer_never_touches_services() -> Result<()> {
    let script = std::fs::read_to_string(SCRIPT)?;
    for forbidden in [
        "systemctl",
        "launchctl",
        "rc-service",
        "rc-update",
        "/etc/systemd",
    ] {
        assert!(
            !script
                .lines()
                .filter(|l| !l.trim_start().starts_with('#'))
                .any(|l| l.contains(forbidden)),
            "install.sh must not use {forbidden}"
        );
    }
    let env = Env::new()?;
    let out = env.run(&["--yes"], None, true)?;
    assert!(String::from_utf8_lossy(&out.stdout).contains("No service was registered"));
    Ok(())
}

// An alias that is a symlink to a directory must be replaced, not entered.
#[test]
fn alias_that_links_to_a_directory_is_replaced() -> Result<()> {
    let env = Env::new()?;
    std::fs::create_dir_all(env.bin.join("elsewhere"))?;
    symlink("elsewhere", env.bin.join("ita"))?;
    let out = env.run(&["--yes"], None, true)?;
    assert!(out.status.success(), "{}", text(&out));
    assert_all_installed(&env)?;
    assert!(
        std::fs::read_dir(env.bin.join("elsewhere"))?
            .next()
            .is_none(),
        "the new alias was moved into the linked directory"
    );
    Ok(())
}

#[test]
fn malformed_version_and_relative_directory_are_rejected() -> Result<()> {
    let env = Env::new()?;
    for args in [
        vec!["--version", "../etc"],
        vec!["--version", "1.2"],
        vec!["--bin-dir", "relative/dir"],
        vec!["--profile-dir", "relative/dir"],
        vec!["--profile-dir", "/tmp/with space"],
    ] {
        let out = env.run(&args, None, true)?;
        assert!(!out.status.success(), "{args:?}: {}", text(&out));
    }
    Ok(())
}

// A failed install leaves no staging files behind.
#[test]
fn no_staging_files_remain_after_success() -> Result<()> {
    let env = Env::new()?;
    let out = env.run(&["--yes"], None, true)?;
    assert!(out.status.success(), "{}", text(&out));
    let names: Vec<_> = std::fs::read_dir(&env.bin)?
        .map(|e| e.map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect::<std::io::Result<_>>()?;
    assert!(names.iter().all(|n| !n.contains(".new.")), "{names:?}");
    Ok(())
}

// --- macOS (arm64): checksums-darwin.txt, /etc/paths.d ---------------------

const DARWIN: &str = "darwin-arm64";

impl Env {
    /// An environment whose release holds only macOS archives, listed in
    /// `checksums-darwin.txt` as the release workflow publishes them.
    fn darwin() -> Result<Self> {
        let mut env = Self::new()?;
        std::fs::remove_dir_all(env.release.join(VERSION))?;
        build_release_for(&env.release, None, DARWIN)?;
        env.target = DARWIN;
        Ok(env)
    }
}

// 93.AC1 / CL1 on macOS: all three binaries and aliases from the darwin
// archives, verified against checksums-darwin.txt.
#[test]
fn macos_default_flow_installs_all_three_binaries_and_working_aliases() -> Result<()> {
    let env = Env::darwin()?;
    let out = env.run(&[], Some("y\n"), false)?;
    assert!(out.status.success(), "{}", text(&out));
    assert_all_installed(&env)?;
    assert_ne!(help(&env.bin.join("its"))?, help(&env.bin.join("ita"))?);
    Ok(())
}

#[test]
fn macos_latest_version_is_read_from_the_darwin_checksum_file() -> Result<()> {
    let env = Env::darwin()?;
    let out = env.run(&["--yes"], None, true)?;
    assert!(out.status.success(), "{}", text(&out));
    assert!(
        String::from_utf8_lossy(&out.stdout).contains(&format!("iroh-tunnel {VERSION} installed")),
        "{}",
        text(&out)
    );
    Ok(())
}

// 93.AC2 on macOS: PATH is changed through /etc/paths.d (one directory per
// line), after confirmation.
#[test]
fn macos_path_change_writes_a_paths_d_entry_only_when_accepted() -> Result<()> {
    let env = Env::darwin()?;
    let out = env.run(&[], Some("n\n"), false)?;
    assert!(out.status.success(), "{}", text(&out));
    assert!(!env.profile.join("iroh-tunnel").exists());

    let out = env.run(&[], Some("y\n"), false)?;
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(
        std::fs::read_to_string(env.profile.join("iroh-tunnel"))?,
        format!("{}\n", env.bin.display())
    );
    assert!(!env.profile.join("iroh-tunnel.sh").exists());
    Ok(())
}

#[test]
fn macos_declining_a_file_conflict_stops_with_nothing_changed() -> Result<()> {
    let env = Env::darwin()?;
    seed_conflicts(&env)?;
    let before = env.snapshot()?;
    let out = env.run(&[], Some("n\n"), true)?;
    assert!(!out.status.success(), "{}", text(&out));
    assert_eq!(before, env.snapshot()?);
    Ok(())
}
