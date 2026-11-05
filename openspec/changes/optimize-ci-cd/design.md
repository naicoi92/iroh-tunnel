## Context

Today there are 13 workflows in `.github/workflows/` (see brainstorm for the measured baseline). Several are reusable workflows called from more than one place (`checks`, `build`, `package`, `docker`), and each caller gives them a different role: PR gate, cache warmer, release gate, rehearsal or promotion. `release.yml` signs keyless, and the certificate identity `https://github.com/naicoi92/iroh-tunnel/.github/workflows/release.yml@refs/tags/<tag>` with issuer `https://token.actions.githubusercontent.com` is pinned by the shipped updater (`crates/host/src/update/`). Signing must therefore stay a job of `release.yml` itself, never of a called workflow. The default branch has no protection rules and no required checks. PRs merge with merge commits (for example `adffb73 Merge pull request #170`).

There is no application code in scope. The DDD boundaries of the product stay as they are. The only "domain" here is the delivery pipeline, and its invariants are listed under Decisions.

## Goals / Non-Goals

**Goals:**
- Three workflows, one per stage: `ci.yml` (PR), `cache.yml` (main), `release.yml` (tag, ends in deployment).
- No job in more than one stage. Independent jobs in a stage run in parallel.
- Tests only for the application. Tooling gets static lint only.
- The release signing identity stays byte-for-byte the same.

**Non-Goals:**
- Changing targets, archive or package names, image tags or the cask.
- Rewriting `install.sh` / `install.ps1` or any Rust source.
- New test runners or partitioning.

## Decisions

### D1. Stage layout and workflow files

```
ci.yml       on: pull_request
  changes ─┬─ lint           ubuntu   fmt --check + clippy -D warnings     (code)
           ├─ test-linux     ubuntu   cargo test --workspace --all-features (code)
           ├─ test-windows   windows  cargo test ... then service lifecycle (code)
           ├─ scripts        ubuntu   shellcheck + PSScriptAnalyzer + actionlint (scripts)
           └─ ci-ok          always   fails on any failure/cancelled

cache.yml    on: push main (Cargo.toml, crates/*/Cargo.toml, Cargo.lock,
                            .github/actions/setup-rust/**, cache.yml)
  lint-cache ∥ test-cache (ubuntu, windows): compile only, save cache

release.yml  on: push tags v*
  preflight ─ build (8 targets ∥) ─┬─ package ─┐
                                   └───────────┴─ sign+verify ─┬─ github-release
                                                               ├─ images (3 ∥)
                                                               └─ homebrew (stable only)
```

`checks.yml`, `cache-warm.yml`, `snapshot.yml`, `promote.yml`, `release-smoke.yml`, `windows-service-e2e.yml` and `windows-update-e2e.yml` are deleted. `build.yml`, `package.yml` and `docker.yml` are inlined into `release.yml` as jobs.
- **Why inline instead of keeping reusable workflows**: each of them now has exactly one caller. Keeping them separate files would only keep the indirection that made their roles blur. Inlining also keeps every job of the signed run in the file whose name is the signing identity.
- **Alternative**: keep `build.yml` reusable for a future manual rehearsal. Rejected because the owner removed rehearsal.

`lint` and the test jobs share no state, so they run in parallel. The `changes` path filter (`dorny/paths-filter@v3`) stays, with two outputs: `code` (`src/**`, `crates/**`, `tests/**`, `examples/**`, `Cargo.toml`, `Cargo.lock`, `build.rs`, `.github/actions/**`, `.github/workflows/ci.yml`) and `scripts` (`**/*.sh`, `install.ps1`, `.github/**`). The installer scripts are no longer in `code`: no Rust test reads them any more.

### D2. Windows test job carries the service lifecycle

`test-windows` runs `cargo test --workspace --all-features --locked` and then, in the same job and with the same build, `cargo test --all-features --locked --test service_user_windows -- --test-threads=1` with `IROH_TUNNEL_E2E_SERVICE=1`. The second step only relinks, so it adds about a minute. The env var is set only on that Windows step. On Linux the same variable would drive a real `systemd --user` in `tests/service_user.rs`, and the hosted runner does not provide one reliably.
- Dropped: the ARM64 Windows legs, the standard-user Task Scheduler measurement (SM-04 evidence, not a pass/fail check), and the x64 copy of `update_zip_windows`, which already runs in the normal suite.

### D3. Cache workflow compiles, never runs tests

