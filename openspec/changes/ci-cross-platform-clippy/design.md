## Context

`ci.yml` has one `lint` job on `ubuntu-latest`: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, and `cargo doc` with `RUSTDOCFLAGS=-D warnings`. Unit tests already run as a matrix (`unit (linux|macos|windows)`, `fail-fast: false`). `setup-rust` pins Rust 1.91 and keys `Swatinem/rust-cache` by `cache-key`; the cache key also hashes the rustc host triple, so one key name yields a separate cache per OS. `cache.yml` warms `lint` on Linux and `unit` on all three OS. `main` has no branch protection; release preflight reads only `ci-ok`.

Clippy results differ across targets (and across Rust releases): Windows-only modules (SCM, trust model, Task Scheduler) are linted only when the target is Windows. The pinned 1.91 toolchain must be the one linting.

## Goals / Non-Goals

**Goals:**
- Clippy with warnings denied runs natively on Linux, macOS and Windows for every code pull request, on the pinned toolchain.
- Lint results per OS stay separate from unit-test results.
- New lint jobs start from a cache warmed on `main`.

**Non-Goals:**
- Running rustfmt or rustdoc on more than one OS.
- Cross-compilation, new workflows, changes to `ci-ok`, `e2e-ok` or release preflight.
- Any change to the lint configuration itself.

## Decisions

**D1. `lint` becomes an OS matrix, mirroring `unit`.** Matrix entries `linux`/`macos`/`windows` on `ubuntu-latest`/`macos-15`/`windows-latest`, job name `lint (${{ matrix.name }})`, `fail-fast: false`. Alternatives: clippy inside `unit` jobs (mixes signals, little cache sharing); a separate `clippy (<os>)` family next to a Linux-only `lint` (two job families for one concern, more YAML). The matrix keeps the existing job id, so `ci-ok`'s `needs` list is unchanged.

**D2. rustfmt and rustdoc stay Linux-only, as steps gated by `if: matrix.name == 'linux'`.** rustfmt output does not depend on the target; the rustdoc gate is specified for the Linux target ("Broken rustdoc link" scenario) and running it three times adds cost without a stated need. The `rustfmt` component is installed only where used.

**D3. Same clippy command on every OS:** `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`. `--all-features` covers `test-utils` code; locally, default-feature and `--lib --bins` builds were also clean, but adding them would triple the cost; not required by the spec.

**D4. Cache.** Keep `cache-key: lint` for every OS (rust-cache separates by host triple). `cache.yml`'s `lint` job becomes the same three-OS matrix running the identical clippy command (without `-D warnings`, as today, since it only warms).

**D5. Windows shell.** The clippy step uses no shell-specific syntax, so the default PowerShell on Windows runners is fine; `setup-rust` already forces `bash` for its own steps.

## Risks / Trade-offs

- [More runner minutes on macOS/Windows per code PR] → accepted by the user; jobs are path-filtered and run in parallel with unit jobs.
- [Cold cache on the first run after merge] → `cache.yml` warms all three on the next dependency change; until then jobs build from scratch but still pass.
- [A platform-only lint finding already on `main`] → the #216 branch was verified clean on 1.91 for all three OS before this change; the first PR after merge confirms it.
- [Clippy behaviour differs between 1.91 and newer releases] → CI uses only the pinned 1.91; a toolchain bump is a separate change that re-runs this gate.

## Migration Plan

Merge after the #216 branch (so `main` is lint-clean on every OS). Rollback: revert the workflow commit; no state to migrate.
