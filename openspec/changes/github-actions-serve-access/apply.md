# Apply Receipt

> Generated at the end of the apply phase to mark code-implementation
> complete and provide verify with the state it needs.
> Overwritten on each apply iteration; iteration counter grows.

**Change**: `github-actions-serve-access`
**Iteration**: `1`
**Applied at**: `2026-11-06 09:08` (UTC)
**Executor**: `subagent-driven-development` (per task: `superspec.implementer`, then one combined spec+quality review by `superspec.task-reviewer`; final whole-branch review by `superspec.task-reviewer`, one fix wave, scoped re-review Approved)

---

## Workspace

- **Worktree path**: `/Users/naicoi/Projects/iroh/iroh-tunnel.github-actions-serve-access`
- **Worktree branch**: `github-actions-serve-access`
- **Feature branch**: `spec/github-actions-serve-access` (created with `wt` from `main` before step 0, because the session started on `main`; step-0 commit `7c51c80`)
- **Base branch**: `main`

---

## Commits

- **Range**: `7c51c80..915cafe` (plus this receipt commit)
- **Count**: `25` (implementation, fix rounds, task ticks and contract-deviation notes)

---

## Tasks

- **Completed**: `37 of 38` checkboxes in tasks.md flipped to `- [x]`
- **Remaining**: `8.6` — verify on the main-branch run after merge (needs owner permission to push/merge). Runner-only items to watch there: Windows background echo server and exit-6 assertion, container job (bundled node in `ubuntu:24.04`, artifact layout), post step receiving `INPUT_relay-token`, Access readiness with a single probe (Ruling R7).

Verification evidence at `915cafe` (worktree, fresh):
- `action-src`: `bun run typecheck` clean; `bun test` 143 pass / 0 fail; `bun run check-dist` no diff; `bun run check-docs` 2 files ok
- `.github/scripts/move-major-tag.test.sh` 14 passed; `shellcheck .github/scripts/*.sh` clean; `actionlint` clean
- `openspec validate github-actions-serve-access --strict` valid
- `action-src/scripts/local-smoke.ts` with real `iroh-tunnel-serve`/`iroh-tunnel-access` (debug build of this commit's tree) over the n0 relays: `SMOKE OK: round trips equal, both posts ran, no iroh-tunnel- process left` (TCP and UDP round trips; config mode 600 after the binary saved its key)
- `git diff --stat 7c51c80..HEAD -- src crates tests Cargo.toml Cargo.lock` empty

Rulings made during apply (cost if wrong):
- R1 build.ts written in Task 1, first exercised in Task 5 — build bug found late.
- R2 BUILD_VERSION 0.5.0 < MIN_VERSION 0.6.0 on this branch; default install rejected until the release-bump PR — bump Cargo.toml to 0.6.0 and rebuild bundles before tagging (CONTRIBUTING step; release job guard `MIN_ACTION_VERSION`).
- R3 workflow changes verified by actionlint + local smoke only — CI fix-up commit after merge.
- R4 8.6 left open until push/merge is authorized.
- R5 check-dist compares 5 generated files (plan said six) — none.
- R6 docs paths kept out of the `action` filter (spec "Docs-only change") — check-docs does not run on docs-only PRs.
- Task 6 ruling: release build kept in e2e (spec requires executables built from the commit) — a few minutes per e2e cell on main.
- R7 single readiness probe kept as specified — possible flaky Access readiness; fix would need a spec tweak.

---

## Contract deviations

- `scanLog(text, listen)` (internal): `<addr>` is matched unbracketed for IPv6 (`::1:15999`), as the CLI prints `listen_addr` exactly as written in config; the first draft said bracketed. Verified against the 0.5.0 binary.
- `createInstaller` options (internal): added optional `buildVersion` / `minVersion` (default `BUILD_VERSION` / `MIN_VERSION`) so tests can pin them.
- `Launcher.spawn` (internal): now returns `Promise<{ pid, exitCode() }>`; Node reports ENOENT/EACCES through a later `'error'` event, so a synchronous return could not carry the cause (Task 3 fix round 1).
- Serve output `home-relay` (public, owner decision before apply): removed, and Serve readiness is "status file has `node_id`", because the status file never reports the home relay (issue naicoi92/iroh-tunnel#177). Recorded in specs/design/plan before apply started.
- `move-major-tag.sh` (internal): added env `MIN_ACTION_VERSION` guard (skip moving `vX` for a stable tag below the first release that ships the actions); `release.yml` sets it to `0.6.0`, kept equal to `MIN_VERSION` in `action-src/build.ts` (final-review fix).

---

## Next step

Run `/opsx-verify`. After verify, task 8.6 needs owner authorization to push/merge so the PR `action` job and the main-branch `e2e` / `e2e-action-container` runs can be observed.
