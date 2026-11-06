Single writer, single worktree; groups run in order 1 → 8 (group 5 needs 2-4; groups 6-8 need 5). TDD for every unit under `action-src/`: write the failing test from the named scenarios first. Platforms: unit tests run on Linux in CI; end-to-end coverage on Linux, macOS and Windows runners (group 6). No change to the Rust crates.

Scenario names refer to `specs/github-actions-integration/spec.md` (GAI) and `specs/ci-cd-pipeline/spec.md` (CCP).

## 1. Toolchain and package skeleton

Owns: `action-src/package.json`, `action-src/bun.lock`, `action-src/tsconfig.json`, `action-src/build.ts`, `.gitattributes` (generated-file entries only).
Scenarios: CCP "Action change only" (install from lockfile, typecheck).

- [ ] 1.1 Create `action-src/` as a bun-managed TypeScript package: `bun init`-style `package.json` (private, `type: module`), `tsconfig.json` (strict, Node 24 target), dependencies `@actions/core`, `@actions/exec`, `@actions/tool-cache` added with `bun add` (design D2); commit `bun.lock`
- [ ] 1.2 Add scripts `typecheck` (`tsc --noEmit`), `test` (`bun test`), `build` (runs `build.ts`) and `check-dist` (build into a temp dir and diff with the committed outputs)
- [ ] 1.3 Write `build.ts`: bundle `access/main`, `access/post`, `serve/main`, `serve/post` with `bun build --target=node` into `access/dist/` and `serve/dist/`; embed the build-time release version (from root `Cargo.toml` `version`) and the minimum supported version as constants; generate the root `action.yml` from `access/action.yml` by rewriting only `runs.main`/`runs.post` (design D2, D6)
- [ ] 1.4 Mark `access/dist/**`, `serve/dist/**` and root `action.yml` as generated in `.gitattributes` (`linguist-generated`)
- [ ] 1.5 Verify: `bun install --frozen-lockfile && bun run typecheck && bun test` succeed on the empty skeleton

## 2. Domain: requests and validation (`action-src/src/domain/`)

Owns: `action-src/src/domain/**`, `action-src/test/domain/**`.
Scenarios (GAI): "Missing required Access input", "Missing required Serve input", "Invalid service name", "Valid input", "Port out of range", "Port boundary accepted", "IPv6 address", "Hostname not allowed for listen", "Malformed NodeId", "Unknown protocol", "Relay URL not https", "Relay token with a trailing newline", "Multi-line relay token", "Malformed version", "Input of the other role".

- [ ] 2.1 Test-first: `AccessRequest` and `ServeRequest` parsing from a raw input map (`name`, `node-id`/`listen` or `target`, `protocol`, `relay-url`, `relay-token`, `version`), returning a typed request or an error that names the input (design D1, D4)
- [ ] 2.2 Test-first: value rules: service name, protocol, `host:port` (IPv4, bracketed IPv6, hostname for `target` only, port 1-65535), NodeId (64 hex / 52 base32), https relay URLs one per line, trimmed single-line relay token, version (`X.Y.Z`, `vX.Y.Z`, optional pre-release, `latest`)
- [ ] 2.3 Test-first: config TOML rendering for one service with optional `relay_urls`/`relay_token` and no `secret_key` (design D3); the rendered file parses with the CLI's rules (assert against a fixture checked by `iroh-tunnel-serve run`'s validation in group 6)
- [ ] 2.4 Verify: `bun test test/domain` green; no adapter imports in `src/domain`

## 3. Adapters (`action-src/src/adapters/`)

Owns: `action-src/src/adapters/**`, `action-src/test/adapters/**`.
Scenarios (GAI): "Default version", "Release not found", "Version too old", "Latest", "Checksum mismatch", "Cached binary", "Unsupported runner", "Alpine container job", "Ubuntu runner", "Key file permissions", "Own relay with a token" (masking).

