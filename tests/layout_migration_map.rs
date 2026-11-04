//! The Rust API migration map must account for every baseline public path.

use std::fs;
use std::path::Path;

const BASELINE: &str = "openspec/changes/refactor-codebase/baseline/public-api.txt";
const MAP: &str = "openspec/changes/refactor-codebase/migration-map.md";

fn cells(row: &str) -> Vec<&str> {
    row.trim()
        .trim_matches('|')
        .split('|')
        .map(str::trim)
        .collect()
}

#[test]
fn migration_map_covers_every_baseline_public_path() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let baseline = fs::read_to_string(root.join(BASELINE)).expect("read baseline");
    let map = fs::read_to_string(root.join(MAP)).expect("read migration map");
    let rows: Vec<Vec<&str>> = map
        .lines()
        .filter(|l| l.trim_start().starts_with('|'))
        .map(cells)
        .filter(|c| c.len() >= 2)
        .collect();

    let missing: Vec<&str> = baseline
        .lines()
        .filter(|p| !p.trim().is_empty())
        .filter(|p| {
            let wanted = format!("`{}`", p.trim());
            !rows.iter().any(|c| c[0] == wanted && !c[1].is_empty())
        })
        .collect();
    assert!(
        missing.is_empty(),
        "baseline paths without a migration-map row: {missing:#?}"
    );
}