`cache.yml` runs `cargo clippy --workspace --all-targets --all-features --locked` under cache key `lint`, and `cargo test --workspace --all-features --locked --no-run` under cache key `test` (ubuntu and windows). These are the same keys and flags as the PR jobs, so PRs restore a warm `target/`. `setup-rust` already saves only on `refs/heads/main`. Nothing else changes in `.github/actions/setup-rust`.

### D4. Release preflight: the tagged commit must have passed `ci-ok`

There is no push-to-main CI run, so the evidence is the PR run. Preflight:
1. Fails unless the tag matches `^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$`, and the tagged commit is reachable from `origin/main` (`git merge-base --is-ancestor`).
2. Resolves the PR that introduced the commit with `GET /repos/{repo}/commits/{sha}/pulls`. For a merge commit this returns the merged PR. It then requires a check run named `ci-ok` with conclusion `success` on that PR's head SHA (`GET /repos/{repo}/commits/{head}/check-runs?check_name=ci-ok`). A commit with no associated PR, or no successful `ci-ok`, fails the release.
3. Outputs `version`, which is the tag without its `v`, and `prerelease` (`true` when the version contains `-`).
- **Why the PR head SHA**: the merge commit itself never ran CI, because `ci.yml` is `pull_request` only. GitHub's merge commit for a PR equals the tree that was tested, as long as main had not moved. Main moving between the CI run and the merge is the accepted gap. Branch protection with "require branches to be up to date" closes it, and can be turned on later without changing this design.
- **Alternative**: rerun lint/test on the tag. Rejected by the owner as redundant.
- **Alternative**: add a push-to-main CI run and check that. Rejected: it reruns the suite on main, which is the redundancy being removed.

Preflight needs `contents: read` and `checks: read`, with `pull-requests: read` for the commit-to-PR lookup.

### D5. Build job keeps its current logic, minus the version job

The 8-target matrix, the toolchain setup, the smoke `--version`/`--help` checks on native runners and the archive steps move unchanged into `release.yml`. The version comes from preflight. The `git describe` dependency in `build.rs` stays, so checkout keeps `fetch-depth: 0`. The smoke step tests the shipped binaries, so it is application testing and stays. Artifact retention is 7 days so "Re-run failed jobs" works within a week.

Release builds run with a cold `build-<id>` cache. `setup-rust` saves only on main, and nothing on main builds release targets any more. This is accepted: a release-profile LTO build of this workspace takes a few minutes per target and runs in parallel, and warming eight release caches on every dependency bump would be the redundant work this change removes. `build-<id>` keys are dropped; the job uses `cache-key: release-<id>` only so the key is distinct, and it never saves.

### D6. Package job without the verification script

nFPM packaging stays as it is. The steps that call `packaging/verify-dist.sh` are removed, and so is the script. The packages are uploaded as a run artifact for sign.

### D7. Sign and verify in one job

The `sign` job (the only job with `id-token: write`) collects archives and packages, writes `checksums.txt` and `checksums-darwin.txt` as today, signs both with `cosign sign-blob --new-bundle-format`, and then verifies each bundle in the same job:

```
cosign verify-blob --bundle <f>.sigstore.json \
  --certificate-identity "https://github.com/${GITHUB_REPOSITORY}/.github/workflows/release.yml@${GITHUB_REF}" \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com <f>
```

Before writing the checksums it checks the expected set inline: exactly 3 roles × 8 targets archives, 3 × 2 `.deb`, 3 × 3 `.apk`, by explicit name built from version, target and role, and no other file. After signing it checks that each bundle is non-empty and runs `sha256sum -c` on both checksum files. This is a few lines of bash in the job, not a test suite. This replaces `packaging/verify-release-set.sh` and the Rust verifier tests in `tests/release_artifacts.rs`, which are deleted.
- **Why cosign instead of the shipped verifier**: same pinned identity and issuer, seconds instead of a full compile. The Rust verifier's own logic is covered by the unit tests in `crates/host/src/update/`, which run on every PR.

### D8. Publish jobs run in parallel after sign

