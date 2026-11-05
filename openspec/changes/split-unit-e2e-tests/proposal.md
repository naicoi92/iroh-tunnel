## Why

Each PR runs one `cargo test --workspace` per OS, mixing 324 fast unit tests with 174 slow end-to-end tests. A unit failure only shows when the 5-minute job ends, and the job name does not say which kind failed. macOS is not tested on PRs, and the relay e2e suite never runs in CI because it needs the public n0 relays. The owner wants unit and e2e tests in separate jobs on every OS, and relay e2e run against a self-hosted relay.

## What Changes

**PR test jobs**
- From: `test-linux` and `test-windows`, each running the whole workspace suite.
- To: `unit-linux`, `unit-macos`, `unit-windows` (`--lib --bins`) and `e2e-linux`, `e2e-macos`, `e2e-windows` (`--test '*'`), all in parallel behind `ci-ok`.
- Impact: non-breaking. Two macOS jobs are added to every code PR.

**Relay e2e**
- From: `tests/serve_access_tunnel.rs` is `#[ignore]`d, uses the public n0 relays, and is run only by hand.
- To: the test takes an optional relay URL from the environment. `e2e-linux` starts a self-hosted `n0computer/iroh-relay` container with a job-generated TLS certificate and runs the suite with `--ignored` against it. Without the variable, the test behaves as before.
- Impact: no public relay is contacted in CI; the `test-utils` TLS bypass is used only against the job's own local relay.

**Windows lifecycle**
- From: a step in `test-windows`.
- To: the same step in `e2e-windows`.

**Cache stage**
- From: compiles `cargo test --no-run` on ubuntu and windows.
- To: compiles the unit and e2e targets with the new jobs' keys on ubuntu, macOS and windows.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `ci-cd-pipeline`: the PR test gate splits unit and e2e jobs per OS and adds macOS; the Windows lifecycle moves to `e2e-windows`; the cache stage covers the new jobs; a new requirement covers relay e2e against a self-hosted relay.

## Impact

- **Files**: `.github/workflows/ci.yml`, `.github/workflows/cache.yml`, `tests/serve_access_tunnel.rs` (relay URL from environment, doc comment), `CONTRIBUTING.md` (how to run unit and e2e locally), `openspec/specs/ci-cd-pipeline/spec.md` (via sync at archive).
- **Compatibility**: no product change. The check to require stays `ci-ok`. Local `cargo test` still runs everything except the ignored relay suite.
- **Security**: the relay container listens only on the runner, uses a throwaway self-signed certificate, and no secret is involved.
- **Cost**: two more macOS jobs and one container per code PR.
- **Non-goals**: moving tests between `src/` and `tests/`; changing `release.yml`; enabling the macOS launchd lifecycle unless it proves reliable (design decides).
