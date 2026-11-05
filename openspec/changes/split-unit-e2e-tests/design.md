## Context

After `optimize-ci-cd`, `.github/workflows/ci.yml` has `test-linux` and `test-windows`. Both run `cargo test --workspace --all-features --locked`, and `test-windows` adds a Task Scheduler step (`IROH_TUNNEL_E2E_SERVICE=1 cargo test --all-features --locked --test service_user_windows -- --nocapture --test-threads=1`). `cache.yml` warms cache keys `lint` (ubuntu) and `test` (ubuntu, windows) through `.github/actions/setup-rust`. That action uses `Swatinem/rust-cache@v2` with `shared-key: <cache-key>` and saves only on `main`; rust-cache adds the runner OS to the key.

Measured locally: `--lib --bins` runs 324 tests, and `--test '*'` runs 174 tests plus 4 ignored. Together that is the 498 of the full run. There are no doctests.

`tests/serve_access_tunnel.rs` (4 tests, all `#[ignore]`) runs serve and access in-process via `iroh_tunnel_host::testing::run_unowned`. It writes `serve.toml` and `access.toml` with no `relay_urls`, so both fall back to the n0 relays. Its `MiniServe` helper builds an `iroh::Endpoint` with `RelayMode::Default`. Every `tests/` binary is built with the `test-utils` feature, which makes `crates/engine/src/endpoint/create.rs:32-35` use `CaTlsConfig::insecure_skip_verify()`, so a self-signed relay certificate is accepted. Config validation requires `https://` relay URLs (`crates/host/src/config/dto.rs:480,506`).

The official image `n0computer/iroh-relay` publishes `v1.4.1`, matching `iroh 1.4.1` in `Cargo.lock`. In iroh-relay 1.4.1, `cert_mode = "Manual"` reads `default.crt` and `default.key` from `cert_dir`. `--dev` serves plain HTTP only, which config validation rejects. Docker is not available on the development machine, so the container path can only be verified on CI.

No product code is in scope. The only test-code change is in `tests/serve_access_tunnel.rs`.

## Goals / Non-Goals

**Goals:**
- Six parallel test jobs: unit and e2e on Linux, macOS and Windows.
- Exact coverage: unit plus e2e equals the current suite, with nothing run twice and nothing dropped.
- Relay e2e runs on every code PR against a relay the job owns.

**Non-Goals:**
- Moving tests between `src/` and `tests/`.
- Changing `release.yml`.
- Relay e2e on macOS or Windows; GitHub Docker services exist only on Linux runners.

## Decisions

### D1. Selectors and job names

| Job | Runner | Command | Cache key |
|---|---|---|---|
| `unit-linux` | ubuntu-latest | `cargo test --workspace --all-features --locked --lib --bins` | `unit` |
| `unit-macos` | macos-15 | same | `unit` |
| `unit-windows` | windows-latest | same | `unit` |
| `e2e-linux` | ubuntu-latest | `cargo test --workspace --all-features --locked --test '*'`, then the relay e2e step (D4) | `e2e` |
| `e2e-macos` | macos-15 | same selector | `e2e` |
| `e2e-windows` | windows-latest | same selector, then the Task Scheduler step (unchanged command) | `e2e` |

The jobs are a matrix of two job ids (`unit`, `e2e`), each over `os`, with `strategy.fail-fast: false` so a failing cell never cancels the others. Display names are `unit (linux|macos|windows)` and `e2e (linux|macos|windows)`; the `unit-linux` style ids in the table are shorthand. `ci-ok` keeps its name and `needs: [changes, lint, unit, e2e, scripts]`.

`--doc` cannot be combined with `--lib` in one invocation, so the unit job runs a second step `cargo test --workspace --all-features --locked --doc` (about 1 s today, 0 tests). Doctests added later therefore run in `unit` automatically.

- **Why separate cache keys**: `--lib --bins` and `--test '*'` build different target sets. If they share a key, whichever job saves last overwrites the other's target set and one job restores a cold cache. Separate keys cost more cache storage, but the 10 GB quota is shared per repository and today's keys use about 2 GB.
- **Alternative**: one job per OS running unit, then e2e. One compile, but the unit result is reported late. Rejected because it defeats the purpose.

