## 1. Host: semver ordering and downgrade guard

Blocked by: none

Files: `crates/host/Cargo.toml`, `Cargo.lock`, `crates/host/src/update/types.rs`, `crates/host/src/update/mod.rs`, `crates/host/src/update/update.rs`, `src/app.rs` (compile-only: new field set to `Refuse`), `tests/update.rs`, `tests/update_ownership.rs` (literal only), `tests/update_zip_windows.rs`. Platforms: shared code; Windows zip test only needs the new field.

- [ ] 1.1 Add `semver = "1"` to `iroh-tunnel-host` dependencies (design D2)
- [ ] 1.2 Add `DowngradePolicy { Refuse, Allow }` and `UpdateRequest::downgrade`, re-export from `update/mod.rs`; set `downgrade` in every existing `UpdateRequest` literal (`src/app.rs` → `Refuse`, tests → `Refuse`) (design D1)
- [ ] 1.3 TDD `parse_version` on `semver::Version` (scenarios "Path traversal value", "Pre-release value accepted")
- [ ] 1.4 TDD semver `is_older` with `cmp_precedence`, non-semver running version never older (scenarios "Dev build updates to its release", "Release candidate updates to its release", "Unordered running version"; design D4)
- [ ] 1.5 TDD the downgrade guard before any network request, error text naming both versions and `--force` (scenarios "Release candidate below running release refused", "Older release candidate refused", "Newer release candidate installed", "Forced downgrade", "Force keeps verification", "Same version with force", "Release candidate"; design D3, D5)

## 2. CLI: `--force` flag, mapping and hint

Blocked by: 1

Files: `src/cli.rs`, `src/app.rs`, `src/render.rs`, `tests/update.rs`, `tests/update_ownership.rs`.

- [ ] 2.1 Add `UpdateArgs::force` with `requires = "to"`; real-binary test for exit 2 naming `--to` (scenario "Force without a version"; design D6)
- [ ] 2.2 Map `--force` to `DowngradePolicy::Allow` in `run_update_cli`; real-binary tests: `update --to 0.0.1` against `http://127.0.0.1:9` exits 1 naming `0.0.1` and `--force` (scenario "Downgrade refused"); `update -i --to 0.0.1 --force` installs the published 0.0.1 build (scenario "Forced downgrade")
- [ ] 2.3 Real-binary ownership tests: container `update --to 0.0.1` prints "redeploy", exit 0 (scenario "Container decision precedes the guard"); Homebrew-owned `update --to 0.0.1 --force` prints owner guidance and leaves the file unchanged (scenario "Force does not override ownership")
- [ ] 2.4 TDD the `NewerThanLatest` hint `--to <latest> --force` in `src/render.rs` plus a real-binary stdout check (scenario "Latest older than running"; design D7)
- [ ] 2.5 Confirm "Explicit version" and "Unknown version" real-binary tests still pass unchanged

## 3. Docs and changelog

Blocked by: none

Files: `docs/cli.md`, `docs/install.md`, `CHANGELOG.md`.

- [ ] 3.1 Document `--force` and release-candidate pinning in `docs/cli.md` (`update` synopsis/options) and `docs/install.md` (replace "does not downgrade unless you pass `--to`")
- [ ] 3.2 Add a **Breaking** entry and an Added entry under `[Unreleased]` in `CHANGELOG.md`
