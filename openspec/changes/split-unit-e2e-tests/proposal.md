## Why

Each PR runs one `cargo test --workspace` per OS, mixing 324 fast unit tests with 174 slow end-to-end tests, so PR feedback waits on e2e. macOS is not tested at all, and the relay e2e suite never runs in CI because it needs the public n0 relays. The owner wants PRs to run unit tests only, e2e after merge on `main`, every OS covered, relay e2e against a self-hosted relay, and releases blocked by a red e2e.

## What Changes

**PR test jobs**
- From: `test-linux` and `test-windows`, each running the whole workspace suite.
- To: `unit (linux|macos|windows)` (`--lib --bins` and `--doc`) behind `ci-ok`. No e2e on PRs.
- Impact: faster PR feedback; e2e regressions surface after merge.

**E2E on main (new `e2e.yml`)**
- To: on every push to `main`, `e2e (linux|macos|windows)` (`--test '*'`) and an always-reporting `e2e-ok`.

**Release preflight**
- From: requires `ci-ok` on the merged PR head.
- To: also requires the most recent `e2e-ok` on the tagged commit to be `success`.

**Relay e2e**
- From: `tests/serve_access_tunnel.rs` is `#[ignore]`d, uses the public n0 relays, and is run only by hand.
- To: the test takes an optional relay URL from the environment. `e2e (linux)` in `e2e.yml` starts a self-hosted `n0computer/iroh-relay` container with a job-generated TLS certificate and runs the suite with `--ignored` against it. Without the variable, the test behaves as before.
- Impact: no public relay is contacted in CI; the `test-utils` TLS bypass is used only against the job's own local relay.

**Windows lifecycle**
- From: a step in `test-windows`.
- To: the same step in `e2e (windows)` in `e2e.yml`.

**Cache stage**
- From: compiles `cargo test --no-run` on ubuntu and windows.
- To: compiles the unit targets (key `unit`) on ubuntu, macOS and windows; e2e caches are written by `e2e.yml` itself.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `ci-cd-pipeline`: four workflows instead of three; the PR gate runs unit tests only (adds macOS); a new e2e stage on `main`; the Windows lifecycle moves to `e2e (windows)`; the cache stage covers unit; release preflight also requires `e2e-ok`; a new requirement covers relay e2e against a self-hosted relay.

## Impact

- **Files**: `.github/workflows/ci.yml`, `.github/workflows/e2e.yml` (new), `.github/workflows/cache.yml`, `.github/workflows/release.yml` (preflight only), `tests/serve_access_tunnel.rs` (relay URL from environment, doc comment), `CONTRIBUTING.md` (how to run unit and e2e locally), `openspec/specs/ci-cd-pipeline/spec.md` (via sync at archive).
- **Compatibility**: no product change. The check to require stays `ci-ok`. Local `cargo test` still runs everything except the ignored relay suite.
- **Security**: the relay container listens only on the runner, uses a throwaway self-signed certificate, and no secret is involved.
- **Cost**: one macOS unit job per code PR; three e2e jobs (one with a container) per push to `main`.
- **Non-goals**: moving tests between `src/` and `tests/`; changing release build, sign or publish; enabling the macOS launchd lifecycle unless it proves reliable (design decides).
