//! #93: PowerShell installer (`install.ps1`) for the three role binaries.
//!
//! Every case runs the real script under `pwsh`, downloads real `.zip`
//! archives (made with `Compress-Archive`, as `build.yml` does) over real HTTP
//! from a local server standing in for GitHub Releases, and installs into a
//! temporary directory. The machine PATH is the Windows registry value on a
//! real Windows host; the machine-PATH cases use `IROH_TUNNEL_PATH_FILE` to
//! keep the test host's registry untouched, so the registry write itself is
//! covered only by the Windows CI job (see `registry_*`).
//!
//! Acceptance map:
//! - 93.AC1 / CL1 (INSTALL-DEFAULT-01): default selection is all three
//!   executables in one system-wide directory; installed files equal the
//!   archive members; no aliases on Windows (owner decision).
//! - 93.AC2 / CL1 (INSTALL-CONSENT-01): PATH change and file conflicts need
//!   confirmation; declining leaves the previous state unchanged.
//! - 93.CL2: the installer never registers or changes a service.
mod common;

use anyhow::{Context, Result};
use common::release_server::ReleaseServer;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/install.ps1");
const TARGET: &str = "windows-amd64";
const VERSION: &str = "9.9.9";
const EXES: [(&str, &str); 3] = [
    ("iroh-tunnel", env!("CARGO_BIN_EXE_iroh-tunnel")),
    ("iroh-tunnel-serve", env!("CARGO_BIN_EXE_iroh-tunnel-serve")),
    (
        "iroh-tunnel-access",
        env!("CARGO_BIN_EXE_iroh-tunnel-access"),
    ),
];

