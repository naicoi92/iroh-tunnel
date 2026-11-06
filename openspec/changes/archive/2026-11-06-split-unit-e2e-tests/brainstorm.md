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
2. Schedule (revised by the owner after the first PR run, 2026-11-05): **pull requests run unit tests only** (plus lint and scripts) behind `ci-ok`; **e2e runs after merge, on every push to `main`**, in its own workflow `e2e.yml`, reported as one aggregate check `e2e-ok`. `cache.yml` stays separate.
5. **Release gate on e2e**: release preflight additionally requires a successful `e2e-ok` on the tagged commit itself, so a red e2e on `main` blocks publication (owner).
3. Relay e2e (`tests/serve_access_tunnel.rs`, today `#[ignore]`d because it needs the public n0 relays): **run it against a self-hosted `iroh-relay` started as a Docker service in CI** (owner, gate 1, pointing at https://docs.iroh.computer/iroh-services/relays/self-hosted), not against public relays.
4. **macOS runs unit and e2e too** (owner, gate 1 annotation).

**Target job graphs.**

```
ci.yml (pull request)
changes ─┬─ lint
         ├─ unit (linux|macos|windows)   --lib --bins, --doc
         ├─ scripts
         └─ ci-ok

e2e.yml (every push to main)
         ┌─ e2e (linux)     --test '*' + relay e2e against a self-hosted iroh-relay
         ├─ e2e (macos)     --test '*'
         ├─ e2e (windows)   --test '*' + Task Scheduler lifecycle
         └─ e2e-ok          aggregate, checked by release preflight

release.yml (v* tag): preflight requires ci-ok on the merged PR head AND e2e-ok on the tagged commit
```

The self-hosted relay is a Docker service of the official `n0computer/iroh-relay` image, pinned to the iroh version in `Cargo.lock` (1.4.1). It runs inside the CI job, so the test no longer depends on the public Internet. GitHub-hosted Docker services exist only on Linux runners, so the relay e2e runs in `e2e-linux` only; macOS and Windows keep the in-process-relay e2e tests.

**Goals**
- PR feedback is bounded by the unit jobs (a few minutes), not by e2e.
- Linux, macOS and Windows each run unit tests on PRs and e2e tests on `main`.
- The relay e2e suite runs on every push to `main`, against a relay the job controls, never against public relays.
- E2E failures are reported in jobs named `e2e (<os>)` and block releases of that commit.
- The split is exact: together the jobs run every test the current single job runs.

**Non-goals**
- Moving or rewriting tests, or adding test frameworks (`cargo-nextest`, features, attributes). Exception: `tests/serve_access_tunnel.rs` learns to take the relay URL from the environment instead of the hard-coded n0 default.
- Changing release build/sign/publish behaviour (only preflight gains the `e2e-ok` check).

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

**Accepted trade-off of the revised schedule**: an e2e regression is found after merge, on `main`, not before. It cannot reach users because release preflight requires `e2e-ok` on the tagged commit.

## Key Decisions

1. Unit selector: `cargo test --workspace --all-features --locked --lib --bins`. E2E selector: `cargo test --workspace --all-features --locked --test '*'`. Doc tests: none exist today; adding `--doc` to the unit job is the builder's call, recorded in design.
2. Jobs: `unit (linux|macos|windows)` in `ci.yml`, replacing `test-linux` and `test-windows`; `e2e (linux|macos|windows)` in the new `e2e.yml`. Each OS runs both kinds, because the code has `cfg(unix)` / `cfg(windows)` / macOS-specific splits in both `src/` and `tests/`.
3. `ci-ok` keeps its name and depends on `changes`, `lint`, `unit` and `scripts`. `e2e.yml` reports `e2e-ok` the same way.
4. `cache.yml` keeps warming `lint` and `unit` for PRs; `e2e.yml` runs on `main` and saves its own `e2e` cache, so e2e is not compiled twice.
5. Relay e2e: a self-hosted `iroh-relay` Docker service in `e2e-linux`, reached over HTTPS with a certificate the job generates. The `test-utils` build skips relay TLS verification, which is acceptable here because the relay is local and job-owned; the project rule only forbids it against public relays. The suite's `#[ignore]` stays (so local `cargo test` without a relay still passes) and CI runs it with `--ignored` and the relay URL in the environment.
6. macOS: the macOS user-service lifecycle test (launchd, `IROH_TUNNEL_E2E_SERVICE=1`) is not enabled by default; whether GitHub's macOS runners allow a launchd user agent is left to design, which may enable it if it works reliably.

## Open Questions

None blocking. Left to design: the exact relay startup (image tag, config, generated certificate, health wait), the environment variable that carries the relay URL into the test, and whether the macOS launchd lifecycle can run on hosted runners.
