# Split Unit and E2E Tests Implementation Plan

> Executed via `/opsx-apply`, which chooses the executor.
> Spec reference: this change's `specs/` (`ci-cd-pipeline`) and `design.md`.

**Goal:** Run unit tests and e2e tests as separate parallel PR jobs on Linux, macOS and Windows, and run the relay e2e suite against a self-hosted `iroh-relay` container instead of public relays.

**Architecture:** `ci.yml` gets two matrix jobs, `unit` (`--lib --bins`) and `e2e` (`--test '*'`), over three runners, all behind `ci-ok`. The Windows e2e cell keeps the Task Scheduler step; the Linux e2e cell starts `n0computer/iroh-relay:v1.4.1` with a generated certificate and runs `tests/serve_access_tunnel.rs --ignored` against it via `IROH_TUNNEL_TEST_RELAY_URL`. `cache.yml` warms the matching keys.

**Tech Stack:** GitHub Actions matrices, `Swatinem/rust-cache@v2` via `.github/actions/setup-rust`, Docker on `ubuntu-latest`, openssl, Rust test harness, `iroh 1.4.1` (`RelayMode::Custom`, `RelayMap`, `RelayUrl`).

## Global Constraints

- Unit selector, verbatim: `cargo test --workspace --all-features --locked --lib --bins`. E2E selector, verbatim: `cargo test --workspace --all-features --locked --test '*'`.
- Runners: `ubuntu-latest`, `macos-15`, `windows-latest`. Cache keys: `lint`, `unit`, `e2e`.
- Display names: `unit (linux)`, `unit (macos)`, `unit (windows)`, `e2e (linux)`, `e2e (macos)`, `e2e (windows)`. The aggregate check stays `ci-ok` (release preflight looks it up by name).
- `IROH_TUNNEL_E2E_SERVICE=1` only on the Windows Task Scheduler step.
- Relay image `n0computer/iroh-relay:v1.4.1` (must equal the `iroh` version in `Cargo.lock`; its Dockerfile at tag v1.4.1 has `ENTRYPOINT ["/iroh-relay"]` and no `USER`, so it runs as root); URL `https://127.0.0.1:3443`; env `IROH_TUNNEL_TEST_RELAY_URL`.
- Selector `run:` lines are shell-agnostic (Windows pwsh passes `'*'` literally to cargo). Relay steps set `shell: bash` and run only on Linux; the Task Scheduler step keeps `shell: pwsh`.
- No public relay is contacted in CI. No change to `src/`, `crates/`, `release.yml`, `install.*`.
- `actions/checkout` keeps `persist-credentials: false`; no `${{ }}` interpolated directly into `run:`.
- All workflow, doc and commit text in English.

## Review Focus

1. Matrix job names: `needs: [unit, e2e]` in `ci-ok` must cover every matrix cell; a cancelled cell must fail `ci-ok` (it does, `needs.*.result` is aggregated per job id).
2. Relay container not ready or wrong config key → health wait fails with logs (Task 2 step 3).
3. `test_relay_url()` set but a config writer missed → test silently uses n0; Task 1 step 2 covers all three writers (serve.toml in suite 1, `boot_access`, `MiniServe`).
4. macOS `cargo test` hitting a launchd path → no `IROH_TUNNEL_E2E_SERVICE` on macOS, the gated tests self-skip.
5. Cache key collision between unit and e2e → separate keys (Task 2 step 1, Task 3 step 1).

---

## Contracts