- [ ] 3.1 Test-first: `ReleaseTarget` mapping from `process.platform`/`arch` and libc detection (musl vs glibc) to archive and checksum file names; reject macOS x64 and Linux riscv64 with the supported list (design D6)
- [ ] 3.2 Test-first: `ReleaseSource`: resolve `latest` to the newest stable release, default to the embedded version, reject versions below the embedded minimum, download archive + checksum file from `https://github.com/naicoi92/iroh-tunnel/releases/download/v<ver>/`, verify SHA-256 before extract, tool-cache by (role, version); honour the CI-only `IROH_TUNNEL_ACTION_BIN_DIR` (fake HTTP and cache in tests)
- [ ] 3.3 Test-first: `ActionsIO` wrapper over `@actions/core` (inputs, outputs, saveState/getState, `setSecret` before any use of `relay-token`, log groups, failure)
- [ ] 3.4 Test-first: `StepWorkspace`: create `RUNNER_TEMP/iroh-tunnel-<random>/` with `state/`, `home/`, config file mode 0600 on Unix, log file; delete it (design D3, D5)
- [ ] 3.5 Test-first: `ProcessLauncher` (spawn detached with env `IROH_TUNNEL_STATE_DIR`, Unix `HOME`, stdout/stderr to the log; SIGTERM then SIGKILL after 5 s on Unix, terminate on Windows; liveness check), `StatusFile` reader, `TcpProbe` (connect/close, loopback for `0.0.0.0`/`[::]`) (design D7, D8)
- [ ] 3.6 Verify: `bun test test/adapters` green; typecheck clean

## 4. Use cases: start and stop (`action-src/src/usecases/`)

Owns: `action-src/src/usecases/**`, `action-src/test/usecases/**`.
Scenarios (GAI): "Access ready", "Serve ready", "UDP Access step", "Readiness timeout", "Tunnel exits during start", "No outputs on failure", "System service on a self-hosted Linux runner", "Second Serve step on Windows", "Normal stop", "Main step failed before starting", "Successful job", "Failed job", "No key leaks", "Key removed at job end", "Later steps use the tunnel".

