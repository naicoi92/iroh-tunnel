//! Distribution-path checks for the workspace layout: CI gates, path filters,
//! Dockerfile dependency-cache stages and the root-level installer scripts.

use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(relative: &str) -> String {
    fs::read_to_string(root().join(relative)).unwrap_or_else(|e| panic!("read {relative}: {e}"))
}

fn workflow_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(root().join(".github/workflows"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "yml"))
        .collect();
    files.sort();
    files
}

/// Quoted `paths:` / filter entries of a workflow: every `- 'x'` or `- "x"` item.
fn quoted_entries(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("- "))
        .map(|item| item.trim_matches(|c| c == '"' || c == '\''))
        .map(str::to_owned)
        .collect()
}

#[test]
fn ci_gates_use_workspace_flags() {
    let checks = read(".github/workflows/checks.yml");
    for gate in [
        "cargo fmt --all",
        "cargo clippy --workspace --all-targets --all-features",
        "cargo test --workspace --all-features",
    ] {
        assert!(checks.contains(gate), "checks.yml must run `{gate}`");
    }
    let ci = read(".github/workflows/ci.yml");
    assert!(
        ci.contains("checks.yml"),
        "ci.yml must delegate the gates to checks.yml"
    );
}

#[test]
fn ci_path_filters_include_crates() {
    for path in workflow_files() {
        let text = fs::read_to_string(&path).unwrap();
        let entries = quoted_entries(&text);
        if entries.iter().any(|e| e == "src/**") {
            assert!(
                entries.iter().any(|e| e == "crates/**"),
                "{} lists src/** but not crates/**",
                path.display()
            );
        }
    }
}

#[test]
fn ci_path_filters_name_existing_source_files() {
    for path in workflow_files() {
        let text = fs::read_to_string(&path).unwrap();
        for entry in quoted_entries(&text) {
            let is_source_file =
                (entry.starts_with("src/") || entry.starts_with("tests/")) && !entry.contains('*');
            if is_source_file {
                assert!(
                    Path::new(&root()).join(&entry).exists(),
                    "{} filters on missing file {entry}",
                    path.display()
                );
            }
        }
    }
}

#[test]
fn dockerfiles_copy_member_manifests() {
    for name in [
        "Dockerfile",
        "Dockerfile.release",
        "Dockerfile.release.access",
        "Dockerfile.release.serve",
    ] {
        let text = read(name);
        let copies: Vec<&str> = text
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with("COPY ") && !l.contains("--from="))
            .collect();
        let copies_manifest = copies.iter().any(|l| l.contains("Cargo.toml"));
        let copies_all = copies.iter().any(|l| l.starts_with("COPY . "));
        if copies_manifest && !copies_all {
            for member in ["crates/engine/Cargo.toml", "crates/host/Cargo.toml"] {
                assert!(
                    copies.iter().any(|l| l.contains(member)),
                    "{name} copies Cargo.toml for caching but not {member}"
                );
            }
        }
    }
}

#[test]
fn installer_scripts_remain_at_root() {
    assert!(root().join("install.sh").is_file());
    assert!(root().join("install.ps1").is_file());
    let readme = read("README.md");
    assert!(
        readme.contains("https://raw.githubusercontent.com/naicoi92/iroh-tunnel/main/install.sh")
    );
}
