## Design Summary

Issue #216 introduced a workspace-wide no-panic lint policy (`[workspace.lints.clippy]`, `-D warnings`). Most platform-specific code (Windows SCM, trust model, Task Scheduler, `cfg(windows)` paths) is only compiled, and therefore only linted, on its own target. Today the pull-request `lint` job runs clippy on Linux only, so a lint violation in Windows- or macOS-only code reaches `main` unnoticed. While implementing #216 this happened in practice: suppressions that were clean on one target or toolchain were unfulfilled on another, and only a local cross-compile caught it.

The change makes the pull-request gate run clippy, with warnings denied, on Linux, macOS and Windows, using the pinned CI toolchain, so the #216 acceptance criterion "clippy passes on Linux, macOS and Windows" is enforced by CI rather than by hand.

Who it serves: contributors (early, per-platform lint feedback on a PR) and maintainers (no platform-only regressions on `main`).

Living requirements involved (`openspec/specs/ci-cd-pipeline/spec.md`):
- **Pull-request test gate**: `lint` runs rustfmt, clippy with warnings denied and the workspace rustdoc build; it runs only for code paths and feeds `ci-ok`. This change **alters** that requirement: clippy runs per operating system instead of once on Linux.
- **Cache stage compiles without running tests**: compiles "the lint target on Linux". This change **alters** it: the lint cache is warmed on all three operating systems.
- **Release preflight requires a passed test gate** reads only `ci-ok`; it is unchanged and keeps working because per-OS lint jobs still feed `ci-ok`.
- **Three single-purpose pipeline stages**: unchanged; no new workflow.

## Alternatives Considered

### Alternative A: Per-OS lint jobs
- **Approach**: the pull-request `lint` job becomes one job per operating system (Linux, macOS, Windows), mirroring the existing `unit (<os>)` jobs; formatting and rustdoc checks stay single-platform.
- **Pros**: lint failures are reported separately from test failures and per platform; native compilation on each OS; same shape as the unit jobs; uses the pinned toolchain.
- **Cons**: more macOS/Windows runner minutes per code PR.
- **Why not chosen**: chosen.

### Alternative B: Clippy inside the unit jobs
- **Approach**: each `unit (<os>)` job runs clippy before its tests.
- **Pros**: no new jobs.
- **Cons**: lint and test failures share one job; clippy (check) and test builds barely share cache, so the job gets slower anyway.
- **Why not chosen**: loses the separate lint signal for no real cost saving.

### Alternative C: Cross-compiled clippy on a Linux runner
- **Approach**: lint Windows (and possibly macOS) targets from Linux with cross toolchains.
- **Pros**: cheapest in runner minutes.
- **Cons**: fragile C cross-compilation for native dependencies; macOS needs an SDK that cannot be redistributed, so macOS would stay unlinted.
- **Why not chosen**: does not cover macOS and adds a brittle toolchain.

## Agreed Approach

Alternative A, approved by the user: clippy with warnings denied runs in its own pull-request job on each of Linux, macOS and Windows, natively, with the pinned toolchain; it is gated by the same code-path filter and reported through `ci-ok` like the other checks. macOS stays in scope (the #216 acceptance names all three platforms).

## Key Decisions

- All three operating systems are linted on every code pull request (user).
- Lint stays a separate signal from unit tests (user, via A).
- Formatting and the rustdoc build are platform-independent enough to stay on one platform; only clippy needs every OS (my call: rustfmt output does not depend on target, and the rustdoc gate is already specified for the Linux target).
- The cache stage warms the lint cache on every OS so the new jobs start warm, as it already does for unit jobs (my call, follows the cache-stage rule "same flags and cache keys as the pull-request jobs").
- No new workflow and no change to release preflight or `ci-ok` naming.

## Open Questions

None blocking. Exact job names and how fmt/doc are split from the per-OS clippy jobs are left to design.
