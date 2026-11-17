# Cross-platform clippy in CI Implementation Plan

> Executed via `/opsx-apply`, which chooses the executor.
> Spec reference: this change's `specs/` and `design.md`.

**Goal:** Every code pull request runs clippy with warnings denied natively on Linux, macOS and Windows, and `main` warms that lint cache on all three.

**Architecture:** The `lint` job in `ci.yml` becomes an OS matrix (same shape as `unit`) whose Linux entry also runs rustfmt and rustdoc; `cache.yml`'s `lint` warm-up becomes the same matrix. Job id `lint` is kept so `ci-ok`'s `needs` list and release preflight (which reads `ci-ok`) are untouched.

**Tech Stack:** GitHub Actions YAML, the repo's `.github/actions/setup-rust` composite action (Rust 1.91, `Swatinem/rust-cache`), actionlint.

## Global Constraints

- Toolchain: the one pinned by `setup-rust` (Rust 1.91); do not add a `toolchain` override.
- Clippy command, identical on every OS: `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` (cache warm-up: same without `-- -D warnings`).
- Runners: `ubuntu-latest`, `macos-15`, `windows-latest` (same as `unit`).
- Job names: `lint (linux)`, `lint (macos)`, `lint (windows)`; `fail-fast: false`.
- rustfmt check and `RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps --locked` run only in `lint (linux)`.
- No new workflow file, no new third-party action, permissions stay `contents: read`, `ci-ok` / `e2e-ok` names unchanged.
- Never push or open a PR without the user's explicit approval.
- Base: the work branch is cut from `refactor/no-panic-policy` (#216), because `main` is lint-clean on every OS only after #216 merges (design, Migration Plan); `/opsx-apply` creates the worktree.
- Local cross-lint prerequisites: `zig`, `cargo-zigbuild`, `cargo-xwin`, and rustup targets `x86_64-unknown-linux-gnu` and `x86_64-pc-windows-msvc` on toolchain 1.91.

## Review Focus

- A `-D warnings` clippy failure on Windows only → `lint (windows)` fails while the other two finish; covered by the local cross-lint step (1.4) and the CI check (1.5).
- `rustfmt` component missing on macOS/Windows jobs → those jobs must not run `cargo fmt`; covered by Step 2's `if:` guard and actionlint.
- `ci-ok` must still fail when only one matrix entry fails → `needs.lint.result` aggregates to `failure`; verified in Step 6 by reading `ci-ok`'s logic (unchanged) and in Step 8.
- Matrix expression typos (`matrix.name` vs `matrix.os`) → actionlint catches undefined matrix keys (Step 5).
- Cache key drift between `ci.yml` and `cache.yml` → both use `cache-key: lint`; checked in Step 5 by grep.

---

## Contracts

| Name | new / modified | Shape | public |
|---|---|---|---|
| `ci.yml` job `lint` | modified | `runs-on: ${{ matrix.os }}`, matrix `include: [{name: linux, os: ubuntu-latest}, {name: macos, os: macos-15}, {name: windows, os: windows-latest}]`, `name: lint (${{ matrix.name }})`, `fail-fast: false`, `needs: changes`, `if: needs.changes.outputs.code == 'true'`; steps: checkout, `setup-rust` (`components` = `rustfmt,clippy` on linux else `clippy`, `cache-key: lint`), fmt check (linux only), clippy with `-D warnings`, rustdoc with `RUSTDOCFLAGS: -D warnings` (linux only) | public (PR check names) |
| `cache.yml` job `lint` | modified | `runs-on: ${{ matrix.os }}`, matrix `os: [ubuntu-latest, macos-15, windows-latest]`, `name: cache (lint, ${{ matrix.os }})`, `fail-fast: false`; steps: checkout, `setup-rust` (`components: clippy`, `cache-key: lint`), clippy without `-D warnings` | no |

---

## Task 1: Per-OS lint jobs — implements 1

**Covers:** 1.1, 1.2, 1.3, 1.4, 1.5
**Difficulty:** Easy
**Blocked by:** none

**Files:**
- Modify: `.github/workflows/ci.yml` (header diagram lines 1-10, `lint` job lines 74-91)
- Modify: `.github/workflows/cache.yml` (header comment lines 1-8, `lint` job lines 30-41)
- Modify: `CONTRIBUTING.md` ("Code quality gates", line ~52)

**Interfaces:**
- Consumes: `.github/actions/setup-rust` inputs `components`, `cache-key` (unchanged).
- Produces: PR check names `lint (linux)`, `lint (macos)`, `lint (windows)` (contract `ci.yml` job `lint`).

- [ ] **Step 1: Baseline the static checks (red for the new expectation)**

Run: `grep -cF 'name: lint (${{ matrix.name }})' .github/workflows/ci.yml`
Expected: `0` — no per-OS lint job exists yet.

- [ ] **Step 2: Rewrite `ci.yml` `lint` as the matrix in the contract**

Keep job id `lint`. Pass `components: ${{ matrix.name == 'linux' && 'rustfmt,clippy' || 'clippy' }}`. Put `if: matrix.name == 'linux'` on the fmt and doc steps. Update the header diagram to `lint (linux|macos|windows)  (clippy; fmt + doc on linux)`. Set `runs-on: ${{ matrix.os }}`. Keep `RUSTDOCFLAGS` in the step's `env:` (not inline) and add no `shell:`: the default pwsh on Windows is intended (D5).

- [ ] **Step 3: Rewrite `cache.yml` `lint` as the matrix in the contract**

Set `runs-on: ${{ matrix.os }}`; keep the clippy command identical to `ci.yml` minus `-- -D warnings`. Update the header comment so it states the lint cache is warmed on Linux, macOS and Windows.

- [ ] **Step 4: Update `CONTRIBUTING.md`**

Replace "PRs run `lint`, `unit (linux|macos|windows)`" with "PRs run `lint (linux|macos|windows)` (clippy on every OS; rustfmt and rustdoc on Linux), `unit (linux|macos|windows)`".

- [ ] **Step 5: Static verification**

Run: `actionlint .github/workflows/ci.yml .github/workflows/cache.yml`
Expected: no output, exit 0.

Run: `grep -c 'cache-key: lint' .github/workflows/ci.yml .github/workflows/cache.yml`
Expected: `1` for each file.

Run: `grep -cF 'name: lint (${{ matrix.name }})' .github/workflows/ci.yml`
Expected: `1`.

Run: `grep -cF 'runs-on: ${{ matrix.os }}' .github/workflows/ci.yml .github/workflows/cache.yml`
Expected: `ci.yml:2`, `cache.yml:2` (lint and unit in each).

Run: `grep -F 'cargo clippy --workspace --all-targets --all-features --locked' .github/workflows/cache.yml`
Expected: exactly one line, without `-D warnings`.

Run: `git diff -U0 refactor/no-panic-policy -- .github/workflows/ci.yml .github/workflows/cache.yml`
Expected: hunks only in each file's header comment and its `lint` job; `on:`, `concurrency:`, `changes:`, `unit:`, `scripts:`, `action:`, `ci-ok:` and `cache.yml`'s `on.push.paths` are unchanged.

Run: `ls .github/workflows`
Expected: exactly `cache.yml ci.yml e2e.yml release.yml`.

Run: `openspec validate ci-cross-platform-clippy --strict`
Expected: `Change 'ci-cross-platform-clippy' is valid`.

- [ ] **Step 6: Local lint of all three targets on the pinned toolchain**

Run (host macOS): `cargo +1.91 clippy --workspace --all-targets --all-features --locked -- -D warnings` → exit 0.
Run (Linux target): with `/tmp/zcc/cc` (bash wrapper dropping `--target=*` and running `zig cc -target x86_64-linux-gnu`) and `/tmp/zcc/ar` (`zig ar`): `CC_x86_64_unknown_linux_gnu=/tmp/zcc/cc AR_x86_64_unknown_linux_gnu=/tmp/zcc/ar cargo +1.91 clippy --workspace --all-targets --all-features --locked --target x86_64-unknown-linux-gnu --target-dir target/lx -- -D warnings` → exit 0. (Changed during apply: the installed cargo-zigbuild has no `clippy` subcommand.)
Run (Windows target): `RUSTUP_TOOLCHAIN=1.91 cargo xwin clippy --workspace --all-targets --all-features --locked --target x86_64-pc-windows-msvc -- -D warnings` → exit 0.
Record the exact commands in `apply.md`. Read the unchanged `ci-ok` step: `needs: [changes, lint, ...]` and the `failure|cancelled` match still cover a failed matrix entry.

- [ ] **Step 7: Commit**

```bash
git add .github/workflows/ci.yml .github/workflows/cache.yml CONTRIBUTING.md
git commit -m "ci: run clippy on Linux, macOS and Windows"
```

- [ ] **Step 8: CI confirmation (gated on user approval)**

Ask the user before any push. If approved, push and open the PR only as the user directs, then confirm `lint (linux)`, `lint (macos)`, `lint (windows)` and `ci-ok` pass. Otherwise record 1.5 as pending in `apply.md`.

---

## Scenario → test

- Code change → Step 8 (PR on a code path shows three lint jobs plus three unit jobs); Step 5 actionlint for structure
- Installer script change only → unchanged path filter; Step 5 (`changes` job untouched, confirmed by diff review)
- Workflow change only → unchanged path filter; Step 5 diff review
- Action change only → unchanged path filter; Step 5 diff review
- Version bump → unchanged `action` filter; Step 5 diff review
- Mixed change → Step 8 (this change edits `ci.yml`, so its own PR runs lint, unit and action)
- Docs-only change → unchanged; `ci-ok` logic untouched (Step 6 reading)
- A check fails → Step 6 reading of `ci-ok`; Step 8 if a matrix entry fails
- Superseded push → unchanged `concurrency` block; Step 5 diff review
- Broken rustdoc link → Step 2 keeps rustdoc in `lint (linux)`; Step 8
- Platform-only lint finding → Step 6 Windows/Linux cross-lint shows each target is linted separately; Step 8 with `fail-fast: false`
- Dependency bump merged → Step 3 + actionlint; observed on the next `Cargo.lock` push to `main`
- Source-only change merged → unchanged `cache.yml` path filter; Step 5 diff review
- Workflow inventory (baseline) → Step 5 `ls .github/workflows`
