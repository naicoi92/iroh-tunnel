//! Homebrew renderer and stable tap publication contracts (Task 3).
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");
const VERSION: &str = "1.2.3";

struct Role {
    selector: &'static str,
    name: &'static str,
    alias: &'static str,
    sha: &'static str,
    url: &'static str,
}

// Literal expectations from the approved renderer/artifact contracts.
const ROLES: [Role; 3] = [
    Role {
        selector: "combined",
        name: "iroh-tunnel",
        alias: "it",
        sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        url: "https://github.com/naicoi92/iroh-tunnel/releases/download/v#{version}/iroh-tunnel-#{version}-darwin-arm64.tar.gz",
    },
    Role {
        selector: "serve",
        name: "iroh-tunnel-serve",
        alias: "its",
        sha: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        url: "https://github.com/naicoi92/iroh-tunnel/releases/download/v#{version}/iroh-tunnel-serve-#{version}-darwin-arm64.tar.gz",
    },
    Role {
        selector: "access",
        name: "iroh-tunnel-access",
        alias: "ita",
        sha: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        url: "https://github.com/naicoi92/iroh-tunnel/releases/download/v#{version}/iroh-tunnel-access-#{version}-darwin-arm64.tar.gz",
    },
];

fn isolated<'a>(command: &'a mut Command, root: &Path) -> &'a mut Command {
    for (key, dir) in [
        ("HOME", "home"),
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_DATA_HOME", "data"),
        ("XDG_CACHE_HOME", "cache"),
        ("XDG_STATE_HOME", "state"),
        ("XDG_RUNTIME_DIR", "runtime"),
    ] {
        let path = root.join(dir);
        fs::create_dir_all(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        command.env(key, path);
    }
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
}

fn successful(output: Output) -> String {
    assert!(
        output.status.success(),
        "process failed: {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .replace("\r\n", "\n")
}

fn render(args: &[&str]) -> Output {
    let temp = TempDir::new().unwrap();
    isolated(&mut Command::new("bash"), temp.path())
        .arg(Path::new(ROOT).join("packaging/homebrew-cask.sh"))
        .args(args)
        .output()
        .expect("run cask renderer")
}

fn quoted_values(line: &str) -> Vec<&str> {
    line.split('"').skip(1).step_by(2).collect()
}

fn field<'a>(cask: &'a str, key: &str) -> &'a str {
    let values: Vec<_> = cask
        .lines()
        .filter_map(|line| line.trim().strip_prefix(key))
        .map(|line| quoted_values(line)[0])
        .collect();
    assert_eq!(values.len(), 1, "expected one {key} field in {cask}");
    values[0]
}

fn assert_cask(cask: &str, role: &Role) {
    assert_eq!(field(cask, "cask "), role.name);
    assert_eq!(field(cask, "name "), role.name);
    assert_eq!(field(cask, "version "), VERSION);
    assert_eq!(field(cask, "sha256 "), role.sha);
    assert_eq!(field(cask, "url "), role.url);
    assert_eq!(field(cask, "binary "), role.name);
    assert!(cask.lines().any(|l| l.trim() == "depends_on arch: :arm64"));
    let commands = cask_commands(cask);
    assert_eq!(commands.len(), 3, "unexpected side effects in {cask}");
    assert_eq!(commands[0].0, "/usr/bin/xattr");
    assert_eq!(
        commands[0].1,
        [
            "-dr",
            "com.apple.quarantine",
            &format!("#{{staged_path}}/{}", role.name)
        ]
    );
    assert_eq!(commands[1].0, "/bin/ln");
    assert_eq!(
        commands[1].1,
        [
            "-sf",
            role.name,
            &format!("#{{HOMEBREW_PREFIX}}/bin/{}", role.alias)
        ]
    );
    assert_eq!(commands[2].0, "/bin/rm");
    assert_eq!(
        commands[2].1,
        ["-f", &format!("#{{HOMEBREW_PREFIX}}/bin/{}", role.alias)]
    );
}

#[test]
fn renderer_selects_role_specific_assets() {
    for role in &ROLES {
        let cask = successful(render(&[VERSION, role.sha, role.selector]));
        assert_cask(&cask, role);
    }
}

