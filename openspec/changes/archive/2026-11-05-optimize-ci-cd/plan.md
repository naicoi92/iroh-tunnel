# Optimize CI/CD Implementation Plan

> Executed via `/opsx-apply`, which chooses the executor.
> Spec reference: this change's `specs/` (`ci-cd-pipeline`, `install-release-packaging`) and `design.md`.

**Goal:** Replace 13 overlapping workflows with three single-purpose stages (PR test gate, compile-only cache on main, tag release that publishes automatically), and remove tests that target auxiliary tooling.

**Architecture:** `ci.yml` runs lint, Linux tests, Windows tests (with the real Task Scheduler lifecycle) and static script lint in parallel behind `ci-ok`. `cache.yml` compiles on main to seed caches. `release.yml` is one self-contained run: preflight (checks `ci-ok` on the merged PR) → build (8 targets) → package → sign+verify → publish (GitHub Release ∥ ghcr.io ∥ Homebrew). No reusable workflows remain.

**Tech Stack:** GitHub Actions, `dorny/paths-filter@v3`, `Swatinem/rust-cache@v2` (via `.github/actions/setup-rust`), cosign v3.1.4 via `sigstore/cosign-installer@v4.1.2`, nFPM, docker buildx, `gh` CLI, shellcheck, actionlint 1.7.7, PSScriptAnalyzer.

## Global Constraints

- Signing identity, byte for byte: `https://github.com/naicoi92/iroh-tunnel/.github/workflows/release.yml@refs/tags/<tag>`, issuer `https://token.actions.githubusercontent.com`. `sign` is a job of `release.yml`; it is the only job with `id-token: write`.
- `.github/workflows/` ends with exactly `ci.yml`, `cache.yml`, `release.yml`. None has `workflow_dispatch`.
- The aggregate PR check keeps the job name and display name `ci-ok`.
- Rust toolchain stays 1.91 through `.github/actions/setup-rust` (unchanged). Cache keys: `lint`, `test` (saved on main by `cache.yml`), `release-<id>` (never saved; release builds are cold by decision, design D5).
- Tasks run in order 1 → 8. Between Task 3 and Task 6 the branch's `release.yml` still calls `checks.yml`/`build.yml`; that is fine because nothing is tagged mid-branch, and `checks.yml` is deleted only in Task 6.
- Artifact names, archive names, package names, image names/tags and the cask are unchanged from today's `build.yml` / `package.yml` / `docker.yml` / `promote.yml`.
- No change to Rust source under `src/` or `crates/`. `install.ps1` may change only to satisfy PSScriptAnalyzer.
- All workflow, doc and commit text in English.

## Review Focus

