//! Build script — resolve `APP_VERSION` from the git tag at build time.
//!
//! Behaviour (ADR-0003):
//! - If HEAD is exactly on a tag `vX.Y.Z` (or a pre-release like `vX.Y.Z-rc.1`),
//!   `APP_VERSION` = `X.Y.Z` (the `v` prefix is stripped).
//! - Otherwise (no tag, tag not at HEAD, commits ahead of tag, or no `.git`),
//!   `APP_VERSION` = `{CARGO_PKG_VERSION}-dev` (e.g. `0.1.0-dev`).
//!
//! The resolved value is always a valid Cargo semver, so downstream packaging
//! (`.deb`/`.apk`/container tags via GoReleaser) never breaks.
//!
//! vergen-gitcl additionally emits `VERGEN_GIT_*` env vars (sha, branch,
//! describe) for richer build metadata; these are reserved for a future
//! `--build-info` flag (see ADR-0003 §8, deferred).

use std::error::Error;
use std::process::Command;

use vergen_gitcl::{Emitter, GitclBuilder};

/// Return the version derived from the tag at HEAD, if any.
///
/// `git describe --tags --exact-match HEAD` succeeds only when HEAD points
/// exactly at a tag (annotated or lightweight). For any other state (no tag,
/// commits ahead, shallow clone without `.git`) it fails and we return `None`,
/// which the caller maps to the `-dev` fallback.
fn tag_at_head() -> Option<String> {
    let output = Command::new("git")
        .args(["describe", "--tags", "--exact-match", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let tag = String::from_utf8_lossy(&output.stdout).trim().to_string();
    // Accept tags shaped vX.Y.Z or X.Y.Z and strip an optional `v` prefix.
    // Non-numeric tags (e.g. internal markers) fall through to `-dev`.
    let stripped = tag.strip_prefix('v').unwrap_or(&tag);
    let starts_with_digit = stripped
        .chars()
        .next()
        .map(|c| c.is_ascii_digit())
        .unwrap_or(false);
    if starts_with_digit {
        Some(stripped.to_string())
    } else {
        None
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    // (1) vergen-gitcl emits VERGEN_GIT_* env vars for richer metadata
    //     (sha, branch, describe). describe(true) opts into `--tags` so that
    //     lightweight tags (our release convention) are recognised.
    let gitcl = GitclBuilder::default()
        .describe(true, false, None)
        .build()?;
    Emitter::default().add_instructions(&gitcl)?.emit()?;

    // (2) Resolve the final APP_VERSION that source code reads via
    //     `env!("APP_VERSION")`. Computed here (not from VERGEN_*) because
    //     vergen's emitted vars are only visible to the compiler, not to this
    //     build script.
    let cargo_version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string());
    let app_version = tag_at_head().unwrap_or_else(|| format!("{cargo_version}-dev"));
    println!("cargo:rustc-env=APP_VERSION={app_version}");

    Ok(())
}
