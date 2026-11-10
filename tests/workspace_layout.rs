//! Workspace shape and engine-extraction invariants (Task 19).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

/// A source file with line endings normalized: a Windows checkout may
/// convert to CRLF, and the checks below match LF-joined lines.
fn read_source(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap().replace("\r\n", "\n")
}
const MEMBERS: [&str; 3] = ["iroh-tunnel", "iroh-tunnel-host", "iroh-tunnel-engine"];
/// Internal packages that must carry `publish = false`.
const INTERNAL_PACKAGES: [&str; 2] = ["iroh-tunnel-host", "iroh-tunnel-engine"];

fn metadata() -> Value {
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--locked"])
        .current_dir(ROOT)
        .output()
        .expect("run cargo metadata");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("metadata json")
}

fn package<'a>(meta: &'a Value, name: &str) -> &'a Value {
    meta["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == name)
        .unwrap_or_else(|| panic!("package {name} missing"))
}

fn names_of(meta: &Value, ids: &Value) -> BTreeSet<String> {
    let ids: BTreeSet<&str> = ids
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    meta["packages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| ids.contains(p["id"].as_str().unwrap()))
        .map(|p| p["name"].as_str().unwrap().to_string())
        .collect()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn engine_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    rust_files(&Path::new(ROOT).join("crates/engine/src"), &mut files);
    files.sort();
    files
}

fn file_name(path: &Path) -> &str {
    path.file_name().unwrap().to_str().unwrap()
}

/// Lines of `text` outside `#[cfg(test)]` items (the item starts at the
/// attribute line and ends at the first line closing at the same indent).
fn non_test_lines(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut skip_indent: Option<usize> = None;
    let mut pending_attr = false;
    for (n, line) in text.lines().enumerate() {
        let indent = line.len() - line.trim_start().len();
        if let Some(i) = skip_indent {
            if indent == i && line.trim_start().starts_with('}') {
                skip_indent = None;
            }
            continue;
        }
        if line.trim() == "#[cfg(test)]" {
            pending_attr = true;
            continue;
        }
        if pending_attr {
            pending_attr = false;
            if line.trim_end().ends_with('{') {
                skip_indent = Some(indent);
            }
            continue;
        }
        out.push((n + 1, line));
    }
    out
}

#[test]
fn workspace_has_exactly_three_members_all_default() {
    let meta = metadata();
    let members = names_of(&meta, &meta["workspace_members"]);
    let defaults = names_of(&meta, &meta["workspace_default_members"]);
    let expected: BTreeSet<String> = MEMBERS.iter().map(|s| s.to_string()).collect();
    assert_eq!(members, expected);
    assert_eq!(defaults, expected);
    let manifest = std::fs::read_to_string(Path::new(ROOT).join("Cargo.toml")).unwrap();
    assert!(manifest.contains("resolver = \"2\""), "resolver 2 missing");
}

#[test]
fn internal_packages_are_not_published() {
    let meta = metadata();
    for name in INTERNAL_PACKAGES {
        assert_eq!(
            package(&meta, name)["publish"],
            serde_json::json!([]),
            "{name} must have publish = false"
        );
    }
}

#[test]
fn engine_depends_on_neither_host_nor_root() {
    let meta = metadata();
    let engine = package(&meta, "iroh-tunnel-engine");
    for dep in engine["dependencies"].as_array().unwrap() {
        let name = dep["name"].as_str().unwrap();
        assert!(
            name != "iroh-tunnel" && name != "iroh-tunnel-host",
            "engine depends on {name} ({} dependency)",
            dep["kind"]
        );
    }
}

#[test]
fn engine_sources_have_no_exit_signal_or_print() {
    let forbidden = [
        concat!("process", "::exit"),
        concat!("tokio", "::signal"),
        concat!("print", "ln!"),
        concat!("eprint", "ln!"),
        concat!("print", "!"),
    ];
    let mut offenders = Vec::new();
    for file in engine_files() {
        if file_name(&file) == "testing.rs" {
            continue;
        }
        let text = read_source(&file);
        for (n, line) in non_test_lines(&text) {
            if line.trim_start().starts_with("//") {
                continue;
            }
            if forbidden.iter().any(|f| line.contains(f)) {
                offenders.push(format!("{}:{n}: {}", file.display(), line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "forbidden calls:\n{}",
        offenders.join("\n")
    );
}

fn host_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    rust_files(&Path::new(ROOT).join("crates/host/src"), &mut files);
    files.sort();
    files
}

#[test]
fn host_depends_neither_on_root_nor_clap() {
    let meta = metadata();
    let host = package(&meta, "iroh-tunnel-host");
    for dep in host["dependencies"].as_array().unwrap() {
        let name = dep["name"].as_str().unwrap();
        assert!(
            !["iroh-tunnel", "clap", "edit", "which", "tracing-subscriber"].contains(&name),
            "host depends on {name} ({} dependency)",
            dep["kind"]
        );
    }
}

/// Offending `file:line` entries for host non-test lines containing any of
/// `forbidden`. Comments, `testing.rs` and `fixtures.rs` (test-only modules
/// declared behind `#[cfg(test)]`) are excluded.
fn host_offenders(forbidden: &[&str]) -> Vec<String> {
    let mut offenders = Vec::new();
    for file in host_files() {
        if ["testing.rs", "fixtures.rs"].contains(&file_name(&file)) {
            continue;
        }
        let text = read_source(&file);
        for (n, line) in non_test_lines(&text) {
            if line.trim_start().starts_with("//") {
                continue;
            }
            if forbidden.iter().any(|f| line.contains(f)) {
                offenders.push(format!("{}:{n}: {}", file.display(), line.trim()));
            }
        }
    }
    offenders
}

#[test]
fn host_sources_have_no_exit_stdin_print_or_signal() {
    let offenders = host_offenders(&[
        concat!("process", "::exit"),
        concat!("tokio", "::signal"),
        concat!("print", "ln!"),
        concat!("eprint", "ln!"),
        concat!("print", "!"),
        concat!("eprint", "!"),
        concat!("std::io::", "stdin"),
        concat!("stdin", "()"),
    ]);
    assert!(
        offenders.is_empty(),
        "forbidden terminal I/O:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn host_sources_open_no_listener() {
    let offenders = host_offenders(&[
        concat!("Tcp", "Listener"),
        concat!("UdpSocket", "::bind"),
        concat!("Unix", "Listener"),
    ]);
    assert!(
        offenders.is_empty(),
        "host opens a listener:\n{}",
        offenders.join("\n")
    );
}

/// Every `run_unowned` / `Ownership::Unenforced` use sits in an item behind
/// `#[cfg(feature = "test-utils")]`; `testing.rs` counts when `lib.rs` gates
/// the `mod testing;` declaration.
#[test]
fn ownership_bypass_is_test_utils_gated() {
    let gate = "#[cfg(feature = \"test-utils\")]";
    let lib = read_source(&Path::new(ROOT).join("crates/host/src/lib.rs"));
    assert!(
        lib.contains(&format!("{gate}\npub mod testing;")),
        "testing module is not test-utils gated in lib.rs"
    );
    let mut offenders = Vec::new();
    for file in host_files() {
        if file_name(&file) == "testing.rs" {
            continue;
        }
        let text = read_source(&file);
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//")
                || !(code.contains("run_unowned") || code.contains("Unenforced"))
            {
                continue;
            }
            // Gated when an attribute run directly above (or the enclosing
            // `#[cfg(feature = "test-utils")]` impl/match-arm head) names it.
            let gated = lines[..i]
                .iter()
                .rev()
                .take_while(|l| {
                    let t = l.trim_start();
                    t.starts_with("#[") || t.starts_with("//")
                })
                .any(|l| l.trim() == gate);
            if !gated {
                offenders.push(format!("{}:{}: {}", file.display(), i + 1, code));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "ungated bypass:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn release_feature_graph_excludes_test_utils() {
    let output = Command::new(env!("CARGO"))
        .args([
            "tree",
            "-e",
            "features",
            "-e",
            "normal",
            "-p",
            "iroh-tunnel",
            "--locked",
        ])
        .current_dir(ROOT)
        .output()
        .expect("run cargo tree");
    assert!(
        output.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let tree = String::from_utf8_lossy(&output.stdout);
    assert!(
        !tree.contains("test-utils"),
        "release feature graph contains test-utils"
    );
}

#[test]
fn root_has_no_self_dev_dependency() {
    let meta = metadata();
    let root = package(&meta, "iroh-tunnel");
    for dep in root["dependencies"].as_array().unwrap() {
        assert_ne!(
            dep["name"], "iroh-tunnel",
            "root depends on itself ({} dependency)",
            dep["kind"]
        );
    }
}

#[test]
fn udp_decision_model_lives_in_engine() {
    let text: String = engine_files().iter().map(|f| read_source(f)).collect();
    for needle in [
        "pub struct FlowId",
        "pub enum Admission",
        "pub struct ClientFlows",
        "pub struct PeerFlows",
        "pub const QUARANTINE",
        "pub fn payload_limit",
    ] {
        assert!(
            text.contains(needle),
            "{needle} not defined under crates/engine/src"
        );
    }
}

/// Private infrastructure files that are exempt from the "has its own unit
/// tests" half of the rule (they are exercised through their callers).
const NO_TEST_ALLOWLIST: [&str; 3] = ["tcp.rs", "proto.rs", "shutdown.rs"];

#[test]
fn engine_files_export_at_most_one_operation() {
    let mut offenders = Vec::new();
    for file in engine_files() {
        let name = file_name(&file);
        if ["lib.rs", "mod.rs", "types.rs", "testing.rs"].contains(&name) {
            continue;
        }
        let text = read_source(&file);
        let count = non_test_lines(&text)
            .iter()
            .filter(|(_, l)| l.starts_with("pub fn ") || l.starts_with("pub async fn "))
            .count();
        if count > 1 {
            offenders.push(format!("{}: {count} exported operations", file.display()));
        }
        if !NO_TEST_ALLOWLIST.contains(&name) && !text.contains("#[cfg(test)]\nmod tests") {
            offenders.push(format!("{}: no #[cfg(test)] mod tests", file.display()));
        }
    }
    assert!(offenders.is_empty(), "{}", offenders.join("\n"));
}

/// Private host infrastructure exempt from the one-operation rule (plan
/// Task 20): each holds helpers, not a frontend-facing operation.
const HOST_INFRASTRUCTURE: [&str; 11] = [
    "store.rs",
    "writer.rs",
    "ownership.rs",
    "native.rs",
    "trust.rs",
    "account.rs",
    "system.rs",
    "pin.rs",
    "task_xml.rs",
    "replace.rs",
    "verify.rs",
];

#[test]
fn host_files_export_at_most_one_operation() {
    let mut offenders = Vec::new();
    for file in host_files() {
        let name = file_name(&file);
        let in_backend = file.parent().is_some_and(|p| p.ends_with("backend"));
        if ["lib.rs", "mod.rs", "types.rs", "testing.rs"].contains(&name) || in_backend {
            continue;
        }
        let text = read_source(&file);
        // Distinct names: a `cfg(unix)` / `cfg(not(unix))` twin is one operation.
        let count = non_test_lines(&text)
            .iter()
            .filter_map(|(_, l)| {
                let rest = l
                    .strip_prefix("pub fn ")
                    .or_else(|| l.strip_prefix("pub async fn "))?;
                Some(rest.split(['(', '<']).next().unwrap().to_string())
            })
            .collect::<BTreeSet<_>>()
            .len();
        // `to_settings.rs` is one mapping operation per role pair (two fns).
        let limit = if name == "to_settings.rs" { 2 } else { 1 };
        if count > limit && !HOST_INFRASTRUCTURE.contains(&name) {
            offenders.push(format!("{}: {count} exported operations", file.display()));
        }
        if !HOST_INFRASTRUCTURE.contains(&name)
            && name != "fixtures.rs"
            && !text.contains("#[cfg(test)]\nmod tests")
        {
            offenders.push(format!("{}: no #[cfg(test)] mod tests", file.display()));
        }
    }
    assert!(offenders.is_empty(), "{}", offenders.join("\n"));
}

/// Engine dependencies are inherited from the workspace
/// (`workspace-boundaries`): every direct entry must be `workspace = true`.
#[test]
fn engine_dependencies_are_inherited_from_the_workspace() {
    let text = read_source(&Path::new(ROOT).join("crates/engine/Cargo.toml"));
    let manifest: toml::Table = toml::from_str(&text).expect("engine manifest parses");
    let offenders = direct_engine_dependencies(&manifest);
    assert!(
        offenders.is_empty(),
        "engine declares dependencies without `workspace = true`:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn direct_engine_dependencies_reports_every_offender() {
    let manifest: toml::Table = toml::from_str(
        r#"
[dependencies]
foo = "1"
off = { workspace = false }
ok = { workspace = true, features = ["x"] }

[dev-dependencies]
bar = { path = "../bar" }

[target.'cfg(unix)'.dependencies]
baz = { version = "1" }
"#,
    )
    .expect("inline manifest parses");
    let mut offenders = direct_engine_dependencies(&manifest);
    offenders.sort();
    // target_tables_are_checked: `baz` under `target.*` is reported.
    // workspace_entry_with_features_passes: `ok` is absent from the output.
    // workspace_false_is_rejected: `off` is reported.
    assert_eq!(
        offenders,
        [
            "dependencies.foo",
            "dependencies.off",
            "dev-dependencies.bar",
            "target.'cfg(unix)'.dependencies.baz",
        ]
    );
}

/// Requirement `workspace-boundaries`: "Engine dependencies are inherited from
/// the workspace". Direct dependency entries of `manifest` that do not inherit
/// from the workspace, as `kind.name` (`target.'<cfg>'.kind.name` under `target.*`).
/// An entry passes only when it is a table with `workspace = true`; Cargo
/// itself rejects `workspace = true` combined with `version`, `path` or `git`,
/// so that combination is not checked here.
fn direct_engine_dependencies(manifest: &toml::Table) -> Vec<String> {
    const KINDS: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];

    fn collect(table: &toml::Table, prefix: &str, offenders: &mut Vec<String>) {
        for (name, value) in table {
            let inherited = matches!(
                value,
                toml::Value::Table(entry)
                    if entry.get("workspace") == Some(&toml::Value::Boolean(true))
            );
            if !inherited {
                offenders.push(format!("{prefix}.{name}"));
            }
        }
    }

    let mut offenders = Vec::new();
    for kind in KINDS {
        if let Some(toml::Value::Table(deps)) = manifest.get(kind) {
            collect(deps, kind, &mut offenders);
        }
    }
    if let Some(toml::Value::Table(targets)) = manifest.get("target") {
        for (cfg, target) in targets {
            let toml::Value::Table(target) = target else {
                continue;
            };
            for kind in KINDS {
                if let Some(toml::Value::Table(deps)) = target.get(kind) {
                    collect(deps, &format!("target.'{cfg}'.{kind}"), &mut offenders);
                }
            }
        }
    }
    offenders
}