#[test]
fn omitted_role_preserves_combined_output() {
    let default = successful(render(&[VERSION, ROLES[0].sha]));
    assert_cask(&default, &ROLES[0]);
    assert_eq!(
        default,
        successful(render(&[VERSION, ROLES[0].sha, "combined"]))
    );
}

fn assert_rejected(args: &[&str]) {
    let output = render(args);
    assert!(!output.status.success(), "accepted {args:?}");
    assert!(output.stdout.is_empty(), "partial cask for {args:?}");
    assert!(!output.stderr.is_empty(), "no diagnostic for {args:?}");
}

#[test]
fn renderer_rejects_missing_inputs() {
    for args in [
        vec![],
        vec![VERSION],
        vec!["", ROLES[0].sha],
        vec![VERSION, ""],
    ] {
        assert_rejected(&args);
    }
}

#[test]
fn renderer_rejects_invalid_roles() {
    for role in ["", "both", "Serve", "ita", "serve/access", "serve\naccess"] {
        assert_rejected(&[VERSION, ROLES[0].sha, role]);
    }
    assert_rejected(&[VERSION, ROLES[0].sha, "serve", "extra"]);
}

#[test]
fn renderer_rejects_malformed_release_inputs() {
    for version in ["abc", "1.2", "1.2.3/escape", "1.2.3\"", "1.2.3\n"] {
        assert_rejected(&[version, ROLES[0].sha, "combined"]);
    }
    for sha in ["abc", "not-a-sha256", &"g".repeat(64), &"a".repeat(65)] {
        assert_rejected(&[VERSION, sha, "combined"]);
    }
}

/// Read the emitted command/args pairs, not a second renderer implementation.
/// Unknown commands or changed argument shapes fail before any execution.
fn cask_commands(cask: &str) -> Vec<(String, Vec<String>)> {
    let mut commands = Vec::new();
    let mut pending = None;
    for line in cask.lines().map(str::trim) {
        if line.starts_with("system_command ") {
            assert!(pending.is_none());
            pending = Some(quoted_values(line)[0].to_owned());
        }
        if let Some((_, args)) = line.split_once("args: ") {
            let program = pending.take().expect("args without system_command");
            commands.push((
                program,
                quoted_values(args).into_iter().map(str::to_owned).collect(),
            ));
        }
    }
    assert!(pending.is_none(), "system_command missing args");
    commands
}

fn executable(path: &Path, contents: &str) {
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// Exercise declared link/remove paths with real filesystem operations under
/// an isolated prefix. This is not a replacement for native Homebrew smoke.
struct Prefix {
    temp: TempDir,
    casks: Vec<String>,
}

impl Prefix {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join("bin")).unwrap();
        let casks = ROLES
            .iter()
            .map(|r| successful(render(&[VERSION, r.sha, r.selector])))
            .collect();
        Self { temp, casks }
    }

    fn bin(&self, name: &str) -> PathBuf {
        self.temp.path().join("bin").join(name)
    }

    fn install(&self, index: usize) {
        let role = &ROLES[index];
        assert_cask(&self.casks[index], role);
        let stage = self.temp.path().join(role.name);
        fs::create_dir(&stage).unwrap();
        let binary = field(&self.casks[index], "binary ");
        executable(
            &stage.join(binary),
            &format!("#!/bin/sh\nprintf '%s\\n' '{}'\n", role.name),
        );
        symlink(stage.join(binary), self.bin(binary)).unwrap();
        self.run_command(index, "/bin/ln");
    }

    fn uninstall(&self, index: usize) {
        self.run_command(index, "/bin/rm");
        fs::remove_file(self.bin(field(&self.casks[index], "binary "))).unwrap();
    }

    fn run_command(&self, index: usize, program: &str) {
        let commands = cask_commands(&self.casks[index]);
        let (_, args) = commands.iter().find(|(p, _)| p == program).unwrap();
        let args: Vec<_> = args
            .iter()
            .map(|a| a.replace("#{HOMEBREW_PREFIX}", self.temp.path().to_str().unwrap()))
            .collect();
        successful(
            isolated(&mut Command::new(program), self.temp.path())
                .args(args)
                .output()
                .unwrap(),
        );
    }

    fn assert_installed(&self, installed: &[bool; 3]) {
        for (role, &present) in ROLES.iter().zip(installed) {
            for path in [self.bin(role.name), self.bin(role.alias)] {
                if present {
                    let output = isolated(&mut Command::new(&path), self.temp.path())
                        .arg("--help")
                        .output()
                        .unwrap();
                    assert_eq!(successful(output).trim(), role.name, "{}", path.display());
                } else {
                    assert!(
                        fs::symlink_metadata(&path).is_err(),
                        "unexpected {}",
                        path.display()
                    );
                }
            }
        }
    }
}

