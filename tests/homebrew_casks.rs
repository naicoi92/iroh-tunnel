//! Homebrew renderer and stable tap publication contracts (Task 3).
//! Requires Ruby to execute rendered hooks; `RUBY` overrides the executable.
#![cfg(unix)]

use std::fs;
use std::io::Write;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::Value;

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
    let temp = temp_dir();
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
    assert!(cask.lines().any(|l| l.trim() == "depends_on arch: :arm64"));
    let temp = temp_dir();
    let metadata: Value =
        serde_json::from_str(&successful(ruby(cask, temp.path(), "inspect"))).unwrap();
    assert_eq!(
        metadata["binary"],
        serde_json::json!([[role.name, role.name], [role.name, role.alias]])
    );
    assert_eq!(metadata["hooks"], serde_json::json!(["postflight"]));
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

// Execute the real rendered DSL plus pinned, unmodified Homebrew Binary and
// Symlinked classes. Only DSL/installer/platform boundaries are supplied here.
const RUBY_HARNESS: &str = r##"
require "json"
require "pathname"
require "fileutils"
HOMEBREW_PREFIX = Pathname.new(ARGV.fetch(0))
HOMEBREW_CELLAR = HOMEBREW_PREFIX/"Cellar"
class SystemCommand
  def self.run!(program, args:, **options)
    # The pinned generic Unix artifact emits GNU ln options. Adapt only this
    # OS-command boundary, not Homebrew's conflict/link/unlink decisions.
    if program == "/bin/ln" && args.take(3) == ["--no-dereference", "--force", "--symbolic"]
      args = ["-n", "-f", "-s", *args.drop(3)]
    end
    raise "command failed: #{program}" unless system(program, *args.map(&:to_s))
  end
end
require "cask/artifact/binary"
class Harness
  attr_reader :hooks, :binaries
  def initialize
    @hooks = {}
    @binaries = []
  end
  def cask(name, &block)
    @name = name
    instance_eval(&block)
  end
  def version(value = nil)
    @version = value unless value.nil?
    @version
  end
  def sha256(_value); end
  def url(_value); end
  def name(_value); end
  def desc(_value); end
  def homepage(_value); end
  def depends_on(**_values); end
  def binary(name, target: nil)
    @binaries << Cask::Artifact::Binary.new(self, name, target: target)
  end
  def preflight(&block); @hooks["preflight"] = block; end
  def postflight(&block); @hooks["postflight"] = block; end
  def uninstall_preflight(&block); @hooks["uninstall_preflight"] = block; end
  def uninstall_postflight(&block); @hooks["uninstall_postflight"] = block; end
  def caskroom_path; HOMEBREW_PREFIX/"stage"/@name; end
  def staged_path; caskroom_path/@version; end
  def system_command(program, args:, **options)
    raise "unexpected hook options" unless options.empty?
    case program
    when "/usr/bin/xattr"
      File.open(HOMEBREW_PREFIX/"xattr.jsonl", "a") { |f| f.puts(JSON.generate(args.map(&:to_s))) }
      raise "injected xattr failure" if (HOMEBREW_PREFIX/"fail-xattr").exist?
    when "/bin/ln", "/bin/rm"
      # Only the archived legacy combined cask uses manual alias commands.
      SystemCommand.run!(program, args: args)
    else raise "unexpected hook command"
    end
  end
  def hook(name)
    instance_eval(&@hooks.fetch(name)) if @hooks.key?(name)
  end
  def install
    completed = []
    hook("preflight")
    @binaries.each do |artifact|
      artifact.install_phase
      completed.unshift(artifact)
    end
    hook("postflight")
  rescue
    completed.each(&:uninstall_phase)
    raise
  end
  def uninstall
    hook("uninstall_preflight")
    @binaries.each(&:uninstall_phase)
    hook("uninstall_postflight")
  end
end
h = Harness.new
h.instance_eval(STDIN.read, "rendered-cask.rb")
case ARGV.fetch(1)
when "inspect"
  puts JSON.generate(binary: h.binaries.map { |b| [b.source.basename.to_s, b.target.basename.to_s] }, hooks: h.hooks.keys.sort)
when "preflight" then h.hook("preflight")
when "install" then h.install
when "uninstall" then h.uninstall
else raise "unknown phase"
end
"##;

/// Temp dir under the canonical temp root: the vendored Homebrew code compares
/// `realpath` results with `HOMEBREW_PREFIX`, and macOS `/var` links to `/private/var`.
fn temp_dir() -> TempDir {
    let base = std::env::temp_dir()
        .canonicalize()
        .expect("canonical temp dir");
    TempDir::new_in(base).unwrap()
}

