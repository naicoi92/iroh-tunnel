## Design Summary

**Intent.** After `optimize-ci-cd`, each PR runs one `cargo test --workspace` per OS. That single run mixes fast in-process unit tests with slow end-to-end tests that spawn the real binaries, open sockets and start an in-process relay. A failure in a 3-second unit test is reported only when the whole 5-minute job ends, and the job name says nothing about which kind of test failed. The owner wants unit tests and e2e tests split into separate, clearly named jobs.

**Measured (local, `--all-features --locked`, warm build).**

| Selector | Binaries | Passed | Wall |
|---|---|---|---|
| `cargo test --workspace --lib --bins` (unit) | crate `src/` unit tests | 324 | ~50 s (mostly compile; tests take ~5 s) |
| `cargo test --workspace --test '*'` (e2e) | every file in `tests/` | 174 (+4 ignored) | ~222 s |
| `cargo test --workspace --doc` | none present | 0 | ~1 s |

324 + 174 = 498, the same total as the current single run, so the two selectors cover the suite exactly with no overlap and no gap.

**Owner decisions (chat, 2026-11-05).**
1. Boundary: **unit = tests under `src/` (`--lib --bins`), e2e = every integration test under `tests/` (`--test '*'`)**. No test source changes, no list of files to maintain.
2. Schedule: **on pull requests, unit and e2e run as separate jobs in parallel**, all behind `ci-ok`. Pushes to `main` still only warm the cache.
3. Public-relay e2e (`tests/serve_access_tunnel.rs`, `#[ignore]`d): owner chose to run it **in `release.yml` before publishing**. See the conflict under Open Questions.

**Target PR job graph.**

```
changes ─┬─ lint
         ├─ unit-linux     cargo test --workspace --all-features --locked --lib --bins
         ├─ unit-windows   (same, windows-latest)
         ├─ e2e-linux      cargo test --workspace --all-features --locked --test '*'
         ├─ e2e-windows    (same) + Task Scheduler lifecycle step
         ├─ scripts
         └─ ci-ok
```

**Goals**
- A unit-test failure is visible in a job named `unit-*` within about a minute of the run starting.
- E2E failures are reported in jobs named `e2e-*`.
- Unit and e2e for each OS run in parallel; total wall clock is bounded by the slowest e2e job, as today.
- The split is exact: together the jobs run every test the current single job runs.

**Non-goals**
- Moving or rewriting tests, or adding test frameworks (`cargo-nextest`, features, attributes).
- Changing what `main` runs.
- Changing release build/sign/publish behaviour beyond the public-relay question below.

**Anti-goals**
- A test that silently runs in neither job.
- Doubling compile time per OS by building the workspace twice in one job.

## Alternatives Considered

### Alternative A: Selector split by source location (chosen)
- **Approach**: `--lib --bins` for unit, `--test '*'` for e2e, as separate jobs per OS.
- **Pros**: Zero test changes; the rule "tests in `src/` are unit, tests in `tests/` are e2e" is Rust's own convention; new tests are classified automatically.
- **Cons**: Some fast integration tests (e.g. `cli_golden`, `config_paths`) count as e2e; each job compiles its own profile, so more runner minutes overall.
- **Why chosen**: Owner decision; simplest to keep correct.

### Alternative B: Explicit list of fast `tests/` files counted as unit
- **Approach**: Keep `--lib --bins` plus a named list of quick integration tests in the unit job.
- **Pros**: Unit job covers more behaviour quickly.
- **Cons**: The list must be maintained; a new test file lands in neither or both jobs unless someone updates it.
- **Why not chosen**: Violates the "no test silently in neither job" anti-goal.

### Alternative C: Cargo feature or attribute gating
- **Approach**: Gate e2e files with `#![cfg(feature = "e2e")]` or a naming convention.
- **Pros**: Explicit in the code.
- **Cons**: Touches every e2e file and `Cargo.toml`; easy to forget on new files.
- **Why not chosen**: More churn than A for the same result.

## Agreed Approach

Alternative A, with unit and e2e as separate parallel jobs on each OS under `ci-ok`, and the Windows Task Scheduler lifecycle step staying with the Windows e2e job, because it is an e2e test that needs the e2e build.

## Key Decisions

1. Unit selector: `cargo test --workspace --all-features --locked --lib --bins`. E2E selector: `cargo test --workspace --all-features --locked --test '*'`. Doc tests: none exist today; adding `--doc` to the unit job is the builder's call, recorded in design.
2. Jobs: `unit-linux`, `unit-windows`, `e2e-linux`, `e2e-windows`, replacing `test-linux` and `test-windows`. Each OS keeps both kinds, because the code has `cfg(unix)` / `cfg(windows)` splits in both `src/` and `tests/`.
3. `ci-ok` keeps its name and now depends on the four test jobs plus `lint`, `scripts` and `changes`.
4. The cache stage (`cache.yml`) warms whatever the new jobs build, so PR jobs keep restoring from main.
5. Public-relay e2e placement: see Open Questions. It is decided at this gate.

## Open Questions

**Conflict: public-relay e2e in `release.yml` versus the project's `test-utils` rule.** Every test under `tests/` is built with the `test-utils` feature (dev-dependencies enable it). `test-utils` replaces relay TLS verification with `CaTlsConfig::insecure_skip_verify()` (`crates/engine/src/endpoint/create.rs:32-35`). The project rule says `test-utils` must never be used in production or against public relays. `serve_access_tunnel` talks to the public n0 relays, so running it in the release pipeline would run unverified TLS against public infrastructure on every tag. Options for the gate:
- **Keep it manual** (as before this change): no rule violation. Real-relay forwarding stays covered only by in-process-relay e2e tests in CI.
- **Run it in `release.yml` anyway**: the owner's earlier choice. It needs an explicit exception to the `test-utils` rule.
- **Run it in `release.yml` against the built production binaries instead**: requires a new production-binary forwarding harness (similar in spirit to the removed `release_smoke`), which is a larger change and reverses part of `optimize-ci-cd`.