#[test]
fn casks_install_independently() {
    for index in 0..3 {
        let prefix = Prefix::new();
        let mut installed = [false; 3];
        prefix.install(index);
        installed[index] = true;
        prefix.assert_installed(&installed);
        prefix.uninstall(index);
        prefix.assert_installed(&[false; 3]);
    }
}

#[test]
fn casks_coexist_in_every_install_uninstall_order() {
    let orders = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    for install_order in orders {
        for uninstall_order in orders {
            let prefix = Prefix::new();
            let mut installed = [false; 3];
            for index in install_order {
                prefix.install(index);
                installed[index] = true;
                prefix.assert_installed(&installed);
            }
            for index in uninstall_order {
                prefix.uninstall(index);
                installed[index] = false;
                prefix.assert_installed(&installed);
            }
        }
    }
}

fn homebrew_job() -> String {
    let workflow = fs::read_to_string(Path::new(ROOT).join(".github/workflows/release.yml"))
        .unwrap()
        .replace("\r\n", "\n");
    let (_, rest) = workflow
        .split_once("\n  homebrew:\n")
        .expect("homebrew job");
    rest.lines()
        .take_while(|l| {
            !(l.starts_with("  ") && !l.starts_with("   ") && l.trim_end().ends_with(':'))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn workflow_script() -> String {
    let job = homebrew_job();
    let mut blocks = Vec::new();
    let mut lines = job.lines().peekable();
    while let Some(line) = lines.next() {
        if line.trim() == "run: |" {
            let mut block = String::new();
            while let Some(next) = lines.peek() {
                if next.is_empty() {
                    block.push('\n');
                } else if let Some(code) = next.strip_prefix("          ") {
                    block.push_str(code);
                    block.push('\n');
                } else {
                    break;
                }
                lines.next();
            }
            blocks.push(block);
        }
    }
    assert_eq!(blocks.len(), 2, "render and publication run blocks");
    blocks.join("\n")
}

#[test]
fn homebrew_job_is_stable_only_after_release() {
    let job = homebrew_job();
    assert!(job
        .lines()
        .any(|l| l.trim() == "needs: [preflight, github-release]"));
    assert!(job
        .lines()
        .any(|l| l.trim() == "if: ${{ needs.preflight.outputs.prerelease != 'true' }}"));
}

struct Release {
    temp: TempDir,
    git: String,
}

impl Release {
    fn new(missing: Option<usize>) -> Self {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        for dir in ["packaging", "release-set", "seed/Casks", "tools"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        fs::copy(
            Path::new(ROOT).join("packaging/homebrew-cask.sh"),
            root.join("packaging/homebrew-cask.sh"),
        )
        .unwrap();
        let mut checksums = String::from("dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd  iroh-tunnel-1.2.3-linux-arm64.tar.gz\neeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee  iroh-tunnel-serve-1.2.30-darwin-arm64.tar.gz\nffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff  iroh-tunnel-access-1.2.3-darwin-amd64.tar.gz\n");
        for (index, role) in ROLES.iter().enumerate() {
            if missing != Some(index) {
                checksums.push_str(&format!(
                    "{}  {}-{VERSION}-darwin-arm64.tar.gz\n",
                    role.sha, role.name
                ));
            }
            fs::write(
                root.join("seed/Casks").join(format!("{}.rb", role.name)),
                "old cask\n",
            )
            .unwrap();
        }
        fs::write(root.join("release-set/checksums-darwin.txt"), checksums).unwrap();
        fs::write(root.join("seed/Casks/unrelated.rb"), "untouched\n").unwrap();
        let git = successful(
            Command::new("sh")
                .args(["-c", "command -v git"])
                .output()
                .unwrap(),
        )
        .trim()
        .to_owned();
        let release = Self { temp, git };
        release.git("seed", &["init", "--initial-branch=main"]);
        release.git("seed", &["config", "user.name", "Test"]);
        release.git("seed", &["config", "user.email", "test@example.invalid"]);
        release.git("seed", &["add", "Casks"]);
        release.git("seed", &["commit", "-m", "fixture"]);
        // Only external publication is replaced: clone uses the local fixture,
        // push records the attempt without any local or remote ref update.
        executable(
            &release.temp.path().join("tools/git"),
            r#"#!/usr/bin/env bash
set -euo pipefail
case "${1:-}" in
  clone)
    [[ "$*" == "clone --depth 1 https://x-access-token:fixture@github.com/naicoi92/homebrew-tap.git tap" ]] || exit 90
    exec "$REAL_GIT" clone --depth 1 "$LOCAL_TAP" tap
    ;;
  push)
    [[ "$*" == "push origin HEAD:main" && "$PWD" -ef "$TEST_ROOT/tap" ]] || exit 91
    "$REAL_GIT" rev-parse HEAD > "$TEST_ROOT/published-head"
    ;;
  config|add|diff|commit) exec "$REAL_GIT" "$@" ;;
  *) echo "unexpected git operation: $1" >&2; exit 92 ;;
esac
"#,
        );
        release
    }

    fn git(&self, dir: &str, args: &[&str]) -> String {
        successful(
            isolated(&mut Command::new(&self.git), self.temp.path())
                .current_dir(self.temp.path().join(dir))
                .args(args)
                .output()
                .unwrap(),
        )
    }

    fn run(&self) -> Output {
        let root = self.temp.path();
        let path = format!(
            "{}:{}",
            root.join("tools").display(),
            std::env::var("PATH").unwrap()
        );
        isolated(&mut Command::new("bash"), root)
            .args(["-c", &workflow_script()])
            .current_dir(root)
            .env("PATH", path)
            .env("VERSION", VERSION)
            .env("TAP_TOKEN", "fixture")
            .env("REAL_GIT", &self.git)
            .env("LOCAL_TAP", root.join("seed"))
            .env("TEST_ROOT", root)
            .output()
            .unwrap()
    }
}

