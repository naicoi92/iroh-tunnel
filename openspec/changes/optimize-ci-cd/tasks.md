Single writer, single worktree. Run groups in order 1 → 5: group 2 keeps `checks.yml` until group 4 removes its last caller. Platforms: ubuntu and windows (PR), all eight release targets (tag).

## 1. Remove auxiliary tests and scripts

Owns: `tests/installer.rs`, `tests/installer_ps.rs`, `tests/layout_distribution.rs`, `tests/release_artifacts.rs`, `tests/release_smoke.rs`, `tests/layout_cli.rs`, `tests/update_ownership.rs` (doc comment only), `packaging/verify-dist.sh`, `packaging/verify-release-set.sh`, `Cargo.toml` (only if a dev-dependency becomes unused).
Scenarios: ci-cd-pipeline "Suite contents".

- [ ] 1.1 Delete `tests/installer.rs`, `tests/installer_ps.rs`, `tests/layout_distribution.rs`, `tests/release_artifacts.rs`, `tests/release_smoke.rs`
- [ ] 1.2 Remove `ci_build_workflow_reads_workspace_version` from `tests/layout_cli.rs` and update its module doc
- [ ] 1.3 Delete `packaging/verify-dist.sh` and `packaging/verify-release-set.sh`; reword the `verify-dist.sh` reference in the `tests/update_ownership.rs` doc comment
- [ ] 1.4 Check that no remaining test reads `.github/`, `packaging/`, `install.sh` or `install.ps1`, and that `cargo test --workspace --all-features --locked --no-run` and `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` pass (unused dev-dependencies or helpers removed if any)

## 2. Pull-request stage (`ci.yml`)

Owns: `.github/workflows/ci.yml`, deletion of `windows-service-e2e.yml`, `windows-update-e2e.yml`; `install.ps1` only for PSScriptAnalyzer findings.
Scenarios: "Code change", "Installer script change only", "Workflow change only", "Docs-only change", "A check fails", "Superseded push", "Script lint", "Lifecycle runs on Windows", "Linux unaffected".

- [ ] 2.1 Rewrite `ci.yml` with jobs `changes`, `lint`, `test-linux`, `test-windows`, `scripts`, `ci-ok` per design D1, path sets per D1, inlining the steps from `checks.yml`
- [ ] 2.2 Add the service-lifecycle step to `test-windows` (`IROH_TUNNEL_E2E_SERVICE=1`, `--test service_user_windows -- --test-threads=1`) per design D2
- [ ] 2.3 Add PSScriptAnalyzer to `scripts` per design D9; run it locally on `install.ps1` and fix or justify-suppress every Warning/Error finding
- [ ] 2.4 Delete `windows-service-e2e.yml`, `windows-update-e2e.yml` (`checks.yml` is deleted in 4.9, after its last caller is gone)

## 3. Cache stage (`cache.yml`)

Owns: `.github/workflows/cache.yml` (new), deletion of `cache-warm.yml`.
Scenarios: "Dependency bump merged", "Source-only change merged".

- [ ] 3.1 Create `cache.yml` (push to `main`, paths per design D3) with compile-only `lint` and `test` jobs (ubuntu, windows) using the same cache keys and flags as `ci.yml`
- [ ] 3.2 Delete `cache-warm.yml`

## 4. Release stage (`release.yml`)

Owns: `.github/workflows/release.yml`, deletion of `checks.yml`, `build.yml`, `package.yml`, `docker.yml`, `snapshot.yml`, `promote.yml`, `release-smoke.yml`.
Scenarios: preflight scenarios ("Commit passed CI", "Pre-release tag", "Commit without a green gate", "Tag off main", "Malformed tag"); release scenarios ("Stable tag", "Pre-release tag", "Signature verification fails", "Build target fails", "Release already exists", "Signing identity unchanged", "Concurrent runs"); install-release-packaging "Tagged build", "Supported targets and naming", "Archive contents", "Native smoke check", "Package targets", "Complete set", "Defective set", "Signing identity and checksum names", "Valid bundle", "Tampered input", "Runtime and promotion tags", "Pre-release tag" (images), "Render".

- [ ] 4.1 `preflight` job: tag format, ancestry of `origin/main`, PR-of-merge-commit lookup, latest `ci-ok` on the PR head, outputs `version`/`prerelease` (design D4)
- [ ] 4.2 `build` job: move the 8-target matrix from `build.yml` unchanged except version from preflight and 1-day retention (D5)
- [ ] 4.3 `package` job: move nFPM packaging from `package.yml` without the `verify-dist.sh` steps (D6)
- [ ] 4.4 `sign` job: expected-set check, checksums, keyless signing, non-empty bundle check, `cosign verify-blob` with the pinned identity/issuer, upload `release-set` (D7)
- [ ] 4.5 `github-release` job: fail if a release exists, create as a transient draft with all assets (pre-release when `-`), then publish it (D8)
- [ ] 4.6 `images` job: 3-image matrix from the `target-gnu-*` artifacts, push to ghcr.io with today's tag rules (D8)
- [ ] 4.7 `homebrew` job: stable only, after `github-release`, render cask from `checksums-darwin.txt` in `release-set`, push to the tap (D8)
- [ ] 4.8 Trigger only on `v*` tag push, keep `concurrency` with `cancel-in-progress: false`, `id-token: write` only on `sign`
- [ ] 4.9 Delete `checks.yml`, `build.yml`, `package.yml`, `docker.yml`, `snapshot.yml`, `promote.yml`, `release-smoke.yml`

## 5. Docs and whole-change verification

Owns: `README.md`, `CONTRIBUTING.md`, `CHANGELOG.md`, header comments of `Dockerfile.release*`, doc comment of `tests/common/release_server.rs`.
Scenarios: "Workflow inventory", "Merge to main".

- [ ] 5.1 README: replace the `snapshot` badge with `ci`/`release`, drop `verify-dist.sh` from the layout table
- [ ] 5.2 CONTRIBUTING: rewrite the PR/CI paragraph and layout table for the three stages; add a short "Releasing" section (tag → automatic publish, recovery by deleting a partial release and re-running, a bad release is superseded by a new tag)
- [ ] 5.3 Update stale comments: `Dockerfile.release*` line 1 (no `build.yml`/`docker.yml`), `tests/common/release_server.rs` doc (no installer tests)
- [ ] 5.4 CHANGELOG `[Unreleased]`: note that releases now publish automatically on tag
- [ ] 5.5 Run actionlint and shellcheck on the result, check `.github/workflows/` holds exactly `ci.yml`, `cache.yml`, `release.yml`, run the full local test suite, and run `openspec validate optimize-ci-cd --strict`
