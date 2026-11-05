## Why

A typical PR waits about 16 minutes, and roughly 12 of those go to two test files that exercise the installer scripts, not the application. The same suite also reruns on main to warm a cache and again on every tag. Windows e2e checks run in separate workflows outside the merge gate. Releasing takes three manual steps: draft, smoke, promote. The aim is application-only tests, one clear purpose per workflow, parallel jobs, and a tag that deploys on its own.

## What Changes

**Test scope**
- From: the PR suite tests the installer scripts, workflow and Dockerfile contents, release-set completeness and packaged smoke runs, next to the application.
- To: the PR suite tests only the application. `install.sh`, `install.ps1` and the workflows get static linting only (shellcheck, PSScriptAnalyzer, actionlint).
- Reason: auxiliary tooling cost more CI time than the product.
- Impact: non-breaking for users. Installer regressions are no longer caught by CI.

**Pull-request stage**
- From: `ci.yml` plus two separate Windows e2e workflows (x64 and ARM64) that recompile from scratch and are not part of `ci-ok`.
- To: one `ci.yml` with lint, Linux tests, Windows tests (including the real user-service lifecycle) and script lint in parallel, all behind `ci-ok`.

**Main branch**
- From: `cache-warm` reruns the full suite (15 min), and `snapshot` builds every release artifact as a rehearsal.
- To: one compile-only job writes the caches. No rehearsal build.

**Release is deployment**
- From: a tag reruns lint/test, builds, signs, verifies with the Rust test suite and uploads a DRAFT. Publishing, Homebrew and images need a manual `promote` run after a manual `release-smoke` run.
- To: a tag confirms the commit passed `ci-ok`, then builds, packages, signs, verifies the signatures with `cosign verify-blob` under the identity `update` pins, and publishes the GitHub Release, ghcr.io images and Homebrew cask in parallel.
- Impact: **no manual stop before publication**. A bad release is fixed by a new tag. The signing identity is unchanged, so `update` keeps working.

**Removed**: workflows `cache-warm`, `snapshot`, `promote`, `release-smoke`, `windows-service-e2e`, `windows-update-e2e`, `checks`; tests `installer.rs`, `installer_ps.rs`, `layout_distribution.rs`, `release_artifacts.rs`, `release_smoke.rs` and the CI-content check in `layout_cli.rs`; scripts `packaging/verify-dist.sh`, `packaging/verify-release-set.sh`.

## Capabilities

### New Capabilities
- `ci-cd-pipeline`: the stages of CI/CD (PR test gate, cache on main, tag release that deploys), what runs in each, and how the stages are kept apart.

### Modified Capabilities
- `install-release-packaging`: drafts, manual promotion, the packaged smoke test, rehearsal CI and the release-set script are replaced by automatic publication on tag. Evidence that pointed at removed tests and scripts is updated. Installer, package, image and cask behaviour is unchanged.

## Impact

- **Files**: `.github/workflows/*` (7 removed, `ci.yml`/`release.yml`/`build.yml`/`package.yml`/`docker.yml` rewritten, one cache workflow added), `tests/` (5 files removed, 1 edited), `packaging/` (2 scripts removed), `Cargo.toml` dev-dependencies if any become unused, `README.md` badge, `CONTRIBUTING.md` CI section, `tests/update_ownership.rs` doc comment.
- **Security**: keyless signing stays in `release.yml` with `id-token: write` scoped to the sign job. The certificate identity `.../release.yml@refs/tags/<tag>` must not change. Publishing permissions (`contents: write`, `packages: write`, the tap token) move into the tag workflow.
- **Compatibility**: no change to the CLI, the config, `update`, artifact names, packages or image tags. Branch protection has no required checks today; `ci-ok` stays the single check to require.
- **Non-goals**: changing release targets, artifact layout or the signing identity; new test tooling.