#[test]
fn stable_release_publishes_three_matching_casks() {
    let release = Release::new(None);
    successful(release.run());
    let root = release.temp.path();
    for role in &ROLES {
        let file = format!("{}.rb", role.name);
        let rendered = fs::read_to_string(root.join("tap-out").join(&file)).unwrap();
        assert_cask(&rendered, role);
        assert_eq!(
            rendered,
            fs::read_to_string(root.join("tap/Casks").join(file)).unwrap()
        );
    }
    assert_eq!(fs::read_dir(root.join("tap-out")).unwrap().count(), 3);
    assert_eq!(
        release.git("tap", &["diff", "--name-only", "HEAD^", "HEAD"]),
        "Casks/iroh-tunnel-access.rb\nCasks/iroh-tunnel-serve.rb\nCasks/iroh-tunnel.rb\n"
    );
    assert!(release.git("tap", &["status", "--porcelain"]).is_empty());
    assert_eq!(
        fs::read_to_string(root.join("tap/Casks/unrelated.rb")).unwrap(),
        "untouched\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("published-head")).unwrap(),
        release.git("tap", &["rev-parse", "HEAD"])
    );
}

#[test]
fn missing_role_checksum_prevents_publication() {
    for (index, role) in ROLES.iter().enumerate() {
        let release = Release::new(Some(index));
        let output = release.run();
        assert!(
            !output.status.success(),
            "published without {} checksum",
            role.name
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains(role.name));
        assert!(
            !release.temp.path().join("tap").exists(),
            "cloned tap after checksum failure"
        );
        assert!(!release.temp.path().join("published-head").exists());
    }
}
