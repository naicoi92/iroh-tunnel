//! Workspace shape and engine-extraction invariants (Task 19).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");
/// The target is three members; `iroh-tunnel-host` is created by Task 20, so
/// until then the workspace holds the other two. Task 20 adds the host here.
const MEMBERS: [&str; 2] = ["iroh-tunnel", "iroh-tunnel-engine"];
/// Internal packages that must carry `publish = false`.
const INTERNAL_PACKAGES: [&str; 1] = ["iroh-tunnel-engine"];

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
        let text = std::fs::read_to_string(&file).unwrap();
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

#[test]
fn udp_decision_model_lives_in_engine() {
    let text: String = engine_files()
        .iter()
        .map(|f| std::fs::read_to_string(f).unwrap())
        .collect();
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
        let text = std::fs::read_to_string(&file).unwrap();
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

#[test]
fn no_external_crate_added_since_baseline() {
    let lock = std::fs::read_to_string(Path::new(ROOT).join("Cargo.lock")).unwrap();
    let mut current = BTreeSet::new();
    for block in lock.split("[[package]]").skip(1) {
        if !block.contains("source = ") {
            continue;
        }
        let name = block
            .lines()
            .find_map(|l| l.strip_prefix("name = \""))
            .and_then(|l| l.strip_suffix('"'))
            .unwrap();
        current.insert(name.to_string());
    }
    let baseline = std::fs::read_to_string(
        Path::new(ROOT).join("openspec/changes/refactor-codebase/baseline/external-crates.txt"),
    )
    .unwrap();
    let baseline: BTreeSet<String> = baseline
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    let added: Vec<_> = current.difference(&baseline).collect();
    assert!(added.is_empty(), "external crates added: {added:?}");
}