### D2. macOS runner

`macos-15` (arm64) is the runner the release build already uses for `darwin-arm64`, so the tested CPU and OS match the shipped binary. `IROH_TUNNEL_E2E_SERVICE` is **not** set on macOS. The launchd lifecycle (`tests/service_user.rs`) needs a logged-in GUI session to bootstrap a user agent, and hosted runners do not guarantee one. Enabling it is a follow-up, to be done once a manual run proves it reliable. This keeps the spec scenario "Linux unaffected" true for macOS as well.

### D3. Relay URL from the environment

`tests/serve_access_tunnel.rs` gains one helper, `fn test_relay_url() -> Option<String>`, which reads `IROH_TUNNEL_TEST_RELAY_URL`.
- When it is set, every generated `serve.toml` and `access.toml` gets `[node] relay_urls = ["<url>"]`. `MiniServe` builds its endpoint with `RelayMode::Custom(RelayMap::from(url))` instead of `RelayMode::Default`.
- When it is unset, the test behaves exactly as today (n0 relays).

`MiniServe` is a bare `iroh::Endpoint` that does not go through the engine, so it does not get the engine's `test-utils` TLS bypass; when the variable is set it also calls `.ca_tls_config(CaTlsConfig::insecure_skip_verify())`. The `#[ignore]` attributes stay, so `cargo test` without a relay never touches the network. The module doc explains both ways to run the suite. No new dependency is needed: `iroh::RelayMap` and `RelayUrl` are already in scope through the `iroh` dev-dependency.

### D4. Self-hosted relay in `e2e-linux`

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

`cache.yml` replaces the `lint` and `test` jobs with `lint` (ubuntu, key `lint`, unchanged) and a matrix `{kind: [unit, e2e]} × {os: [ubuntu-latest, macos-15, windows-latest]}`. Each cell compiles with the same selector as its PR job plus `--no-run`, under key `unit` or `e2e`. It still never runs tests.

### D6. Spec changes

`ci-cd-pipeline` gets:
- MODIFIED "Pull-request test gate": the job names and macOS.
- MODIFIED "Windows test job includes the user-service lifecycle": the job is now `e2e-windows`, and macOS is added to the "not set" side.
- MODIFIED "Cache stage compiles without running tests": the matrix.
- ADDED "Unit and end-to-end tests are separate jobs": the selectors and exact coverage.
- ADDED "Relay e2e runs against a self-hosted relay".

No other capability changes.

## Risks / Trade-offs

- [Each OS compiles twice, once for unit and once for e2e] → More runner minutes. Wall clock is bounded by the slowest e2e job, and separate cache keys keep both warm.
- [macOS hosted runners are slower and cost more minutes] → Accepted by the owner (gate 1). The two macOS jobs run in parallel with the others.
- [iroh-relay config keys or image entrypoint differ from the 1.4.1 source] → First PR run proves it. The health-wait step prints container logs on failure.
- [Self-hosted relay e2e is flaky] → `--test-threads=1`, a 60 s health wait and container logs on failure. If it stays flaky, the follow-up is to quarantine it to non-blocking, an owner decision.
- [QUIC address discovery is off on the self-hosted relay, so probes to UDP 7824 go unanswered] → Same as any product deployment of a custom relay without QAD. If home-relay selection proves slow or flaky on CI, enable QAD (`enable_quic_addr_discovery = true`, `quic_bind_addr = "0.0.0.0:7824"`, `-p 127.0.0.1:7824:7824/udp`).
- [Image tag drifts from the `iroh` crate version] → A comment beside the tag. A mismatch shows up as a protocol error in the relay e2e step.
- [macOS launchd lifecycle not covered] → Recorded as a follow-up (D2). It is no worse than today, where macOS has no PR tests at all.

## Migration Plan

One PR. The PR's own CI exercises the new jobs, including the relay container. After merge, `cache.yml` runs if its paths changed. This change touches `cache.yml`, so it runs and seeds the `unit` and `e2e` keys. Old `test` cache entries age out. Rollback is to revert the merge.

## Open Questions

None.
