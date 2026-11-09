# Apply Receipt

> Generated at the end of the apply phase to mark code-implementation
> complete and provide verify with the state it needs.
> Overwritten on each apply iteration; iteration counter grows.

**Change**: `serve-readiness-waits-for-relay`
**Iteration**: `1`
**Applied at**: `2026-11-09T04:20Z`
**Executor**: `subagent-driven-development`

---

## Workspace

- **Worktree path**: `/Users/naicoi/Projects/iroh/iroh-tunnel.serve-readiness-waits-for-relay`
- **Worktree branch**: `serve-readiness-waits-for-relay`
- **Feature branch**: `issue-183`
- **Base branch**: `main`

---

## Commits

- **Range**: `6fcbdc6..c83d4b6` (this receipt commit follows c83d4b6)
- **Count**: `5` before the receipt commit

---

## Tasks

- **Completed**: `11 of 11` checkboxes in tasks.md flipped to `- [x]`
- **Remaining**: none

---

## Contract deviations

- none

---

## Lanes

| Task | Lane branch | Lane range | Review | Merge commit |
|---|---|---|---|---|
| Task 1 | sequential | `6fcbdc6..aa3501b` | Approved (1 Minor parked) | n/a (sequential) |
| Task 2 | sequential | `1bb25b9..2ea1681` | Approved | n/a (sequential) |
| Task 3 | sequential | none (controller-run E2E, tick only) | n/a (no code) | n/a (sequential) |

---

## Evidence

- `action-src`: `bun run typecheck` clean; `bun test` 157 pass / 0 fail (baseline 151); `bun run check-dist` ok; `bun run check-docs` → `check-docs: 2 files ok`.
- E2E 3.1 (release binary from this branch, default relays): `node serve/dist/main.js` exit 0 in 6 s, `node-id` written to `GITHUB_OUTPUT`, `serve-status.json` `home_relay: "https://aps1-1.relay.n0.iroh.link./"`; post step stopped pid 79443.
- E2E 3.2 (`INPUT_RELAY-URL=https://127.0.0.1:9/`): exit 1 at 60 s with `::error::iroh-tunnel-serve was not ready within 60 s. No home relay was registered; check the relay-url and relay-token inputs and outbound HTTPS access to the relay. Last status: {...,"home_relay":null}.`; `GITHUB_OUTPUT` empty; post step exit 0, pid 79642 gone.
- Limits: E2E ran on macOS arm64 only; Linux/Windows/container Serve steps not exercised (`.github/workflows/e2e.yml` runs them after merge, or via manual dispatch with approval).
- Final whole-branch review (code-reviewer): Ready to merge — Yes; 0 Critical, 0 Important.
- Parked: Minor — `action-src/test/usecases/start.test.ts` timeout assertions check substrings, not the order prefix → reason → Last status (implementation correct; E2E output shows the order).

---

## Next step

Run `/opsx-verify`.
