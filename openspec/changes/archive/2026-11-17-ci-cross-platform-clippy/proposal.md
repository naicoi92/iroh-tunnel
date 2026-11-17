## Why

The no-panic policy (#216) denies clippy lints workspace-wide, but most Windows- and macOS-only code is only compiled, and so only linted, on its own target. The pull-request `lint` job runs clippy on Linux only, so platform-only violations reach `main` unnoticed; this already happened while implementing #216. Linting every supported OS in CI makes the #216 acceptance "clippy passes on Linux, macOS and Windows" an enforced gate instead of a manual check.

## What Changes

**Pull-request lint**
- From: one `lint` job on Linux runs rustfmt, clippy with warnings denied and the rustdoc build.
- To: clippy with warnings denied runs in one lint job per operating system (`lint (linux)`, `lint (macos)`, `lint (windows)`); the rustfmt check and rustdoc build still run once, on Linux. Every lint job runs only for code paths and feeds `ci-ok`; a failing lint job does not cancel the others.
- Reason: platform-only code must be linted on its platform.
- Impact: non-breaking for contributors; more macOS/Windows runner minutes per code pull request. `ci-ok` and release preflight are unchanged.

**Cache stage**
- From: the lint target is compiled (clippy) on Linux only.
- To: it is compiled on Linux, macOS and Windows with the same flags and cache key as the pull-request lint jobs.
- Reason: keep the new lint jobs starting from a warm cache.
- Impact: non-breaking; more runner minutes on dependency-bump pushes to `main`.

Non-goals: no new workflow, no change to `ci-ok`/`e2e-ok` names or release preflight, no cross-compiled linting, no lint-policy change.

## Capabilities

### New Capabilities

None.

### Modified Capabilities
- `ci-cd-pipeline`: the "Pull-request test gate" requirement runs clippy on every supported OS; the "Cache stage compiles without running tests" requirement warms the lint cache on every supported OS.

## Impact

- `.github/workflows/ci.yml` (`lint` job, header comment), `.github/workflows/cache.yml` (`lint` job), `CONTRIBUTING.md` (names of the pull-request checks).
- No Rust source, dependency or public API change. Security: unchanged permissions (`contents: read`), no new secrets or third-party actions.