- `github-release` (`contents: write`): `gh release create "$TAG" --draft --verify-tag --title "$TAG" --generate-notes [--prerelease] <all assets>`, then `gh release edit "$TAG" --draft=false`. The draft exists for seconds inside one job; it is never a manual step. A half-uploaded release is therefore never public, and `releases/latest/download/*` (used by the installers and `update`) never points at a release with missing assets. If a release (draft or published) for the tag already exists, the job fails without touching it. Recovery after a failure in this job is to delete the leftover draft and re-run all jobs, which is documented in CONTRIBUTING.
- `images` (`packages: write`, 3-image matrix): runs after `sign` and uses the `target-gnu-amd64` / `target-gnu-arm64` run artifacts directly. These are the same bytes that were checksummed and signed. Tags are as today: `v<ver>`, `<ver>`, `v<major>.<minor>`, `v<major>`, plus `latest` for stable versions. Buildx and metadata steps are kept. The download-from-release path (`release_tag` input) goes away.
- `homebrew` (only when `prerelease == false`): renders the cask with `packaging/homebrew-cask.sh` from the darwin checksum in the `release-set` artifact and pushes it to `naicoi92/homebrew-tap` with `HOMEBREW_TAP_GITHUB_TOKEN`. It needs `github-release`, so the cask never points at a release that does not exist.

`concurrency: release-${{ github.ref }}`, `cancel-in-progress: false` stays. `workflow_dispatch` is removed from `release.yml`. A release is started only by a tag push, so `GITHUB_REF` is always `refs/tags/<tag>` and the identity is well-defined.

### D9. Static lint for the installer scripts

The `scripts` job runs `shellcheck --severity=warning` on every tracked `*.sh` (as today), actionlint 1.7.7 (as today), and `Invoke-ScriptAnalyzer -Path install.ps1 -Severity Warning,Error -EnableExit` in `pwsh`, which is preinstalled on ubuntu-latest. PSScriptAnalyzer is installed with `Install-Module PSScriptAnalyzer -Scope CurrentUser -Force`. Any existing findings in `install.ps1` are fixed or suppressed per rule with `[Diagnostics.CodeAnalysis.SuppressMessageAttribute]` and a reason. This is the only edit allowed in `install.ps1`.

### D10. Test removals and their dependencies

Behaviour previously checked only by `packaging/verify-dist.sh` (package members, install/remove in containers, image contents, binary formats) stays in the spec as product behaviour but is no longer verified by CI. The spec marks those requirements accordingly.


Deleted: `tests/installer.rs`, `tests/installer_ps.rs`, `tests/layout_distribution.rs`, `tests/release_artifacts.rs`, `tests/release_smoke.rs`. Edited: `tests/layout_cli.rs` loses `ci_build_workflow_reads_workspace_version`, because it asserts on workflow text. `tests/common/release_server.rs` stays, since `update*.rs` uses it. All dev-dependencies stay: `sha2` is still used by `update*.rs`. `tests/update_ownership.rs` keeps its tests, and only the doc comment that cites `verify-dist.sh` changes.

## Risks / Trade-offs

- [No manual stop before publication] → Preflight refuses commits without a green `ci-ok`, and build smoke-tests every native binary. A bad release is superseded by a new tag, and the CONTRIBUTING release section says so.
- [Signing identity drift breaks `update` for every user] → The identity string in the verify step is built from the same `GITHUB_REF`, and verify runs before any publish job. A mismatch fails the run before anything is public.
- [Main moved between PR CI and merge, so the merged tree was never tested] → Accepted. Can be closed later by turning on branch protection "require up-to-date", with no change to this design.
- [Installer, package or image regressions reach users] → Owner-accepted. Static lint catches syntax and common mistakes; the expected-set check catches missing or extra artifacts.
- [Partial GitHub release on upload failure] → Fail on an existing release, documented manual recovery (delete, re-run).
- [Windows service lifecycle test leaves a task on the runner] → It already removes its task on exit and uses a unique suffix. Hosted runners are ephemeral.
- [No rehearsal of `release.yml` before a real tag] → first tag after merge is an `-rc` (Migration Plan step 3); CONTRIBUTING says every release-workflow change ships first as an `-rc`.
- [Cold release builds] → accepted (D5); parallel matrix keeps wall clock bounded by the slowest target.
- [PSScriptAnalyzer download adds time or flakiness] → One module install of a few seconds. If it proves flaky, cache the module path.

## Migration Plan

1. Land everything in one PR from a worktree branch. The PR itself exercises the new `ci.yml`.
2. After merge, `cache.yml` runs once, triggered by its own path, and seeds the caches.
3. The first tag after merge MUST be a pre-release (`vX.Y.Z-rc.N`). It runs the new `release.yml` end to end, except the `latest` image tag and the cask, so a pipeline bug does not reach stable users. The first stable tag follows only after that run is green. Rollback means reverting the merge. Old drafts made by the previous pipeline (for example `v0.4.3-rc.3`, if still a draft) are published or deleted by hand. They are not migrated.

## Open Questions

None.