fn ruby_executable() -> String {
    std::env::var("RUBY").unwrap_or_else(|_| "ruby".into())
}

/// The vendored Homebrew fixtures use Ruby 3.1 syntax; fail with guidance
/// instead of a SyntaxError from an older system Ruby (macOS ships 2.6).
fn require_ruby_3_1() {
    static CHECKED: std::sync::Once = std::sync::Once::new();
    CHECKED.call_once(|| {
        let ruby = ruby_executable();
        let output = Command::new(&ruby)
            .args([
                "-e",
                "exit(Gem::Version.new(RUBY_VERSION) >= Gem::Version.new('3.1') ? 0 : 1)",
            ])
            .output()
            .expect("Ruby is required for Homebrew cask hook tests (set RUBY to override)");
        assert!(
            output.status.success(),
            "Homebrew cask hook tests need Ruby >= 3.1; `{ruby}` is older. Set RUBY, for \
             example to Homebrew's portable Ruby \
             ($(brew --repository)/Library/Homebrew/vendor/portable-ruby/current/bin/ruby)"
        );
    });
}

fn ruby(cask: &str, root: &Path, phase: &str) -> Output {
    require_ruby_3_1();
    let mut child = isolated(&mut Command::new(ruby_executable()), root)
        .arg("-I")
        .arg(Path::new(ROOT).join("tests/fixtures/homebrew"))
        .args(["-e", RUBY_HARNESS])
        .arg(root)
        .arg(phase)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Ruby is required for Homebrew cask hook tests (set RUBY to override)");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(cask.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn executable(path: &Path, contents: &str) {
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// Executes real rendered Ruby hooks against an isolated Homebrew prefix.
struct Prefix {
    temp: TempDir,
    casks: Vec<String>,
}

impl Prefix {
    fn new() -> Self {
        let temp = temp_dir();
        fs::create_dir(temp.path().join("bin")).unwrap();
        let casks = ROLES
            .iter()
            .map(|r| successful(render(&[VERSION, r.sha, r.selector])))
            .collect();
        for role in &ROLES {
            let stage = temp.path().join("stage").join(role.name).join(VERSION);
            fs::create_dir_all(&stage).unwrap();
            executable(
                &stage.join(role.name),
                &format!("#!/bin/sh\nprintf '%s\\n' '{}'\n", role.name),
            );
        }
        Self { temp, casks }
    }

    fn bin(&self, name: &str) -> PathBuf {
        self.temp.path().join("bin").join(name)
    }

    fn install(&self, index: usize) {
        successful(ruby(&self.casks[index], self.temp.path(), "install"));
        let log = fs::read_to_string(self.temp.path().join("xattr.jsonl")).unwrap();
        let args: Value = serde_json::from_str(log.lines().last().unwrap()).unwrap();
        assert_eq!(
            args,
            serde_json::json!([
                "-dr",
                "com.apple.quarantine",
                self.temp
                    .path()
                    .join("stage")
                    .join(ROLES[index].name)
                    .join(VERSION)
                    .join(ROLES[index].name)
            ])
        );
    }

    fn uninstall(&self, index: usize) {
        let before = fs::read(self.temp.path().join("xattr.jsonl")).unwrap();
        successful(ruby(&self.casks[index], self.temp.path(), "uninstall"));
        assert_eq!(
            before,
            fs::read(self.temp.path().join("xattr.jsonl")).unwrap(),
            "quarantine command ran during uninstall"
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

/// Reject GNU-only options even on Linux, but execute accepted commands on the
/// real filesystem. Reverting the command-boundary adapter must break this test.
#[test]
fn artifact_commands_install_with_bsd_compatible_ln_options() {
    for (index, role) in ROLES.iter().enumerate() {
        let mut prefix = Prefix::new();
        prefix.casks[index] = format!(
            r#"
class << SystemCommand
  def system(program, *args)
    if program == "/bin/ln" && args.any? {{ |arg| arg.start_with?("--") }}
      raise "BSD ln does not accept GNU long options"
    end
    super
  end
end
{}
"#,
            prefix.casks[index]
        );
        symlink("missing", prefix.bin(role.alias)).unwrap();
        prefix.install(index);
        let mut installed = [false; 3];
        installed[index] = true;
        prefix.assert_installed(&installed);
        prefix.uninstall(index);
        prefix.assert_installed(&[false; 3]);
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

#[test]
fn preflight_has_no_install_side_effects() {
    for index in 0..ROLES.len() {
        let prefix = Prefix::new();
        successful(ruby(&prefix.casks[index], prefix.temp.path(), "preflight"));
        assert_eq!(
            fs::read_dir(prefix.temp.path().join("bin"))
                .unwrap()
                .count(),
            0
        );
        assert!(!prefix.temp.path().join("xattr.jsonl").exists());
    }
}

#[derive(Clone, Copy, Debug)]
enum Conflict {
    File,
    ForeignLink,
    Directory,
    DirectoryLink,
}
const CONFLICTS: [Conflict; 4] = [
    Conflict::File,
    Conflict::ForeignLink,
    Conflict::Directory,
    Conflict::DirectoryLink,
];

fn conflict(prefix: &Prefix, path: &Path, kind: Conflict) {
    let foreign = prefix.temp.path().join("foreign");
    match kind {
        Conflict::File => fs::write(path, "unrelated file").unwrap(),
        Conflict::ForeignLink => {
            fs::write(&foreign, "unrelated target").unwrap();
            symlink(&foreign, path).unwrap();
        }
        Conflict::Directory => {
            fs::create_dir(path).unwrap();
            fs::write(path.join("sentinel"), "unrelated child").unwrap();
        }
        Conflict::DirectoryLink => {
            fs::create_dir(&foreign).unwrap();
            fs::write(foreign.join("sentinel"), "unrelated child").unwrap();
            symlink(&foreign, path).unwrap();
        }
    }
}
fn assert_conflict_unchanged(prefix: &Prefix, path: &Path, kind: Conflict) {
    match kind {
        Conflict::File => assert_eq!(fs::read_to_string(path).unwrap(), "unrelated file"),
        Conflict::ForeignLink => {
            assert_eq!(
                fs::read_link(path).unwrap(),
                prefix.temp.path().join("foreign")
            );
            assert_eq!(fs::read_to_string(path).unwrap(), "unrelated target");
        }
        Conflict::Directory | Conflict::DirectoryLink => {
            assert!(path.is_dir());
            assert_eq!(
                fs::read_to_string(path.join("sentinel")).unwrap(),
                "unrelated child"
            );
            assert_eq!(fs::read_dir(path).unwrap().count(), 1);
        }
    }
}

#[test]
fn managed_artifacts_reject_live_conflicts_without_leaving_links() {
    for (index, role) in ROLES.iter().enumerate() {
        for destination in [role.name, role.alias] {
            for kind in CONFLICTS {
                let prefix = Prefix::new();
                let path = prefix.bin(destination);
                conflict(&prefix, &path, kind);
                let output = ruby(&prefix.casks[index], prefix.temp.path(), "install");
                assert!(
                    !output.status.success(),
                    "accepted {kind:?} at {destination}"
                );
                assert!(String::from_utf8_lossy(&output.stderr).contains(destination));
                assert_conflict_unchanged(&prefix, &path, kind);
                let other = if destination == role.name {
                    role.alias
                } else {
                    role.name
                };
                assert!(
                    fs::symlink_metadata(prefix.bin(other)).is_err(),
                    "partial install not rolled back"
                );
                assert!(!prefix.temp.path().join("xattr.jsonl").exists());
            }
        }
    }
}

#[test]
fn managed_artifacts_follow_homebrew_dangling_link_policy() {
    for (index, role) in ROLES.iter().enumerate() {
        let prefix = Prefix::new();
        symlink("missing", prefix.bin(role.alias)).unwrap();
        prefix.install(index);
        assert_eq!(
            fs::read_link(prefix.bin(role.alias)).unwrap(),
            prefix
                .temp
                .path()
                .join("stage")
                .join(role.name)
                .join(VERSION)
                .join(role.name)
        );
        prefix.uninstall(index);
        prefix.assert_installed(&[false; 3]);
    }
}

#[test]
fn managed_artifacts_follow_homebrew_replacement_policy() {
    for (index, role) in ROLES.iter().enumerate() {
        for kind in CONFLICTS {
            let prefix = Prefix::new();
            prefix.install(index);
            let alias = prefix.bin(role.alias);
            fs::remove_file(&alias).unwrap();
            conflict(&prefix, &alias, kind);
            prefix.uninstall(index);
            if matches!(kind, Conflict::ForeignLink | Conflict::DirectoryLink) {
                assert!(
                    fs::symlink_metadata(&alias).is_err(),
                    "Homebrew unlinks non-formula replacement symlinks"
                );
                assert!(
                    prefix.temp.path().join("foreign").exists(),
                    "link target must survive"
                );
            } else {
                assert_conflict_unchanged(&prefix, &alias, kind);
            }
        }
        let prefix = Prefix::new();
        prefix.install(index);
        let alias = prefix.bin(role.alias);
        let same_target = fs::read_link(&alias).unwrap();
        fs::remove_file(&alias).unwrap();
        symlink(&same_target, &alias).unwrap();
        prefix.uninstall(index);
        assert!(
            fs::symlink_metadata(alias).is_err(),
            "approved same-target replacement limitation"
        );
    }
}

#[test]
fn managed_artifacts_skip_formula_owned_alias() {
    for (index, role) in ROLES.iter().enumerate() {
        let prefix = Prefix::new();
        let formula = prefix.temp.path().join("Cellar/other/1/bin/tool");
        fs::create_dir_all(formula.parent().unwrap()).unwrap();
        executable(&formula, "#!/bin/sh\nexit 0\n");
        let alias = prefix.bin(role.alias);
        symlink(&formula, &alias).unwrap();
        prefix.install(index);
        assert_eq!(fs::read_link(&alias).unwrap(), formula);
        prefix.uninstall(index);
        assert_eq!(fs::read_link(alias).unwrap(), formula);
    }
}

#[test]
fn postflight_failure_rolls_back_managed_artifacts() {
    for index in 0..ROLES.len() {
        let prefix = Prefix::new();
        fs::write(prefix.temp.path().join("fail-xattr"), "").unwrap();
        let output = ruby(&prefix.casks[index], prefix.temp.path(), "install");
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("injected xattr failure"));
        prefix.assert_installed(&[false; 3]);
        assert_eq!(
            fs::read_dir(prefix.temp.path().join("bin"))
                .unwrap()
                .count(),
            0
        );
    }
}

#[test]
fn upgrade_relinks_both_artifacts_to_new_version() {
    for (index, role) in ROLES.iter().enumerate() {
        let mut prefix = Prefix::new();
        prefix.install(index);
        let old = prefix
            .temp
            .path()
            .join("stage")
            .join(role.name)
            .join(VERSION);
        prefix.uninstall(index);
        let next = old.parent().unwrap().join("1.2.4");
        fs::create_dir(&next).unwrap();
        executable(
            &next.join(role.name),
            &format!("#!/bin/sh\nprintf '%s\\n' '{}'\n", role.name),
        );
        fs::remove_dir_all(&old).unwrap();
        prefix.casks[index] = successful(render(&["1.2.4", role.sha, role.selector]));
        successful(ruby(&prefix.casks[index], prefix.temp.path(), "install"));
        for name in [role.name, role.alias] {
            assert_eq!(
                fs::read_link(prefix.bin(name)).unwrap(),
                next.join(role.name)
            );
            assert_eq!(
                successful(
                    isolated(&mut Command::new(prefix.bin(name)), prefix.temp.path())
                        .output()
                        .unwrap()
                )
                .trim(),
                role.name
            );
        }
        prefix.uninstall(index);
        prefix.assert_installed(&[false; 3]);
    }
}

#[test]
fn legacy_combined_upgrade_removes_manual_alias_before_managed_install() {
    let prefix = Prefix::new();
    let old = prefix.temp.path().join("stage/iroh-tunnel/1.2.2");
    fs::create_dir(&old).unwrap();
    executable(&old.join("iroh-tunnel"), "#!/bin/sh\nexit 0\n");
    let legacy = include_str!("fixtures/homebrew/legacy-combined.rb");
    successful(ruby(legacy, prefix.temp.path(), "install"));
    assert_eq!(
        fs::read_link(prefix.bin("it")).unwrap(),
        Path::new("iroh-tunnel")
    );
    // Homebrew uses the stored old cask's artifacts/hooks for this phase.
    successful(ruby(legacy, prefix.temp.path(), "uninstall"));
    prefix.assert_installed(&[false; 3]);
    fs::remove_dir_all(old).unwrap();
    prefix.install(0);
    assert_eq!(
        fs::read_link(prefix.bin("it")).unwrap(),
        prefix
            .temp
            .path()
            .join("stage/iroh-tunnel")
            .join(VERSION)
            .join("iroh-tunnel")
    );
    prefix.uninstall(0);
    prefix.assert_installed(&[false; 3]);
}

#[test]
fn managed_artifacts_accept_existing_same_source_link() {
    for (index, role) in ROLES.iter().enumerate() {
        let prefix = Prefix::new();
        let source = prefix
            .temp
            .path()
            .join("stage")
            .join(role.name)
            .join(VERSION)
            .join(role.name);
        let alias = prefix.bin(role.alias);
        symlink(&source, &alias).unwrap();
        prefix.install(index);
        assert_eq!(fs::read_link(&alias).unwrap(), source);
        prefix.uninstall(index);
        assert!(fs::symlink_metadata(alias).is_err());
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
        let temp = temp_dir();
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
