## Context

After `optimize-ci-cd`, `.github/workflows/ci.yml` has `test-linux` and `test-windows`. Both run `cargo test --workspace --all-features --locked`, and `test-windows` adds a Task Scheduler step (`IROH_TUNNEL_E2E_SERVICE=1 cargo test --all-features --locked --test service_user_windows -- --nocapture --test-threads=1`). `cache.yml` warms cache keys `lint` (ubuntu) and `test` (ubuntu, windows) through `.github/actions/setup-rust`. That action uses `Swatinem/rust-cache@v2` with `shared-key: <cache-key>` and saves only on `main`; rust-cache adds the runner OS to the key.

Measured locally: `--lib --bins` runs 324 tests, and `--test '*'` runs 174 tests plus 4 ignored. Together that is the 498 of the full run. There are no doctests.

`tests/serve_access_tunnel.rs` (4 tests, all `#[ignore]`) runs serve and access in-process via `iroh_tunnel_host::testing::run_unowned`. It writes `serve.toml` and `access.toml` with no `relay_urls`, so both fall back to the n0 relays. Its `MiniServe` helper builds an `iroh::Endpoint` with `RelayMode::Default`. Every `tests/` binary is built with the `test-utils` feature, which makes `crates/engine/src/endpoint/create.rs:32-35` use `CaTlsConfig::insecure_skip_verify()`, so a self-signed relay certificate is accepted. Config validation requires `https://` relay URLs (`crates/host/src/config/dto.rs:480,506`).

The official image `n0computer/iroh-relay` publishes `v1.4.1`, matching `iroh 1.4.1` in `Cargo.lock`. In iroh-relay 1.4.1, `cert_mode = "Manual"` reads `default.crt` and `default.key` from `cert_dir`. `--dev` serves plain HTTP only, which config validation rejects. Docker is not available on the development machine, so the container path can only be verified on CI.

No product code is in scope. The only test-code change is in `tests/serve_access_tunnel.rs`.

## Goals / Non-Goals

**Goals:**
- PRs run unit tests only, on Linux, macOS and Windows.
- Every push to `main` runs e2e on Linux, macOS and Windows, and the result gates releases.
- Exact coverage: unit plus e2e equals the current suite, with nothing run twice and nothing dropped.
- Relay e2e runs on every push to `main` against a relay the job owns.

**Non-Goals:**
- Moving tests between `src/` and `tests/`.
- Changing release build, sign or publish (only preflight changes, D8).
- Relay e2e on macOS or Windows; GitHub Docker services exist only on Linux runners.

## Decisions

### D1. Selectors and job names

| Job | Runner | Command | Cache key |
|---|---|---|---|
| `unit-linux` | ubuntu-latest | `cargo test --workspace --all-features --locked --lib --bins` | `unit` |
| `unit-macos` | macos-15 | same | `unit` |
| `unit-windows` | windows-latest | same | `unit` |
| `e2e-linux` (in `e2e.yml`) | ubuntu-latest | `cargo test --workspace --all-features --locked --test '*'`, then the relay e2e step (D4) | `e2e` |
| `e2e-macos` (in `e2e.yml`) | macos-15 | same selector | `e2e` |
| `e2e-windows` (in `e2e.yml`) | windows-latest | same selector, then the Task Scheduler step (unchanged command) | `e2e` |

`unit` is a matrix job in `ci.yml`, and `e2e` is a matrix job in `e2e.yml` (D7), each over `os` with `strategy.fail-fast: false`. Display names are `unit (linux|macos|windows)` and `e2e (linux|macos|windows)`; the `unit-linux` style ids in the table are shorthand. `ci-ok` keeps its name and `needs: [changes, lint, unit, scripts]`; its body is unchanged.

`--doc` cannot be combined with `--lib` in one invocation, so the unit job runs a second step `cargo test --workspace --all-features --locked --doc` (about 1 s today, 0 tests). Doctests added later therefore run in `unit` automatically.