| Name | new / modified | Shape | public |
|---|---|---|---|
| `ci.yml` jobs | modified | `changes` → {`lint`, `unit` (matrix os ∈ linux, macos, windows), `e2e` (matrix os ∈ linux, macos, windows), `scripts`} → `ci-ok` | no |
| `ci-ok` check | unchanged name | job id/name `ci-ok`; `needs: [changes, lint, unit, e2e, scripts]` | yes — release preflight depends on the name |
| `cache.yml` jobs | modified | `lint` (ubuntu, key `lint`); `build` matrix `kind ∈ {unit, e2e}` × `os ∈ {ubuntu-latest, macos-15, windows-latest}`, key = kind | no |
| `IROH_TUNNEL_TEST_RELAY_URL` | new | env var, `https://…` relay URL; unset = previous behaviour | no (test-only) |
| `fn test_relay_url() -> Option<String>` | new | in `tests/serve_access_tunnel.rs`; returns `Some(url)` when the env var is set and non-empty | no |
| `fn relay_urls_toml() -> String` | new | in `tests/serve_access_tunnel.rs`; `relay_urls = ["<url>"]\n` when set, empty string otherwise | no |
| Matrix keys | new | `unit`/`e2e`: `include: [{name: linux, os: ubuntu-latest}, {name: macos, os: macos-15}, {name: windows, os: windows-latest}]`, job `name: unit (${{ matrix.name }})` / `e2e (${{ matrix.name }})` | no |

---

## Task 1: Relay URL from the environment — implements 1.1, 1.2, 1.3, 1.4

**Files:** Modify: `tests/serve_access_tunnel.rs` (module doc lines 1-35; serve config writer ~line 83; `MiniServe::start` ~line 218; `boot_access` ~line 293).

**Interfaces:** Produces contract `IROH_TUNNEL_TEST_RELAY_URL`, `test_relay_url`.