fn pwsh() -> Option<&'static str> {
    ["pwsh", "powershell"].into_iter().find(|name| {
        Command::new(name)
            .args(["-NoProfile", "-Command", "exit 0"])
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

/// A missing PowerShell skips locally but fails in CI, where every runner has it.
macro_rules! require_pwsh {
    () => {
        match pwsh() {
            Some(name) => name,
            None if std::env::var_os("CI").is_some() => panic!("PowerShell is required in CI"),
            None => {
                eprintln!("skipped: no PowerShell on this host");
                return Ok(());
            }
        }
    };
}

fn sha256(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    Ok(Sha256::digest(std::fs::read(path)?)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn text(out: &Output) -> String {
    format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn build_release(shell: &str, root: &Path, skip: Option<&str>) -> Result<()> {
    let dir = root.join(VERSION);
    std::fs::create_dir_all(&dir)?;
    let mut sums = String::new();
    for (exe, source) in EXES {
        if skip == Some(exe) {
            continue;
        }
        let staging = tempfile::tempdir()?;
        std::fs::copy(source, staging.path().join(format!("{exe}.exe")))?;
        std::fs::write(staging.path().join("README.md"), "readme")?;
        let archive = dir.join(format!("{exe}-{VERSION}-{TARGET}.zip"));
        let status = Command::new(shell)
            .args(["-NoProfile", "-Command"])
            .arg(format!(
                "Compress-Archive -Path '{}/*' -DestinationPath '{}'",
                staging.path().display(),
                archive.display()
            ))
            .status()?;
        assert!(status.success());
        sums.push_str(&format!(
            "{}  {}\n",
            sha256(&archive)?,
            archive.file_name().unwrap().to_string_lossy()
        ));
    }
    std::fs::write(dir.join("checksums.txt"), sums)?;
    Ok(())
}

struct Env {
    shell: &'static str,
    _root: tempfile::TempDir,
    release: PathBuf,
    bin: PathBuf,
    path_file: PathBuf,
    answers: PathBuf,
    server: ReleaseServer,
}

impl Env {
    fn new(shell: &'static str) -> Result<Self> {
        let root = tempfile::tempdir()?;
        let release = root.path().join("release");
        build_release(shell, &release, None)?;
        let server = ReleaseServer::start(release.clone(), VERSION)?;
        let path_file = root.path().join("machine-path.txt");
        std::fs::write(&path_file, "C:\\Windows\\system32")?;
        Ok(Self {
            shell,
            bin: root.path().join("Program Files").join("iroh-tunnel"),
            answers: root.path().join("answers"),
            path_file,
            release,
            server,
            _root: root,
        })
    }

    /// `answers` are the lines typed at the prompts; `None` means no terminal.
    /// `on_path` puts the install directory on the machine PATH.
    fn run(&self, args: &[&str], answers: Option<&str>, on_path: bool) -> Result<Output> {
        if on_path {
            std::fs::write(
                &self.path_file,
                format!("C:\\Windows\\system32;{}", self.bin.display()),
            )?;
        }
        let input = match answers {
            Some(text) => {
                std::fs::write(&self.answers, text)?;
                self.answers.clone()
            }
            None => PathBuf::from("/nonexistent/no-terminal"),
        };
        Ok(Command::new(self.shell)
            .args(["-NoProfile", "-NonInteractive", "-File", SCRIPT])
            .arg("-BinDir")
            .arg(&self.bin)
            .args(args)
            .env("IROH_TUNNEL_BASE_URL", &self.server.url)
            .env("IROH_TUNNEL_TARGET", TARGET)
            .env("IROH_TUNNEL_PROMPT_INPUT", input)
            .env("IROH_TUNNEL_PATH_FILE", &self.path_file)
            .stdin(Stdio::null())
            .output()?)
    }

    /// Puts the install directory on the machine PATH.
    fn put_on_path(&self) -> Result<()> {
        std::fs::write(
            &self.path_file,
            format!("C:\\Windows\\system32;{}", self.bin.display()),
        )?;
        Ok(())
    }

    fn machine_path(&self) -> Result<String> {
        Ok(std::fs::read_to_string(&self.path_file)?)
    }

    fn snapshot(&self) -> Result<BTreeMap<String, String>> {
        let mut map = BTreeMap::new();
        map.insert("machine-path".to_owned(), self.machine_path()?);
        if self.bin.exists() {
            for entry in std::fs::read_dir(&self.bin)? {
                let path = entry?.path();
                map.insert(path.display().to_string(), sha256(&path)?);
            }
        }
        Ok(map)
    }
}

fn assert_all_installed(env: &Env) -> Result<()> {
    for (exe, source) in EXES {
        let installed = env.bin.join(format!("{exe}.exe"));
        assert_eq!(
            sha256(&installed).with_context(|| installed.display().to_string())?,
            sha256(Path::new(source))?,
            "{exe} is not the archive member"
        );
    }
    let mut names: Vec<_> = std::fs::read_dir(&env.bin)?
        .map(|e| e.map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect::<std::io::Result<_>>()?;
    names.sort();
    assert_eq!(
        names,
        [
            "iroh-tunnel-access.exe",
            "iroh-tunnel-serve.exe",
            "iroh-tunnel.exe"
        ],
        "only the three executables, no aliases or staging files"
    );
    Ok(())
}

// 93.AC1 / INSTALL-DEFAULT-01
#[test]
fn default_flow_installs_all_three_executables() -> Result<()> {
    let shell = require_pwsh!();
    let env = Env::new(shell)?;
    let out = env.run(&[], Some("y\n"), false)?;
    assert!(out.status.success(), "{}", text(&out));
    assert_all_installed(&env)?;
    // On Windows the installed executables are the real ones: run them.
    if cfg!(windows) {
        for (exe, _) in EXES {
            let help = Command::new(env.bin.join(format!("{exe}.exe")))
                .arg("--help")
                .output()?;
            assert!(help.status.success(), "{exe}: {}", text(&help));
        }
    }
    Ok(())
}

// 93.AC1: the default directory is system-wide (Program Files), not per-user.
#[test]
fn dry_run_defaults_to_a_system_wide_directory_and_all_roles() -> Result<()> {
    let shell = require_pwsh!();
    let out = Command::new(shell)
        .args(["-NoProfile", "-NonInteractive", "-File", SCRIPT, "-DryRun"])
        .env("IROH_TUNNEL_TARGET", TARGET)
        .env("ProgramFiles", "C:\\Program Files")
        .stdin(Stdio::null())
        .output()?;
    assert!(out.status.success(), "{}", text(&out));
    let stdout = String::from_utf8(out.stdout)?;
    assert!(stdout.contains("Program Files"), "{stdout}");
    for (exe, _) in EXES {
        assert!(stdout.contains(exe), "{stdout}");
    }
    Ok(())
}

#[test]
fn explicit_version_resolves_and_a_missing_one_installs_nothing() -> Result<()> {
    let shell = require_pwsh!();
    let env = Env::new(shell)?;
    let out = env.run(&["-Version", &format!("v{VERSION}"), "-Yes"], None, true)?;
    assert!(out.status.success(), "{}", text(&out));
    assert_all_installed(&env)?;

    let env = Env::new(shell)?;
    let out = env.run(&["-Version", "1.2.3", "-Yes"], None, true)?;
    assert!(!out.status.success(), "{}", text(&out));
    assert!(!env.bin.exists());
    Ok(())
}

// 93.AC2: PATH change needs confirmation; decline leaves PATH unchanged.
#[test]
fn declining_the_path_change_installs_executables_but_leaves_path_alone() -> Result<()> {
    let shell = require_pwsh!();
    let env = Env::new(shell)?;
    let before = env.machine_path()?;
    let out = env.run(&[], Some("n\n"), false)?;
    assert!(out.status.success(), "{}", text(&out));
    assert_all_installed(&env)?;
    assert_eq!(before, env.machine_path()?);
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("PATH was not changed"),
        "{}",
        text(&out)
    );
    Ok(())
}

#[test]
fn accepting_the_path_change_appends_the_directory_once() -> Result<()> {
    let shell = require_pwsh!();
    let env = Env::new(shell)?;
    let out = env.run(&[], Some("y\n"), false)?;
    assert!(out.status.success(), "{}", text(&out));
    let path = env.machine_path()?;
    assert_eq!(
        path,
        format!("C:\\Windows\\system32;{}", env.bin.display()),
        "existing entries are kept, the directory is appended"
    );
    // A second run finds it on PATH and asks nothing.
    let again = env.run(&[], None, false)?;
    assert!(again.status.success(), "{}", text(&again));
    assert_eq!(path, env.machine_path()?);
    Ok(())
}

#[test]
fn no_prompt_is_shown_when_the_directory_is_already_on_path() -> Result<()> {
    let shell = require_pwsh!();
    let env = Env::new(shell)?;
    let out = env.run(&[], None, true)?;
    assert!(out.status.success(), "{}", text(&out));
    assert!(!text(&out).contains("[y/N]"), "{}", text(&out));
    assert_all_installed(&env)
}

#[test]
fn without_a_terminal_the_answer_is_no() -> Result<()> {
    let shell = require_pwsh!();
    let env = Env::new(shell)?;
    let before = env.machine_path()?;
    let out = env.run(&[], None, false)?;
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(before, env.machine_path()?);
    assert_all_installed(&env)
}

fn seed_conflict(env: &Env) -> Result<()> {
    std::fs::create_dir_all(&env.bin)?;
    std::fs::write(
        env.bin.join("iroh-tunnel-serve.exe"),
        "someone else's program",
    )?;
    Ok(())
}

#[test]
fn declining_a_file_conflict_stops_with_nothing_changed() -> Result<()> {
    let shell = require_pwsh!();
    let env = Env::new(shell)?;
    seed_conflict(&env)?;
    env.put_on_path()?;
    let before = env.snapshot()?;
    let out = env.run(&[], Some("n\n"), true)?;
    assert!(!out.status.success(), "{}", text(&out));
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("iroh-tunnel-serve.exe"),
        "{}",
        text(&out)
    );
    assert_eq!(before, env.snapshot()?, "decline must change nothing");
    Ok(())
}

#[test]
fn accepting_a_file_conflict_replaces_only_the_conflicting_file() -> Result<()> {
    let shell = require_pwsh!();
    let env = Env::new(shell)?;
    seed_conflict(&env)?;
    std::fs::write(env.bin.join("unrelated.txt"), "keep me")?;
    let out = env.run(&[], Some("y\n"), true)?;
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(
        sha256(&env.bin.join("iroh-tunnel-serve.exe"))?,
        sha256(Path::new(EXES[1].1))?
    );
    assert_eq!(
        std::fs::read_to_string(env.bin.join("unrelated.txt"))?,
        "keep me"
    );
    Ok(())
}

#[test]
fn reinstalling_the_same_release_asks_nothing() -> Result<()> {
    let shell = require_pwsh!();
    let env = Env::new(shell)?;
    let first = env.run(&[], None, true)?;
    assert!(first.status.success(), "{}", text(&first));
    let before = env.snapshot()?;
    let again = env.run(&[], None, true)?;
    assert!(again.status.success(), "{}", text(&again));
    assert!(!text(&again).contains("[y/N]"), "{}", text(&again));
    assert_eq!(before, env.snapshot()?);
    Ok(())
}

// Integrity: a bad download must never replace an installed executable.
#[test]
fn checksum_mismatch_installs_nothing_and_keeps_existing_files() -> Result<()> {
    let shell = require_pwsh!();
    let env = Env::new(shell)?;
    seed_conflict(&env)?;
    let archive = env
        .release
        .join(VERSION)
        .join(format!("iroh-tunnel-access-{VERSION}-{TARGET}.zip"));
    let mut bytes = std::fs::read(&archive)?;
    bytes.push(0);
    std::fs::write(&archive, bytes)?;
    env.put_on_path()?;
    let before = env.snapshot()?;
    let out = env.run(&["-Yes"], None, true)?;
    assert!(!out.status.success(), "{}", text(&out));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("checksum mismatch"),
        "{}",
        text(&out)
    );
    assert_eq!(before, env.snapshot()?);
    Ok(())
}

#[test]
fn a_release_missing_one_role_installs_nothing() -> Result<()> {
    let shell = require_pwsh!();
    let env = Env::new(shell)?;
    std::fs::remove_dir_all(env.release.join(VERSION))?;
    build_release(shell, &env.release, Some("iroh-tunnel-access"))?;
    let out = env.run(&["-Yes"], None, true)?;
    assert!(!out.status.success(), "{}", text(&out));
    assert!(!env.bin.exists());
    Ok(())
}

// 93.CL2: binary publication implies no service policy.
#[test]
fn installer_never_touches_services() -> Result<()> {
    let script = std::fs::read_to_string(SCRIPT)?;
    for forbidden in [
        "New-Service",
        "Set-Service",
        "Start-Service",
        "sc.exe",
        "schtasks",
        "Register-ScheduledTask",
    ] {
        assert!(
            !script
                .lines()
                .filter(|l| !l.trim_start().starts_with('#'))
                .any(|l| l.contains(forbidden)),
            "install.ps1 must not use {forbidden}"
        );
    }
    let shell = require_pwsh!();
    let env = Env::new(shell)?;
    let out = env.run(&["-Yes"], None, true)?;
    assert!(String::from_utf8_lossy(&out.stdout).contains("No service was registered"));
    Ok(())
}

#[test]
fn malformed_version_is_rejected() -> Result<()> {
    let shell = require_pwsh!();
    let env = Env::new(shell)?;
    for version in ["../etc", "1.2"] {
        let out = env.run(&["-Version", version], None, true)?;
        assert!(!out.status.success(), "{version}: {}", text(&out));
    }
    Ok(())
}

// The real machine PATH is the registry value. Only on a Windows host, and only
// when the runner is elevated (GitHub-hosted runners are).
#[test]
#[cfg(windows)]
fn registry_machine_path_is_changed_only_when_accepted() -> Result<()> {
    let shell = require_pwsh!();
    let env = Env::new(shell)?;
    let read = || -> Result<String> {
        let out = Command::new(shell)
            .args(["-NoProfile", "-Command"])
            .arg(
                "[Microsoft.Win32.Registry]::LocalMachine.OpenSubKey('SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment').GetValue('Path','',[Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)",
            )
            .output()?;
        Ok(String::from_utf8(out.stdout)?.trim().to_owned())
    };
    let original = read()?;
    let run = |answers: &str| -> Result<Output> {
        std::fs::write(&env.answers, answers)?;
        Ok(Command::new(shell)
            .args(["-NoProfile", "-NonInteractive", "-File", SCRIPT])
            .arg("-BinDir")
            .arg(&env.bin)
            .env("IROH_TUNNEL_BASE_URL", &env.server.url)
            .env("IROH_TUNNEL_TARGET", TARGET)
            .env("IROH_TUNNEL_PROMPT_INPUT", &env.answers)
            .stdin(Stdio::null())
            .output()?)
    };
    // Restore the runner's PATH whatever happens below.
    struct Restore<'a>(&'a str, String);
    impl Drop for Restore<'_> {
        fn drop(&mut self) {
            let script = format!(
                "$k=[Microsoft.Win32.Registry]::LocalMachine.OpenSubKey('SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment',$true);\
                 $k.SetValue('Path','{}',[Microsoft.Win32.RegistryValueKind]::ExpandString)",
                self.1.replace('\'', "''")
            );
            let _ = Command::new(self.0)
                .args(["-NoProfile", "-Command", &script])
                .status();
        }
    }
    let _restore = Restore(shell, original.clone());

    let declined = run("n\n")?;
    assert!(declined.status.success(), "{}", text(&declined));
    assert_eq!(original, read()?);

    let accepted = run("y\n")?;
    assert!(accepted.status.success(), "{}", text(&accepted));
    let changed = read()?;
    assert_eq!(
        changed,
        format!("{original};{}", env.bin.display()),
        "existing entries, including unexpanded variables, are kept"
    );
    Ok(())
}