- **Why separate cache keys**: `--lib --bins` and `--test '*'` build different target sets. If they share a key, whichever job saves last overwrites the other's target set and one job restores a cold cache. Separate keys cost more cache storage, but the 10 GB quota is shared per repository and today's keys use about 2 GB.
- **Alternative**: one job per OS running unit, then e2e. One compile, but the unit result is reported late. Rejected because it defeats the purpose.
- **Revision (owner, after PR #173's first run)**: e2e moved from the PR gate to `main`. PR wall clock drops from about 12 minutes (Windows e2e) to about 4.5 minutes (Windows unit).

### D2. macOS runner

`macos-15` (arm64) is the runner the release build already uses for `darwin-arm64`, so the tested CPU and OS match the shipped binary. `IROH_TUNNEL_E2E_SERVICE` is **not** set on macOS. The launchd lifecycle (`tests/service_user.rs`) needs a logged-in GUI session to bootstrap a user agent, and hosted runners do not guarantee one. Enabling it is a follow-up, to be done once a manual run proves it reliable. This keeps the spec scenario "Linux unaffected" true for macOS as well.

### D3. Relay URL from the environment

`tests/serve_access_tunnel.rs` gains one helper, `fn test_relay_url() -> Option<String>`, which reads `IROH_TUNNEL_TEST_RELAY_URL`.
- When it is set, every generated `serve.toml` and `access.toml` gets `[node] relay_urls = ["<url>"]`. `MiniServe` builds its endpoint with `RelayMode::Custom(RelayMap::from(url))` instead of `RelayMode::Default`.
- When it is unset, the test behaves exactly as today (n0 relays).

`MiniServe` is a bare `iroh::Endpoint` that does not go through the engine, so it does not get the engine's `test-utils` TLS bypass; when the variable is set it also calls `.ca_tls_config(CaTlsConfig::insecure_skip_verify())`. The `#[ignore]` attributes stay, so `cargo test` without a relay never touches the network. The module doc explains both ways to run the suite. No new dependency is needed: `iroh::RelayMap` and `RelayUrl` are already in scope through the `iroh` dev-dependency.

### D4. Self-hosted relay in `e2e (linux)` (in `e2e.yml`)

The relay runs as a step-started container, not as a `services:` entry. A service container starts before checkout, so it cannot mount a certificate that the job generates. Steps, all in `e2e (linux)` after the e2e suite, each with `if: ${{ !cancelled() && matrix.name == 'linux' }}` so a failing e2e suite does not hide the relay result:
1. Generate a throwaway certificate: `openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj /CN=localhost -addext subjectAltName=DNS:localhost,IP:127.0.0.1 -keyout default.key -out default.crt` into `$RUNNER_TEMP/relay/certs`.
2. Write `$RUNNER_TEMP/relay/config.toml`:
   ```toml
   enable_relay = true
   http_bind_addr = "0.0.0.0:3340"
   enable_quic_addr_discovery = false
   enable_metrics = false
   [tls]
   https_bind_addr = "0.0.0.0:3443"
   cert_mode = "Manual"
   cert_dir = "/certs"
   ```
3. `docker run -d --name iroh-relay -p 127.0.0.1:3443:3443 -v $RUNNER_TEMP/relay/certs:/certs:ro -v $RUNNER_TEMP/relay/config.toml:/config.toml:ro n0computer/iroh-relay:v1.4.1 --config-path /config.toml`
4. Wait for health: poll `curl -fsk https://127.0.0.1:3443/healthz` for up to 60 s. On timeout, print `docker logs iroh-relay` and fail.
5. Run `IROH_TUNNEL_TEST_RELAY_URL=https://127.0.0.1:3443 cargo test --workspace --all-features --locked --test serve_access_tunnel -- --ignored --test-threads=1`. Same package selection as the e2e step, so it reuses that build.
6. Always (`if: always()`): `docker logs iroh-relay` and `docker rm -f iroh-relay`.

Keys and versions:
- **Pinning**: the image tag is pinned to the iroh version in `Cargo.lock`. A comment in `ci.yml` says to bump both together.
- **Why `https://127.0.0.1`**: the config requires `https`; the port is published on IPv4 loopback only, so an IP avoids `localhost` resolving to `::1`. Certificate verification is skipped (engine `test-utils` for serve/access, an explicit `insecure_skip_verify` on the test's own `MiniServe` endpoint), and the SAN still lists `IP:127.0.0.1`.
- **Image facts** (Dockerfile at tag v1.4.1): `ENTRYPOINT ["/iroh-relay"]`, no `USER` (runs as root), so arguments go straight to the binary and the mounted key is readable; the files are still `chmod 644` so this does not depend on the uid.
- **`--test-threads=1`**: the four tests share one relay. Serialising them avoids flakes from port and relay contention. The suite takes about 1 to 2 minutes, dominated by the 60 s idle test.
- **Exact config keys**: they come from iroh-relay 1.4.1 `main.rs`. If CI shows a key mismatch, adjusting the config is an internal change: record it, do not ask.

### D5. Cache stage

`cache.yml` keeps `lint` (ubuntu, key `lint`) and replaces `test` with a `unit` matrix over `[ubuntu-latest, macos-15, windows-latest]`, compiling `cargo test --workspace --all-features --locked --lib --bins --no-run` under key `unit`. Trigger and paths are unchanged, and it still never runs tests. The e2e cache is not warmed here: `e2e.yml` runs on every push to `main` and `setup-rust` saves on `main`, so the e2e jobs keep their own `e2e` cache warm. Warming it in `cache.yml` too would compile e2e twice on the same push.

### D6. Spec changes

`ci-cd-pipeline` gets:
- MODIFIED "Three single-purpose pipeline stages": four workflows. `e2e.yml` on pushes to `main` is the e2e stage, and "Merge to main" now runs `e2e.yml` (always) and `cache.yml` (paths only).
- MODIFIED "Pull-request test gate": unit jobs only, plus macOS.
- MODIFIED "Windows test job includes the user-service lifecycle": `e2e (windows)` in `e2e.yml`.
- MODIFIED "Cache stage compiles without running tests": unit matrix on three OSes.
- MODIFIED "Release preflight requires a passed test gate": adds `e2e-ok` on the tagged commit.
- ADDED "Unit and end-to-end tests are separate jobs": selectors, exact coverage and stages.
- ADDED "End-to-end stage on main": trigger, jobs and `e2e-ok`.
- ADDED "Relay e2e runs against a self-hosted relay".

### D7. E2E workflow on main (`e2e.yml`)

- **Trigger**: `on: push: branches: [main]`, with no path filter. Every merge runs e2e, including docs-only merges, so every tagged commit has an `e2e-ok`. A path-filtered skip would leave docs-only commits untaggable.
- **Concurrency**: `group: e2e-${{ github.sha }}`, `cancel-in-progress: false`. A newer push must not cancel the run for an older commit that may be tagged.
- **Permissions**: `contents: read`.
- **Jobs**: `e2e` (matrix as in D1, with the Windows Task Scheduler step and the Linux relay steps from D4, moved unchanged from `ci.yml`), plus `e2e-ok`: `if: always()`, `needs: [e2e]`, and the same "fail on failure/cancelled" body as `ci-ok`.

### D8. Release preflight also requires `e2e-ok`

After the existing `ci-ok` check on the PR head, preflight queries `GET /repos/{repo}/commits/{GITHUB_SHA}/check-runs?check_name=e2e-ok` and takes the most recent run by `completed_at`. Its conclusion must be `success`. Otherwise preflight fails with "e2e-ok on <sha> is '<conclusion or missing>', expected success" and nothing is built. A tag pushed while `e2e.yml` is still running fails preflight (missing). The fix is to wait for `e2e-ok` and re-run the release run. Preflight permissions are unchanged (`checks: read` is already granted). The job name `e2e-ok` is therefore load-bearing, like `ci-ok`.

- **Alternative**: let preflight wait for `e2e-ok` by polling. Rejected: it can hold a runner for 12 minutes, and "tag after main is green" is a simple rule to document.
- **Why the tagged commit**: `e2e.yml` runs on the merge commit itself, the exact tree that is released.

## Risks / Trade-offs

- [E2E regressions are found after merge] → Owner-accepted. Release preflight requires `e2e-ok` on the tagged commit, so they cannot ship. A red `e2e-ok` on `main` is fixed by a follow-up PR.
- [A tag pushed before `e2e.yml` finishes fails preflight] → Documented in CONTRIBUTING: tag after `e2e-ok` is green, or re-run the release run.
- [Unit and e2e compile separately per OS] → More runner minutes in total, but they run at different times (PR versus main).
- [macOS hosted runners are slower and cost more minutes] → Accepted by the owner (gate 1). One macOS job per PR, and one per push to `main`.
- [iroh-relay config keys or image entrypoint differ from the 1.4.1 source] → First PR run proves it. The health-wait step prints container logs on failure.
- [Self-hosted relay e2e is flaky] → `--test-threads=1`, a 60 s health wait and container logs on failure. If it stays flaky, the follow-up is to quarantine it to non-blocking, an owner decision.
- [QUIC address discovery is off on the self-hosted relay, so probes to UDP 7824 go unanswered] → Same as any product deployment of a custom relay without QAD. If home-relay selection proves slow or flaky on CI, enable QAD (`enable_quic_addr_discovery = true`, `quic_bind_addr = "0.0.0.0:7824"`, `-p 127.0.0.1:7824:7824/udp`).
- [Image tag drifts from the `iroh` crate version] → A comment beside the tag. A mismatch shows up as a protocol error in the relay e2e step.
- [macOS launchd lifecycle not covered] → Recorded as a follow-up (D2). It is no worse than today, where macOS has no PR tests at all.

## Migration Plan

One PR. The PR's own CI exercises the unit jobs. `e2e.yml` cannot run on the PR because it triggers only on `main`; its jobs were already proven by the first revision's PR run (run 42039703639: all three e2e cells, the relay container and the Task Scheduler step green), and they move unchanged. After merge, `e2e.yml` runs on the merge commit, which is the first real `e2e-ok`, and `cache.yml` runs because it changed. The first tag after merge must wait for that `e2e-ok`. Rollback is to revert the merge.

## Open Questions

None.
