# Retrospective: github-actions-serve-access

## Wins
- Verifying binary behaviour against the real 0.5.0 release before writing the plan caught a bug in iroh-tunnel itself (`home_relay` never reported, issue #177) and the fact that Access keeps running after a listener bind failure; both shaped the readiness design instead of surfacing in CI.
- Per-task review found real defects early: an uncaught spawn `'error'` event, a no-secret test that could never fail, a post step that skipped cleanup on errors, a relay token printed in the post step, a release job that could move `v0` below the first action release.
- Local smoke with real binaries over n0 relays gave end-to-end evidence (TCP and UDP) before any runner was involved.

## Misses
- Committed bundles contained the build machine's absolute path (bun injects `__dirname` for `@actions/tool-cache`). Every check ran in the same directory that built them, so `check-dist` passed until finalize re-ran it on the feature branch. Fixed in apply iteration 2; a reproducibility check from a clean `git archive` copy is now part of the evidence.
- The plan miscounted generated files and had a few signature mismatches (`resolveTarget` arity, `Launcher.spawn` sync vs async); caught by the plan reviewer or during tasks.

## Plan deviations
- Serve output `home-relay` and the relay wait were dropped (owner decision, issue #177).
- Internal contracts changed: `scanLog` matches unbracketed IPv6, `Launcher.spawn` is async, `createInstaller` gained test options, `move-major-tag.sh` gained a `MIN_ACTION_VERSION` guard.
- `build.ts` gained `normalizeBundle` (iteration 2).

## Skill/workflow compliance
- Subagent-driven execution with one implementer and one reviewer per task, a whole-branch review and one fix wave; rulings ledgered (R1–R7 plus task rulings) and copied into `apply.md`.
- Finalize stopped before any push when a check failed, and the fix went back through apply and verify.

## Surprises
- The design pivoted three times during planning (service-container mode dropped, one action with `role` split into two actions, Access made the root/Marketplace action).
- `gh` repeatedly switched back to another account; checked and switched before each remote operation.

## Promote candidates
- In TS action projects, run `check-dist` from a clean `git archive` copy as part of the standard verify evidence.
- Fix issue #177 so Serve can wait for relay registration and expose a home relay output again.
