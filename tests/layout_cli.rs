//! Root CLI packaging invariants: three binaries and one shared version (Task 21).

use std::collections::BTreeSet;
use std::process::Command;

use serde_json::Value;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

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

#[test]
fn root_package_defines_three_binaries() {
    let meta = metadata();
    let bins: BTreeSet<&str> = package(&meta, "iroh-tunnel")["targets"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["kind"].as_array().unwrap().iter().any(|k| k == "bin"))
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    let expected: BTreeSet<&str> = ["iroh-tunnel", "iroh-tunnel-serve", "iroh-tunnel-access"]
        .into_iter()
        .collect();
    assert_eq!(bins, expected);
}

#[test]
fn all_packages_share_one_version() {
    let meta = metadata();
    let versions: BTreeSet<&str> = ["iroh-tunnel", "iroh-tunnel-host", "iroh-tunnel-engine"]
        .iter()
        .map(|n| package(&meta, n)["version"].as_str().unwrap())
        .collect();
    assert_eq!(versions.len(), 1, "versions differ: {versions:?}");
}