- [ ] **Step 1:** Add `fn test_relay_url() -> Option<String>`: `std::env::var("IROH_TUNNEL_TEST_RELAY_URL").ok().filter(|s| !s.is_empty())`, and `fn relay_urls_toml() -> String` returning `relay_urls = ["<url>"]\n` (or empty string when unset).
- [ ] **Step 2:** In `serve_access_tunnel_roundtrips_bytes`, insert `relay_urls_toml()` under `[node]` in serve.toml, and add a `[node]` section with it to access.toml (only when set). In `boot_access`, prepend `[node]\n{relay_urls_toml()}` when set. In `MiniServe::start`, when set: `.relay_mode(RelayMode::Custom(RelayMap::from(url.parse::<iroh::RelayUrl>()?)))` **and** `.ca_tls_config(iroh::tls::CaTlsConfig::insecure_skip_verify())` (the bare test endpoint does not get the engine's `test-utils` TLS bypass from `crates/engine/src/endpoint/create.rs`, so without this it rejects the self-signed relay); when unset, keep `RelayMode::Default` and no TLS change.
- [ ] **Step 3:** Module doc: replace the "Why `#[ignore]`" paragraph with: ignored because it needs a real relay; run by hand against n0 with `cargo test --test serve_access_tunnel -- --ignored`, or against a self-hosted relay with `IROH_TUNNEL_TEST_RELAY_URL=https://127.0.0.1:3443 cargo test --workspace --all-features --locked --test serve_access_tunnel -- --ignored --test-threads=1` (CI does the latter, `e2e (linux)`). Note that serve/access (engine `test-utils`) and the test's own `MiniServe` endpoint (explicit `insecure_skip_verify` when the variable is set) skip relay TLS verification, so only a local, self-hosted relay may be named by the variable.
- [ ] **Step 4:** Run `cargo test --all-features --locked --test serve_access_tunnel` — Expected: `0 passed; 0 failed; 4 ignored`. Run `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` and `cargo fmt --all -- --check` — Expected: clean.
- [ ] **Step 5:** Commit: `test(e2e): take the relay URL for serve_access_tunnel from the environment`

---

## Task 2: PR jobs — implements 2.1, 2.2, 2.3, 2.4, 2.5

**Files:** Modify: `.github/workflows/ci.yml`.

**Interfaces:** Consumes Task 1 contract. Produces contracts `ci.yml jobs`, `ci-ok check`.

- [ ] **Step 1:** Replace `test-linux` and `test-windows` with:
  - `unit`: `name: unit (${{ matrix.name }})`, `needs: changes`, `if: code`, `strategy.fail-fast: false`, matrix `include: [{name: linux, os: ubuntu-latest}, {name: macos, os: macos-15}, {name: windows, os: windows-latest}]`, `runs-on: ${{ matrix.os }}`, checkout, setup-rust `cache-key: unit`, run the unit selector, then a second step `cargo test --workspace --all-features --locked --doc`.
  - `e2e`: same shape (`fail-fast: false`), `cache-key: e2e`, run the e2e selector.
- [ ] **Step 2:** In `e2e`, add step `Service lifecycle (Task Scheduler)` with `if: matrix.name == 'windows'`, command and env exactly as today's `test-windows` step.
- [ ] **Step 3:** In `e2e`, add Linux-only steps (`if: ${{ !cancelled() && matrix.name == 'linux' }}`, so they run even when the e2e suite failed; if checkout or toolchain setup failed they fail fast too, accepted), `shell: bash` with `set -euo pipefail`:
  - `Start self-hosted relay`: generate cert into `$RUNNER_TEMP/relay/certs` (`default.crt`, `default.key`, SAN `DNS:localhost,IP:127.0.0.1`, 1 day); `chmod 755` the dir and `chmod 644` the cert, key and config (throwaway key; readable regardless of container uid); write `$RUNNER_TEMP/relay/config.toml` with the D4 keys but bind addresses `0.0.0.0:3340` (http) and `0.0.0.0:3443` (https); `docker run -d --name iroh-relay -p 127.0.0.1:3443:3443 -v "$RUNNER_TEMP/relay/certs:/certs:ro" -v "$RUNNER_TEMP/relay/config.toml:/config.toml:ro" n0computer/iroh-relay:v1.4.1 --config-path /config.toml`; wait loop: `ok=0; for i in $(seq 1 30); do [ "$(docker inspect -f '{{.State.Running}}' iroh-relay)" = true ] || break; if curl -fsk https://127.0.0.1:3443/healthz >/dev/null; then ok=1; break; fi; sleep 2; done; [ "$ok" = 1 ] || { docker ps -a; docker logs iroh-relay; exit 1; }`. Comment above: image tag must match the `iroh` version in `Cargo.lock`.
  - `Relay e2e (self-hosted relay)`: `env: IROH_TUNNEL_TEST_RELAY_URL: https://127.0.0.1:3443`; run `cargo test --workspace --all-features --locked --test serve_access_tunnel -- --ignored --test-threads=1` (same package selection as the e2e step, so the build is reused).
  - `Stop relay`: `if: always() && matrix.name == 'linux'`; `docker logs iroh-relay || true; docker rm -f iroh-relay || true`.
- [ ] **Step 4:** `ci-ok.needs: [changes, lint, unit, e2e, scripts]`; body byte-identical. Header diagram lists `unit (linux|macos|windows)` and `e2e (linux|macos|windows)` and the relay and Task Scheduler extras.
- [ ] **Step 5:** Run `actionlint` — Expected: no output. Extract the relay start script to a temp file and `shellcheck -s bash` it — Expected: no output.
- [ ] **Step 6:** Commit: `ci: separate unit and e2e jobs on Linux, macOS and Windows`

---

## Task 3: Cache stage — implements 3.1, 3.2

**Files:** Modify: `.github/workflows/cache.yml`.

**Interfaces:** Consumes keys `unit`, `e2e` from Task 2.

- [ ] **Step 1:** Keep `lint` unchanged. Replace `test` with `build`: `name: cache (${{ matrix.kind }}, ${{ matrix.os }})`, matrix `kind: [unit, e2e]`, `os: [ubuntu-latest, macos-15, windows-latest]`, `fail-fast: false`; setup-rust `cache-key: ${{ matrix.kind }}`; one step per kind with `if: matrix.kind == 'unit'` → `cargo test --workspace --all-features --locked --lib --bins --no-run` (doctests are compiled at test time and need no warm-up), `if: matrix.kind == 'e2e'` → `cargo test --workspace --all-features --locked --test '*' --no-run`. Update the header comment.
- [ ] **Step 2:** Run `actionlint` — Expected: no output.
- [ ] **Step 3:** Commit: `ci: warm unit and e2e caches on Linux, macOS and Windows`

---

## Task 4: Docs and verification — implements 4.1, 4.2, 4.3, 4.4

**Files:** Modify: `CONTRIBUTING.md` (CI paragraph near line 51 and the PR section), `CHANGELOG.md` (`[Unreleased]`).

- [ ] **Step 1:** CONTRIBUTING: the CI paragraph names `unit` and `e2e` jobs on Linux, macOS and Windows; add a short "Running tests" block with the unit selector (+ `--doc`), the e2e selector, the self-hosted relay command from Task 1 step 3 (plus a pointer to `docs/self-hosted-relay.md`), and the coverage check: the unit and e2e passed counts must add up to the full `cargo test --workspace` count (rule: new tests go in `src/` for unit or `tests/` for e2e).
- [ ] **Step 2:** CHANGELOG `[Unreleased]`: `- CI runs unit and end-to-end tests as separate jobs on Linux, macOS and Windows; the relay end-to-end suite runs against a self-hosted iroh-relay in CI.`
- [ ] **Step 3:** Verify locally: run the unit selector and the e2e selector and the full `cargo test --workspace --all-features --locked`; Expected: unit passed + e2e passed = full passed (today 324 + 174 = 498). `cargo fmt --all -- --check`, clippy `-D warnings`, `actionlint`, `git ls-files -z '*.sh' | xargs -0 shellcheck --severity=warning`, `openspec validate split-unit-e2e-tests --strict` — all clean.
- [ ] **Step 4 (needs owner permission to push):** On the PR run: six test jobs present and parallel; `e2e (windows)` log contains `user_background_task_lifecycle ... ok`; `e2e (linux)` shows the relay healthy and `test result: ok. 4 passed` for `serve_access_tunnel`; `ci-ok` green.
- [ ] **Step 5:** Before asking to merge: confirm the repository requires only `ci-ok` (no ruleset or branch protection names `test (linux)`/`test (windows)`): `gh api repos/naicoi92/iroh-tunnel/rulesets` and `gh api repos/naicoi92/iroh-tunnel/branches/main/protection` (404 = none).
- [ ] **Step 6:** Commit: `docs: describe the unit and e2e test jobs`

---

## Scenario → test

- Code change → Task 4 step 4 (PR run, this PR changes `tests/`)
- Installer script change only → unchanged path filters; observed on next installer-only PR
- Workflow change only → unchanged path filters
- Docs-only change → `ci-ok` body unchanged
- A check fails → `ci-ok` body unchanged; `needs` includes `unit`, `e2e`
- Superseded push → `concurrency` unchanged
- Lifecycle runs on Windows → Task 4 step 4 (`e2e (windows)` log)
- Linux unaffected → Task 2 step 2 (`if: matrix.name == 'windows'`), reviewed in diff
- Dependency bump merged → first push to main touching `Cargo.lock` / `cache.yml` (this change touches `cache.yml`, so the merge itself)
- Source-only change merged → `cache.yml` paths unchanged
- Unit failure reported early → Task 2 step 1 (separate `unit` job, `fail-fast: false`), reviewed in diff; not exercised by a deliberately failing test (recorded as not verified at runtime)
- New integration test file → `--test '*'` selector, reviewed in diff
- Exact coverage → Task 4 step 3 (counts)
- Doctest added → Task 2 step 1 (`--doc` step in `unit`), reviewed in diff
- Relay suite in CI → Task 4 step 4 (`e2e (linux)` log)
- Relay does not start → Task 2 step 3 (health wait with logs), reviewed in diff; not exercised by a failing run (recorded as not verified at runtime)
- Local run without a relay → Task 1 step 4 (`4 ignored`)

---

## Task 5: Revision — e2e on main, unit-only PRs, release gate — implements 5.1, 5.2, 5.3, 5.4, 5.5, 5.6

Owner revision after PR #173's first run (design D1 revision note, D5, D7, D8). The e2e job body proven on run 42039703639 moves unchanged.

**Files:** Create `.github/workflows/e2e.yml`. Modify `.github/workflows/ci.yml`, `.github/workflows/cache.yml`, `.github/workflows/release.yml` (preflight step only), `CONTRIBUTING.md`, `CHANGELOG.md`.

**Contracts (additions):**

| Name | new / modified | Shape | public |
|---|---|---|---|
| `e2e.yml` | new | `on: push: branches: [main]` (no paths); `concurrency: { group: e2e-${{ github.sha }}, cancel-in-progress: false }`; `permissions: contents: read`; jobs `e2e` (matrix as before) → `e2e-ok` | no |
| `e2e-ok` check | new | job id/name `e2e-ok`; `if: always()`; `needs: [e2e]`; body identical to `ci-ok` | yes — release preflight depends on the name |
| `ci-ok` check | modified | `needs: [changes, lint, unit, scripts]` | yes |
| release preflight | modified | adds `e2e-ok` latest-conclusion check on `$GITHUB_SHA` | no |

- [ ] **Step 1:** Create `e2e.yml` with the header (what runs, why on main, that release preflight requires `e2e-ok`, tag only after it is green). Cut the whole `e2e:` job from `ci.yml` and paste it under `jobs:` unchanged except: no `needs: changes` and no `if: needs.changes...` (main has no path filter). Add `e2e-ok` (ubuntu-latest, `if: always()`, `needs: [e2e]`, steps copied byte-for-byte from `ci-ok`).
- [ ] **Step 2:** `ci.yml`: `ci-ok.needs: [changes, lint, unit, scripts]`; header diagram drops e2e and says e2e runs on main in `e2e.yml`.
- [ ] **Step 3:** `cache.yml`: replace the `build` kind×os matrix with `unit` over `os: [ubuntu-latest, macos-15, windows-latest]`, key `unit`, `cargo test --workspace --all-features --locked --lib --bins --no-run`; header says e2e caches are saved by `e2e.yml`.
- [ ] **Step 4:** `release.yml` preflight, after the `ci-ok` check: `e2e=$(gh api "repos/$GITHUB_REPOSITORY/commits/$GITHUB_SHA/check-runs?check_name=e2e-ok" --jq '[.check_runs[]] | sort_by(.completed_at) | last | .conclusion // empty')`; fail unless `success` with `e2e-ok on $GITHUB_SHA is '${e2e:-missing}', expected success (tag after e2e.yml is green on main)`. Rename the step to `Check tag, main ancestry, ci-ok and e2e-ok`; summary line mentions both. Header comment: preflight requires `ci-ok` and `e2e-ok`.
- [ ] **Step 5:** CONTRIBUTING: CI paragraph says PRs run `lint`, `unit (linux|macos|windows)` and `scripts`; e2e runs on every push to `main` (`e2e.yml`) and releases require its `e2e-ok`; "Releasing" says tag only after `e2e-ok` is green on the commit (otherwise preflight fails; re-run the release run once it is green). CHANGELOG line: PRs run unit tests; e2e runs on `main` and gates releases.
- [ ] **Step 6:** `actionlint` clean; extract and `shellcheck -s bash` the preflight script; `ls .github/workflows` = `cache.yml ci.yml e2e.yml release.yml`; dry-run the `e2e-ok` query against a commit without it (expect empty → fail branch) — `gh api "repos/naicoi92/iroh-tunnel/commits/$(git rev-parse main)/check-runs?check_name=e2e-ok"`.
- [ ] **Step 7:** Commit: `ci: run e2e on main and gate releases on e2e-ok; PRs run unit tests only`

Scenario → test (Task 5): Workflow inventory → step 6; Merge to main / Merge runs e2e / Docs-only merge / Back-to-back merges → first merge after landing (trigger and concurrency reviewed in diff); E2E red or missing on the tagged commit → step 6 dry-run (missing) + step 4 reviewed; Commit passed CI → first tag after merge; Code change / A check fails → PR run (no e2e jobs, `ci-ok` green); Lifecycle runs on Windows / Relay suite in CI / Relay does not start → job body unchanged from run 42039703639 (proven), re-proven on the first push to `main`.