- [ ] 4.1 Test-first: `start(request)` orchestration with fakes: validate → install → write workspace/config → launch → await readiness (Serve: `node_id` present, issue #177; Access TCP: probe then non-empty `transports`; Access UDP: listener only, log note) polled every 500 ms up to 60 s → set output `node-id` → save state (design D7)
- [ ] 4.2 Test-first: start failures: early exit (exit code, CLI error class, log tail), timeout (last status, log tail), exit code 6 mapped to "role already running for this runner user" with system-service/other-step hints and the Windows one-step-per-role limit (design D5, D7)
- [ ] 4.3 Test-first: `stop()` orchestration: no state → no-op success; else print final status in a group, stop the process, print the log only when the job failed, delete the workspace, fail if still alive (design D8)
- [ ] 4.4 Test-first: no generated secret key in any output, log line or summary (scan fake IO output for the key read from the fixture config)
- [ ] 4.5 Verify: `bun test` green for the whole package

## 5. Entry points, metadata and bundles

Owns: `action-src/src/entry/**`, `access/action.yml`, `serve/action.yml`, `access/dist/**`, `serve/dist/**`, root `action.yml` (generated).
Scenarios (GAI): "Access through the root path", "Access through the alias path", "Serve path", "Root metadata", "Serve metadata", "Root metadata mirrors the access directory", "Minimal Access step", "Several Access services", "Serve in front of a runner-mapped job service", "Serve in a container job with a hostname target", "UDP service", "Two relays", "Default relays".

- [ ] 5.1 Entry modules `access/main`, `access/post`, `serve/main`, `serve/post` wiring real adapters into the use cases
- [ ] 5.2 `access/action.yml`: `name: iroh-tunnel`, description, `branding`, inputs `name`, `node-id`, `listen` (required), `protocol`, `relay-url`, `relay-token`, `version` (optional, with descriptions and defaults), internal `job-status` (default `${{ job.status }}`, not documented), output `node-id`, `runs: node24`, `main: dist/main.js`, `post: dist/post.js`, `post-if: always()`
- [ ] 5.3 `serve/action.yml`: inputs `name`, `target` (required) plus the same optional four and internal `job-status`; output `node-id`; same `runs` shape
- [ ] 5.4 Run `bun run build`; commit `access/dist`, `serve/dist` and the generated root `action.yml`; unit test that the root and `access/action.yml` differ only in `runs.main`/`runs.post`
- [ ] 5.5 Verify: `bun run check-dist` reports no diff; `actionlint` accepts workflows using `./`, `./access`, `./serve`

## 6. CI coverage (`.github/workflows/ci.yml`, `.github/workflows/e2e.yml`)

Owns: `.github/workflows/ci.yml`, `.github/workflows/e2e.yml`.
Scenarios (CCP): "Action change only", "Version bump", "Mixed change", "A check fails", "Docs-only change", "Code change", "Suite contents", "Action tests", "Actions on every OS", "Container job on Linux", "Action regression", "No release download in CI", "Alias path on Linux and macOS", "Windows limit asserted"; unchanged and kept passing: "Installer script change only", "Workflow change only", "Superseded push", "Script lint"; (GAI) "Concurrent jobs", "Two Serve steps on Linux", "Concurrent jobs on one self-hosted runner user", "Access and Serve together on Windows".

- [ ] 6.1 `ci.yml`: add filter `action` (`action.yml`, `access/**`, `serve/**`, `action-src/**`, `.github/workflows/ci.yml`, `Cargo.toml` because the bundles embed its version) and job `action` (setup bun pinned, `bun install --frozen-lockfile`, `typecheck`, `test`, `check-dist`); add `action` to `ci-ok.needs`; update the header diagram (design D10)
- [ ] 6.2 `e2e.yml`, each OS cell: build `iroh-tunnel-serve`/`iroh-tunnel-access` (`cargo build --release --locked --bin ...`), set `IROH_TUNNEL_ACTION_BIN_DIR`, start a local TCP echo target, `uses: ./serve`, `uses: ./` with its `node-id`, round-trip bytes through the listener and assert equality
- [ ] 6.3 `e2e.yml`: Linux and macOS add a second Serve step and a `uses: ./access` step with a round trip; Windows runs the second Access step with `continue-on-error` and the CI-only `IROH_TUNNEL_ACTION_ERROR_FILE`, and a following step asserts that file holds the one-step-per-role message
- [ ] 6.4 `e2e.yml`: Linux job `e2e-action-container` running in `container:` with a job service, Serve target by hostname, round trip from an Access step in the same job; include it in `e2e-ok.needs`
- [ ] 6.5 Verify: `actionlint` clean; `bun test` green; the Rust suite is untouched (`git diff --stat -- tests/ crates/ src/` empty)

## 7. Release stage (`.github/workflows/release.yml`)

Owns: `.github/workflows/release.yml`.
Scenarios (CCP): "Stable release", "Pre-release", "Release job failed", "Older patch released later", "Tag move fails".

- [ ] 7.1 Add job `action-tag` after `github-release` (needs it, stable only): compute the highest stable `vX.*.*` tag; when it equals the run's tag, force-push `vX` to that commit with `contents: write`; otherwise log and skip; on push failure, fail with the manual `git tag -f`/`git push -f` command in the log (design D9)
- [ ] 7.2 Verify: `actionlint` clean; shellcheck the job script; dry-run the version-comparison script locally against sample tag lists (stable, rc, older patch)

## 8. Documentation

Owns: `docs/github-actions.md`, `README.md` (new section only), `docs/README.md` (index row), `CONTRIBUTING.md` (Releasing and development notes), `CHANGELOG.md` (`[Unreleased]`).
Scenarios (GAI): "Copy-paste adoption", "Docs page examples", "Embedded client contract"; (CCP) "Marketplace step".

- [ ] 8.1 `docs/github-actions.md`: examples (Access to a private service, Serve with a job service, Serve in `container:`, two services as two steps, own relay with token, Serve + Access in one job), input/output tables, fixed behaviour (readiness incl. the one probe connection, 60 s, cleanup, logs), version pinning (`@v0`, `@vX.Y.Z`, `version`), supported platforms, Windows limit, embedded-client contract (ALPN `iroh-tunnel/<name>`, one bidirectional stream per TCP connection, raw bytes, same relays) (design D11)
- [ ] 8.2 README: short "GitHub Actions" section with one Access and one Serve example and a link; `docs/README.md` index row
- [ ] 8.3 CONTRIBUTING: action development (`action-src/`, `bun run build`, committed bundles), Releasing checklist step to tick "Publish this Action to the GitHub Marketplace" (and accept the agreement on the first release), note on the automatic `v0` move
- [ ] 8.4 CHANGELOG `[Unreleased]`: the Access and Serve GitHub Actions
- [ ] 8.5 Verify: every example in the docs is valid YAML and uses only declared inputs (script check against `access/action.yml`/`serve/action.yml`); `openspec validate github-actions-serve-access --strict`
- [ ] 8.6 Verify on the main-branch run after merge (needs owner permission): `action` job green on the PR; `e2e (linux|macos|windows)` and the container job green with round trips and post-step cleanup logged
