# Apply Receipt

> Generated at the end of the apply phase to mark code-implementation
> complete and provide verify with the state it needs.
> Overwritten on each apply iteration; iteration counter grows.

**Change**: `split-unit-e2e-tests`
**Iteration**: `1`
**Applied at**: `2026-11-05 20:21`
**Executor**: `subagent-driven-development` (implementer: `superspec.implementer`; per-task and whole-branch review: `superspec.task-reviewer`)

---

## Workspace

- **Worktree path**: `/Users/naicoi/Projects/iroh/iroh-tunnel.split-unit-e2e-tests`
- **Worktree branch**: `split-unit-e2e-tests`
- **Feature branch**: `spec/split-unit-e2e-tests`
- **Base branch**: `main`

---

## Commits

- **Range**: `89f0485..0ddd56b` (implementation and task ticks; this receipt is committed on top)
- **Count**: `8`

---

## Tasks

- **Completed**: `15 of 16` checkboxes in tasks.md flipped to `- [x]`
- **Remaining**: `4.4` — verify on the PR run (six test jobs in parallel, `e2e (windows)` Task Scheduler log, `e2e (linux)` relay healthy and `serve_access_tunnel` 4 passed). Needs the owner's permission to push; done in finalize.

Verification at `0ddd56b` (fresh):
- `cargo test --workspace --all-features --locked --lib --bins` 324 passed; `--doc` 0; `--test '*'` 174 passed (+4 ignored); full suite 498 passed → 324 + 0 + 174 = 498.
- `cargo fmt --check`, clippy `-D warnings`, actionlint, shellcheck (tracked `*.sh` and the extracted relay start script), `openspec validate --strict`: clean.
- Relay path, controller-run: the exact `ci.yml` relay config with the official `iroh-relay` v1.4.1 binary (aarch64-apple-darwin; Docker is not available locally) → `/healthz` healthy; `IROH_TUNNEL_TEST_RELAY_URL=https://127.0.0.1:3443 cargo test --workspace --all-features --locked --test serve_access_tunnel -- --ignored --test-threads=1` → 4 passed in 77 s. Negative control with the relay stopped → `serve_access_tunnel_roundtrips_bytes` fails (connection reset), so the suite uses the named relay, not n0.
- Not run yet: the Docker container on GitHub's ubuntu runner and the macOS cells (PR run, task 4.4).

Reviews: every task reviewed; no fix rounds needed. Task 3 (20-line `cache.yml` diff matching design D5) was reviewed by the controller instead of a reviewer seat (ruling below). Final whole-branch review: Ready to merge, no Critical/Important.

Rulings (controller):
- Task 4.4 deferred to finalize (needs push) — cost if wrong: none.
- Task 3 reviewed by the controller from the diff + actionlint — cost if wrong: one extra fix commit.

Deferred minors (can wait): `Relay e2e` step also runs after a failed relay start (noise only); Task Scheduler step lacks `--workspace` (pre-existing, kept per spec); `docs/self-hosted-relay.md` describes a production relay, not the CI test config; relay URL interpolated into TOML unescaped (test-only).

---

## Contract deviations

- none

---

## Next step

Run `/opsx-verify`.
