Single writer, single worktree. Run in order 1 → 4. Platforms: Linux, macOS and Windows (PR); Linux only for the relay container.

## 1. Relay URL from the environment (`tests/serve_access_tunnel.rs`)

Owns: `tests/serve_access_tunnel.rs`.
Scenarios: "Local run without a relay", "Relay suite in CI" (test side).

- [x] 1.1 Add `fn test_relay_url() -> Option<String>` reading `IROH_TUNNEL_TEST_RELAY_URL` (design D3)
- [x] 1.2 When set, write `[node] relay_urls = ["<url>"]` into every generated serve and access config, and build `MiniServe` with `RelayMode::Custom` from that URL plus `CaTlsConfig::insecure_skip_verify()` (it bypasses the engine); when unset, keep today's behaviour exactly
- [x] 1.3 Keep every `#[ignore]`; update the module doc to describe both runs (public relays by hand, self-hosted relay via the variable)
- [x] 1.4 Verify: `cargo test --all-features --locked --test serve_access_tunnel` reports 4 ignored, 0 run; `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` clean

## 2. PR jobs (`.github/workflows/ci.yml`)

Owns: `.github/workflows/ci.yml`.
Scenarios: "Code change", "A check fails", "Unit failure reported early", "New integration test file", "Exact coverage", "Doctest added", "Lifecycle runs on Windows", "Linux unaffected", "Relay suite in CI", "Relay does not start"; unchanged: "Installer script change only", "Workflow change only", "Docs-only change", "Superseded push".

- [x] 2.1 Replace `test-linux` / `test-windows` with `unit` and `e2e` jobs, each a matrix over Linux (`ubuntu-latest`), macOS (`macos-15`) and Windows (`windows-latest`), selectors (unit adds a `--doc` step), `fail-fast: false` and cache keys per design D1
- [x] 2.2 Move the Task Scheduler step (unchanged command and env) into the Windows cell of `e2e`
- [x] 2.3 Add the self-hosted relay steps to the Linux cell of `e2e` (certificate, config, `docker run n0computer/iroh-relay:v1.4.1`, health wait, `--ignored` run with `IROH_TUNNEL_TEST_RELAY_URL`, always-logs-and-remove; run even if the e2e suite failed) per design D4, with a comment tying the image tag to the `iroh` version in `Cargo.lock`
- [x] 2.4 Update `ci-ok.needs` to `[changes, lint, unit, e2e, scripts]` (body unchanged) and the header diagram
- [x] 2.5 Verify: `actionlint` clean; extract the relay shell steps and `shellcheck -s bash` them

## 3. Cache stage (`.github/workflows/cache.yml`)

Owns: `.github/workflows/cache.yml`.
Scenarios: "Dependency bump merged", "Source-only change merged".

- [x] 3.1 Keep `lint`; replace `test` with a matrix `{kind: unit, e2e} × {os: ubuntu-latest, macos-15, windows-latest}` compiling the matching selector with `--no-run` under key `unit` / `e2e` (design D5); paths unchanged
- [x] 3.2 Verify: `actionlint` clean

## 4. Docs and whole-change verification

Owns: `CONTRIBUTING.md`, `CHANGELOG.md`.
Scenarios: "Exact coverage" (local evidence).

- [x] 4.1 CONTRIBUTING: describe the unit / e2e split, the local commands for each, and how to run the relay suite against a self-hosted relay (`IROH_TUNNEL_TEST_RELAY_URL=https://127.0.0.1:3443 cargo test --workspace --all-features --locked --test serve_access_tunnel -- --ignored --test-threads=1`)
- [x] 4.2 CHANGELOG `[Unreleased]`: one line on unit/e2e jobs, macOS coverage and self-hosted relay e2e
- [x] 4.3 Verify locally: unit selector passed count + e2e selector passed count = full `cargo test --workspace --all-features --locked` passed count; `cargo fmt --check`, clippy, actionlint, shellcheck clean; `openspec validate split-unit-e2e-tests --strict`
- [x] 4.4 Verify on the PR run (needs owner permission to push): six test jobs run in parallel, `e2e (windows)` log shows `user_background_task_lifecycle ... ok`, `e2e (linux)` relay step shows the relay healthy and `4 passed` for `serve_access_tunnel`

## 5. Revision: e2e on main, unit-only PRs, release gate (owner, after PR #173 run)

Owns: `.github/workflows/ci.yml`, `.github/workflows/e2e.yml` (new), `.github/workflows/cache.yml`, `.github/workflows/release.yml` (preflight step only), `CONTRIBUTING.md`, `CHANGELOG.md`.
Scenarios: "Workflow inventory", "Merge to main", "Code change", "A check fails", "Lifecycle runs on Windows", "Dependency bump merged", "E2E red or missing on the tagged commit", "Commit passed CI", "Merge runs e2e", "Docs-only merge", "Back-to-back merges", "Relay suite in CI", "Relay does not start", "New integration test file".

- [ ] 5.1 Create `e2e.yml` (design D7): move the `e2e` matrix job from `ci.yml` unchanged (selectors, Task Scheduler step, relay steps), add `e2e-ok` aggregate; trigger push to `main` without paths; concurrency per commit, no cancel
- [ ] 5.2 `ci.yml`: remove the `e2e` job; `ci-ok.needs: [changes, lint, unit, scripts]` (body unchanged); update header
- [ ] 5.3 `cache.yml`: `unit` matrix only (design D5)
- [ ] 5.4 `release.yml` preflight: also require the most recent `e2e-ok` on `$GITHUB_SHA` to be `success` (design D8); update the header and the step summary line
- [ ] 5.5 CONTRIBUTING/CHANGELOG: PRs run unit only, e2e runs on `main`, tag only after `e2e-ok` is green on the commit
- [ ] 5.6 Verify: actionlint, shellcheck (preflight + relay scripts), `openspec validate --strict`; live `gh api` dry-check of the `e2e-ok` query; on the PR run: `ci-ok` green with only `unit`/`lint`/`scripts`, no e2e jobs