1. Tag pushed on a commit where main moved after the PR's CI run → preflight passes (accepted gap, design D4); reviewer confirms the gap is documented in CONTRIBUTING.
2. Release re-run after a partial failure in `github-release` → the job must fail on the existing release rather than overwrite it; covered by Task 6 step 6.
3. Pre-release tag (`-rc.N`) → no `latest` image tag, no cask change, release marked pre-release; covered by Task 6 steps 6-8.
4. PR that changes only `install.ps1` → no Rust compile; covered by Task 3 path sets and Task 8 inventory check.
5. `service_user_windows` gated tests silently pass when the env var is missing → Task 3 step 3 sets it on the step, and Task 8 step 5 checks the PR run log shows the lifecycle test running, not skipped.
6. `release.yml` has no rehearsal → the first tag after merge is `-rc.N` (Task 7 step 2 documents it; Task 8 step 6).
7. Stable tag accidentally marked pre-release (moves GitHub's `latest` and breaks `releases/latest/download`) → Task 6 step 4 uses an args array tested on `PRERELEASE == true` only.

---

## Contracts

| Name | new / modified | Shape | public |
|---|---|---|---|
| `ci.yml` jobs | modified | `changes` → {`lint`, `test-linux`, `test-windows`, `scripts`} → `ci-ok` (always). `changes.outputs`: `code`, `scripts` (strings `'true'`/`'false'`) | no |
| `ci-ok` check | modified (unchanged name) | job id and name `ci-ok`; fails if any `needs.*.result` is `failure` or `cancelled` | yes — preflight and branch protection depend on the name |
| `cache.yml` | new | `on.push.branches: [main]`, `paths`: `Cargo.toml`, `crates/*/Cargo.toml`, `Cargo.lock`, `.github/actions/setup-rust/**`, `.github/workflows/cache.yml`; jobs `lint` (ubuntu, key `lint`), `test` (matrix ubuntu-latest, windows-latest; key `test`) | no |
| `release.yml` trigger | modified | `on.push.tags: ["v*"]` only; `concurrency: { group: release-${{ github.ref }}, cancel-in-progress: false }` | no |
| `release.yml` jobs | modified | `preflight` → `build` (matrix of 8, `fail-fast: true`) → `package` → `sign` → {`github-release`, `images` (matrix of 3)}; `homebrew` needs `github-release` and `if: needs.preflight.outputs.prerelease != 'true'` | no |
| `preflight.outputs` | new | `version` (tag without `v`), `prerelease` (`'true'`/`'false'`) | no |
| Run artifacts | modified | `target-<id>` (archives + raw unix binaries), `packages` (`*.deb`), `packages-apk` (`*.apk`), `release-set` (all assets + `checksums.txt`, `checksums-darwin.txt`, `*.sigstore.json`); retention 7 days for all | no |
| Release asset set | modified (same names) | for each role in {`iroh-tunnel`,`iroh-tunnel-serve`,`iroh-tunnel-access`}: `<role>-<v>-linux-{amd64,arm64}.tar.gz`, `<role>-<v>-linux-{amd64,arm64,riscv64gc}-musl.tar.gz`, `<role>-<v>-darwin-arm64.tar.gz`, `<role>-<v>-windows-{amd64,arm64}.zip`, `<role>_<v>_{amd64,arm64}.deb`, `<role>_<v>_{amd64,arm64,riscv64}.apk`; plus `checksums.txt`, `checksums-darwin.txt` and their `.sigstore.json` | yes — installers and `update` read these names |

---

## Task 1: Remove auxiliary tests and scripts — implements 1.1, 1.2, 1.3, 1.4

**Files:**
- Delete: `tests/installer.rs`, `tests/installer_ps.rs`, `tests/layout_distribution.rs`, `tests/release_artifacts.rs`, `tests/release_smoke.rs`, `packaging/verify-dist.sh`, `packaging/verify-release-set.sh`
- Modify: `tests/layout_cli.rs` (remove `ci_build_workflow_reads_workspace_version`, lines 60-72; module doc lines 1-2), `tests/update_ownership.rs:5-6` (doc comment)

**Interfaces:** Produces: a test suite with no reads of `.github/`, `packaging/`, `install.sh`, `install.ps1`. `tests/common/release_server.rs` stays (used by `update.rs`, `update_ownership.rs`, `update_zip_windows.rs`, `quick_run.rs`, `role_binaries.rs`).

- [ ] **Step 1:** `git rm` the seven files listed above.
- [ ] **Step 2:** In `tests/layout_cli.rs` delete `ci_build_workflow_reads_workspace_version`; module doc becomes `//! Root CLI packaging invariants: three binaries and one shared version (Task 21).`
- [ ] **Step 3:** In `tests/update_ownership.rs` replace the last doc sentence with `Real package managers and images are not exercised by the test suite.`
- [ ] **Step 4:** Run `rg -n '\.github/|packaging/|install\.sh|install\.ps1' tests/` — Expected: no match except comments that do not read files (fix any reader).
- [ ] **Step 5:** Run `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` and `cargo test --workspace --all-features --locked` — Expected: PASS. `tests/common/mod.rs` has `#![allow(dead_code)]`, so also grep each helper in `tests/common/*.rs` for remaining callers and remove helpers with none. Dev-dependencies: every one still used by kept tests except `serde` and `bytes`, which were already unused before this change — leave them (out of scope).
- [ ] **Step 6:** Commit: `git commit -m "test: drop tests and scripts that exercise release tooling"`

---

## Task 2: Lint `install.ps1` with PSScriptAnalyzer — implements 2.3 (script part)

**Files:** Modify: `install.ps1` (only lines flagged).

- [ ] **Step 1:** Run `pwsh -NoProfile -Command 'Install-Module PSScriptAnalyzer -Scope CurrentUser -Force; Invoke-ScriptAnalyzer -Path install.ps1 -Severity Warning,Error'` — record findings.
- [ ] **Step 2:** Fix each finding in place, or add `[Diagnostics.CodeAnalysis.SuppressMessageAttribute('<Rule>', '', Justification = '<reason>')]` at script `param()` level when the pattern is intentional (for example `Write-Host` for user prompts).
- [ ] **Step 3:** Re-run with `-EnableExit` — Expected: exit 0, no output. Also `IROH_TUNNEL_TARGET=windows-amd64 pwsh -NoProfile -File install.ps1 -DryRun -Version 0.4.2` on macOS (no network needed with an explicit version) must print the same dry-run plan as before the edit (diff the two outputs).
- [ ] **Step 4:** Commit: `git commit -m "chore(install): satisfy PSScriptAnalyzer"`

---

## Task 3: Pull-request stage — implements 2.1, 2.2, 2.3 (job), 2.4

**Files:** Modify: `.github/workflows/ci.yml`. Delete: `.github/workflows/checks.yml`, `windows-service-e2e.yml`, `windows-update-e2e.yml`.

**Interfaces:** Produces: contracts `ci.yml jobs`, `ci-ok check`.

- [ ] **Step 1:** Rewrite `ci.yml` header comment to the D1 diagram. Keep `on: pull_request`, `concurrency: ci-${{ github.event.pull_request.number }}` with `cancel-in-progress: true`, `permissions: contents: read`.
- [ ] **Step 2:** `changes` job: same as today; `code` filter = `src/**`, `crates/**`, `tests/**`, `examples/**`, `Cargo.toml`, `Cargo.lock`, `build.rs`, `.github/actions/**`, `.github/workflows/ci.yml`; `scripts` filter = `**/*.sh`, `install.ps1`, `.github/**`.
- [ ] **Step 3:** `lint` (ubuntu-latest, `if: code`): checkout, setup-rust `components: rustfmt,clippy`, `cache-key: lint`, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`. `test-linux` (ubuntu-latest) and `test-windows` (windows-latest), both `if: code`, `cache-key: test`, `cargo test --workspace --all-features --locked`. `test-windows` adds a second step `Service lifecycle (Task Scheduler)`, `shell: pwsh`, `env: IROH_TUNNEL_E2E_SERVICE: "1"`, `run: cargo test --all-features --locked --test service_user_windows -- --nocapture --test-threads=1`.
- [ ] **Step 4:** `scripts` (ubuntu-latest, `if: scripts`): existing shellcheck and actionlint steps, plus `shell: pwsh` step: `Install-Module PSScriptAnalyzer -Scope CurrentUser -Force; Invoke-ScriptAnalyzer -Path install.ps1 -Severity Warning,Error -EnableExit`.
- [ ] **Step 5:** `ci-ok`: `needs: [changes, lint, test-linux, test-windows, scripts]`, body unchanged.
- [ ] **Step 6:** `git rm .github/workflows/windows-service-e2e.yml .github/workflows/windows-update-e2e.yml` (keep `checks.yml`: `cache-warm.yml` and `release.yml` still call it until Tasks 4 and 6). Run `actionlint` — Expected: no output. The `ci-ok` job body stays byte-identical to today's (the Docs-only and A-check-fails scenarios rely on it).
- [ ] **Step 7:** Commit: `git commit -m "ci: one pull-request workflow with parallel lint, tests and script lint"`

---

## Task 4: Cache stage — implements 3.1, 3.2

**Files:** Create: `.github/workflows/cache.yml`. Delete: `.github/workflows/cache-warm.yml`.

**Interfaces:** Consumes: cache keys `lint`, `test` from Task 3. Produces: contract `cache.yml`.

- [ ] **Step 1:** Create `cache.yml` with the trigger and paths from the contract, `concurrency: { group: cache, cancel-in-progress: true }`, `permissions: contents: read`.
- [ ] **Step 2:** Job `lint`: setup-rust `components: clippy`, `cache-key: lint`, run `cargo clippy --workspace --all-targets --all-features --locked`. Job `test` (matrix `os: [ubuntu-latest, windows-latest]`): setup-rust `cache-key: test`, run `cargo test --workspace --all-features --locked --no-run`.
- [ ] **Step 3:** `git rm .github/workflows/cache-warm.yml`; run `actionlint` — Expected: no output.
- [ ] **Step 4:** Commit: `git commit -m "ci: warm caches on main by compiling only"`

---

## Task 5: Release stage — preflight, build, package — implements 4.1, 4.2, 4.3, 4.8

**Files:** Modify: `.github/workflows/release.yml` (rewrite). Source of moved steps: `.github/workflows/build.yml:65-187`, `.github/workflows/package.yml:24-88`.

**Interfaces:** Produces: contracts `release.yml trigger`, `preflight.outputs`, run artifacts `target-<id>`, `packages`, `packages-apk`.

- [ ] **Step 1:** Header comment with the D1 release diagram and the signing-identity note. `on: push: tags: ["v*"]` only; concurrency per contract; top-level `permissions: contents: read`.
- [ ] **Step 2:** `preflight` job (ubuntu-latest; permissions `contents: read`, `checks: read`, `pull-requests: read`; checkout `fetch-depth: 0`; `GH_TOKEN: ${{ github.token }}`). Bash, `set -euo pipefail`:
  - tag `${GITHUB_REF_NAME}` must match `^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$`;
  - `git fetch origin main` then `git merge-base --is-ancestor "$GITHUB_SHA" origin/main`;
  - `pr_head=$(gh api "repos/$GITHUB_REPOSITORY/commits/$GITHUB_SHA/pulls" --jq "[.[] | select(.merge_commit_sha == \"$GITHUB_SHA\")][0].head.sha // empty")`, fail with `no merged pull request for $GITHUB_SHA` when empty;
  - `concl=$(gh api "repos/$GITHUB_REPOSITORY/commits/$pr_head/check-runs?check_name=ci-ok" --jq '[.check_runs[]] | sort_by(.completed_at) | last | .conclusion // empty')`, fail unless `success`;
  - write `version=${GITHUB_REF_NAME#v}` and `prerelease=true|false` (contains `-`) to `$GITHUB_OUTPUT`; append a one-line summary to `$GITHUB_STEP_SUMMARY`.
- [ ] **Step 3:** `build` job: `needs: preflight`, matrix/env/steps copied from `build.yml` job `build` unchanged, with `VERSION: ${{ needs.preflight.outputs.version }}`, `cache-key: release-${{ matrix.id }}` and `retention-days: 7`.
- [ ] **Step 4:** `package` job: `needs: [preflight, build]`, steps from `package.yml` minus the "Verify packages and archives" step; `VERSION` from preflight; retention 7 days.
- [ ] **Step 5:** Run `actionlint` — Expected: no output. Dry-check preflight logic locally against the last merge: `GITHUB_SHA=$(git rev-parse adffb73) GITHUB_REPOSITORY=naicoi92/iroh-tunnel` and run the two `gh api` lines — Expected: `pr_head` = head of PR #170, conclusion `success`.
- [ ] **Step 6:** Commit: `git commit -m "ci(release): preflight on ci-ok, build and package in the tag run"`

---

## Task 6: Release stage — sign, verify, publish — implements 4.4, 4.5, 4.6, 4.7, 4.9

**Files:** Modify: `.github/workflows/release.yml`. Delete: `build.yml`, `package.yml`, `docker.yml`, `snapshot.yml`, `promote.yml`, `release-smoke.yml`. Source of moved steps: `release.yml:63-126` (today's sign), `docker.yml:35-125`, `promote.yml:83-131`.

**Interfaces:** Consumes: Task 5 artifacts and outputs. Produces: run artifact `release-set`; contract `Release asset set`.

- [ ] **Step 1:** `sign` job (`needs: [preflight, build, package]`, permissions `contents: read`, `id-token: write`): download `target-*` and `packages*` as today, collect into `dist/`.
- [ ] **Step 2:** In `sign`, expected-set check before checksums, after the archives (from `target-*`) and packages (from `packages*`) are copied into an otherwise empty `dist/`: build the expected list from the contract (`VERSION` from preflight), `LC_ALL=C sort` both it and `ls dist`, and `comm -3` them; on any line print `FAIL missing <name>` or `FAIL unexpected <name>` and exit 1.
- [ ] **Step 3:** In `sign`, write `checksums.txt` / `checksums-darwin.txt` exactly as today's `release.yml:104-110`, sign both with today's cosign steps, then `test -s` each `.sigstore.json`, then `(cd dist && sha256sum -c checksums.txt && sha256sum -c checksums-darwin.txt)`, then `for f in checksums.txt checksums-darwin.txt; do cosign verify-blob --bundle "$f.sigstore.json" --certificate-identity "https://github.com/${GITHUB_REPOSITORY}/.github/workflows/release.yml@${GITHUB_REF}" --certificate-oidc-issuer https://token.actions.githubusercontent.com "dist/$f"; done` (bundle path `dist/$f.sigstore.json`). Upload `dist/` as `release-set` (retention 7 days).
- [ ] **Step 4:** `github-release` job (`needs: [preflight, sign]`, `contents: write`; env `GH_TOKEN: ${{ github.token }}`, `GH_REPO: ${{ github.repository }}`, `TAG: ${{ github.ref_name }}`, `PRERELEASE: ${{ needs.preflight.outputs.prerelease }}`; no checkout needed): download `release-set` to `dist/`; `if gh release view "$TAG" >/dev/null 2>&1; then echo "release $TAG already exists; refusing to modify it" >&2; exit 1; fi`; `args=(); [[ "$PRERELEASE" == true ]] && args+=(--prerelease)`; `gh release create "$TAG" --draft --verify-tag --title "$TAG" --generate-notes "${args[@]}" dist/*`; `gh release edit "$TAG" --draft=false`; asset digests to the step summary (as today).
- [ ] **Step 5:** `images` job (`needs: [preflight, sign]`, `contents: read`, `packages: write`, matrix of 3 images from `docker.yml`): checkout (Dockerfiles are the build context); download `target-gnu-amd64` to `linux/amd64` and `target-gnu-arm64` to `linux/arm64`; buildx, ghcr login, metadata tags rewritten from `inputs.*` to `needs.preflight.outputs.version` with the push-mode rules (`v<ver>`, `<ver>`, `v{{major}}.{{minor}}`, `v{{major}}`, and `latest` with `enable=${{ needs.preflight.outputs.prerelease != 'true' }}`), build-push with `push: true`, digest to summary. Drop the `release_tag` path and the `check` tag.
- [ ] **Step 6:** `homebrew` job (`needs: [preflight, github-release]`, `if: needs.preflight.outputs.prerelease != 'true'`; env `VERSION` from preflight, `TAP_TOKEN: ${{ secrets.HOMEBREW_TAP_GITHUB_TOKEN }}`): checkout (for `packaging/homebrew-cask.sh`); download `release-set` to `release-set/`; take the sha of `iroh-tunnel-${VERSION}-darwin-arm64.tar.gz` from `release-set/checksums-darwin.txt`, render and push to `naicoi92/homebrew-tap` exactly as `promote.yml:111-131`.
- [ ] **Step 7:** `git rm` `checks.yml`, `build.yml`, `package.yml`, `docker.yml`, `snapshot.yml`, `promote.yml`, `release-smoke.yml`. Run `actionlint` and `ls .github/workflows` — Expected: no lint output; exactly `cache.yml ci.yml release.yml`. Run `grep -c 'id-token: write' .github/workflows/release.yml` — Expected: 1.
- [ ] **Step 8:** Commit: `git commit -m "ci(release): sign, verify and publish GitHub release, images and cask on tag"`

---

## Task 7: Docs — implements 5.1, 5.2, 5.3, 5.4

**Files:** Modify: `README.md:8`, `README.md:936`, `CONTRIBUTING.md:94-110`, `CHANGELOG.md` (`[Unreleased]`), `Dockerfile.release:1`, `Dockerfile.release.serve:1`, `Dockerfile.release.access:1` (comments), `tests/common/release_server.rs:1-2` (doc).

- [ ] **Step 1:** README line 8: replace the `snapshot` badge with `ci` and `release` badges (`actions/workflows/ci.yml/badge.svg`, `release.yml/badge.svg`). Line 936: `nFPM config, Homebrew cask`.
- [ ] **Step 2:** CONTRIBUTING: layout row for `.github/workflows/` = "CI (`ci`), cache warming (`cache`), release (`release`)"; `packaging/` row = "nFPM config, Homebrew cask". Replace PR item 3 with: `ci-ok` must pass; a PR runs only what its changes need (lint and Linux/Windows tests for code, shellcheck/PSScriptAnalyzer/actionlint for scripts and workflows); release artifacts are built only on tags. Add `## Releasing`: push `vX.Y.Z` (or `-rc.N`) on a merged commit on main whose PR passed `ci-ok`; the tag run publishes the GitHub release, ghcr.io images and (stable only) the Homebrew cask; if it fails in `github-release`, delete the leftover draft (`gh release delete <tag>`) and re-run all jobs; a bad release is superseded by a new tag; any change to `release.yml` ships first as an `-rc` tag; known gap: main moving between the PR's CI and the merge is not re-tested.
- [ ] **Step 3:** CHANGELOG `[Unreleased]`: `- Releases publish automatically on a version tag (GitHub release, container images, Homebrew cask); draft/promote steps are gone.`
- [ ] **Step 4:** `Dockerfile.release*` line 1: say the image packages the prebuilt release binaries from the `release` workflow. `tests/common/release_server.rs` doc: `Local HTTP server standing in for GitHub Releases, shared by the update tests.`
- [ ] **Step 5:** Commit: `git commit -m "docs: describe the three CI/CD stages and the release flow"`

---

## Task 8: Whole-change verification — implements 5.5

- [ ] **Step 1:** `actionlint` and `git ls-files -z '*.sh' | xargs -0 shellcheck --severity=warning` — Expected: no output.
- [ ] **Step 2:** `ls .github/workflows` — Expected: `cache.yml ci.yml release.yml`. `grep -l workflow_dispatch .github/workflows/*` — Expected: none.
- [ ] **Step 3:** `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, `cargo test --workspace --all-features --locked` — Expected: PASS; record wall time for the test run.
- [ ] **Step 4:** `openspec validate optimize-ci-cd --strict` — Expected: valid.
- [ ] **Step 5 (after the PR is opened, with owner permission to push):** on the PR run, confirm `lint`, `test-linux`, `test-windows`, `scripts` run in parallel, `ci-ok` is green, `test-windows` log shows `user_background_task_lifecycle ... ok` with Task Scheduler output (not the skip message), and record `test-linux` duration against the 16 min baseline.
- [ ] **Step 6 (after merge, owner action):** the first tag is `vX.Y.Z-rc.N`; confirm preflight, build, sign/verify, release (pre-release), images (no `latest`), no cask change. Only then tag stable.

---

## Scenario → test

There is no application code in this change; verification is by command, by the PR run itself, and by the first tag run. "First tag run" means the next `v*` release after merge.

**ci-cd-pipeline**
- Workflow inventory → Task 8 step 2
- Merge to main → Task 8 step 2 (no `push` trigger except `cache.yml` paths) + first merge after landing
- Code change → Task 8 step 5 (this PR changes `tests/`)
- Installer script change only → Task 3 step 2 path sets reviewed; observed on next installer-only PR
- Workflow change only → Task 3 step 2 path sets reviewed; observed on next workflow-only PR
- Docs-only change → `ci-ok` body unchanged from today (already proven on docs-only PRs, e.g. `docs/bootstrap-specs`)
- A check fails → `ci-ok` body unchanged from today
- Superseded push → `concurrency` unchanged from today
- Suite contents → Task 1 step 4
- Script lint → Task 2 step 3 + Task 8 step 1 + Task 8 step 5 (`scripts` job)
- Lifecycle runs on Windows → Task 8 step 5
- Linux unaffected → Task 3 step 3 (env only on the Windows step), reviewed in diff
- Dependency bump merged → first push to main touching `Cargo.lock`
- Source-only change merged → Task 4 step 1 paths reviewed
- Commit passed CI → Task 5 step 5 (live API check against PR #170) + first tag run
- Pre-release tag (preflight) → first `-rc` tag run summary
- Commit without a green gate → Task 5 step 2 failure branches reviewed; Task 5 step 5 with a SHA that has no PR (e.g. `2bbcba0`) returns empty `pr_head`
- Tag off main → Task 5 step 2 reviewed
- Malformed tag → Task 5 step 2 regex reviewed (`v1.2` does not match)
- Stable tag → first stable tag run, after the `-rc` run in Task 8 step 6
- Pre-release tag (release) → first `-rc` tag run
- Signature verification fails → Task 6 step 3 ordering (publish jobs `needs: sign`) reviewed
- Build target fails → `fail-fast: true` kept (Task 5 step 3)
- Release already exists → Task 6 step 4 guard reviewed
- Signing identity unchanged → Task 6 step 7 (`id-token` count = 1) + first tag run (`cosign verify-blob` passes)
- Concurrent runs → `concurrency` reviewed (Task 5 step 1)

**install-release-packaging**
- Default install, Dry run, Intel macOS, Target override, Malformed version (×2), Relative directory, Latest on Linux, Latest has no archive for the target, Checksum mismatch (×2), Release missing a role, Conflict and PATH rules, PATH consent declined, Declined conflict, Reinstall same release, No terminal, Directory already on PATH, Unwritable directory, Success leaves no staging files, Service state unchanged, Installer controls, Default flow, Unsupported platform, PATH normalization and write failure, Executable replacement fails, Declined PATH change, Accepted PATH change → not verified by CI (owner decision); `install.sh` / `install.ps1` behaviour unchanged, static lint in Task 8 step 1 / Task 2 step 3
- Tagged build, Untagged or invalid tag → `build.rs` unchanged; native smoke check in the build job
- Supported targets and naming → Task 6 step 2 expected-set check
- Archive contents → not verified by CI beyond Native smoke check
- Native smoke check → build job step kept unchanged (Task 5 step 3)
- Package targets → Task 6 step 2 expected-set check
- Exact members, Install, run, remove, Co-install → not verified by CI (owner decision)
- Runtime and promotion tags, Pre-release tag (images) → Task 6 step 5 tag rules reviewed + first tag run
- Image content → not verified by CI (owner decision)
- Signing identity and checksum names, Valid bundle → Task 6 step 3 + first tag run
- Complete set, Defective set → Task 6 step 2
- Tampered input → Task 6 step 3 (`verify-blob` failure stops the job; publish jobs need `sign`)
- Render → `packaging/homebrew-cask.sh` unchanged; first stable tag run
- Demo start → unchanged, not in scope
