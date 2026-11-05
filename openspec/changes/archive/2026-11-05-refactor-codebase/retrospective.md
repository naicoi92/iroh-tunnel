# Retrospective — refactor-codebase

## Wins

- The three-package split landed with no observable regressions on macOS. The fresh suite gives 547 passed, 0 failed, 9 ignored. The Task 2 CLI goldens never needed re-blessing, and 179 relocated tests are all mapped in `baseline/test-mapping.md`.
- Doing the in-package split first (19a and 20a, enforced by "no outward `crate::` paths" scanner tests) before the physical crate moves (19b and 20b) kept every intermediate commit green.
- Per-task reviews caught real defects before they landed:
  - the `cargo metadata` failure in CI being swallowed;
  - the signal-killed `ExitStatus` text changing;
  - the edit scratch-file collision across in-process callers;
  - a re-exec test helper that could pass without running its body.
- The `IROH_CHILD_BODY_RAN` sentinel added in Task 18 later caught renamed re-exec tests during Task 20a.

## Misses

- Task 14 timed out on its first single dispatch and had to be split into 14a and 14b/c. Tasks 19 and 20 were split up front after that.
- Tasks 4 and 5 skipped the RED step. RED evidence was enforced from Task 6 on.
- Linux and Windows `cfg` code was never compiled locally (the ring and aws-lc C cross toolchains are missing). Coverage of those paths depends on CI.
- Review Focus 5 (stop before ready) only became a deterministic test in the final fix wave. Its earlier `#[ignore]` version failed.

## Plan deviations

- One user-approved public change: `RetargetConsent.previous: Option<Pin>`.
- Additive host items:
  - `MigrationNotice` hook;
  - the `OnReady` bridge re-export;
  - read accessors from Task 20b;
  - the extra `ServiceNote` variants;
  - the `NativeOutput` `status_display` field.
- Root kept a minimal `[lib]`. Engine `settings` is a single flat file.
- Contract drift recorded at finalize: config DTO filesystem methods, the `testing` module shape, `Environment::detect` living in root, and status renderers staying in `status_cmd.rs`.

## Skill/workflow compliance

- Each task got a worker, then a reviewer, then a fix loop. All rulings are in the SDD ledger and summarised in the apply output.
- After the user asked for parallelism, work ran in parallel Worktrunk lanes with disjoint file sets, joined by cherry-pick plus a full-suite run.
- Every merge, push and PR was left to the user. AGENTS.md requires explicit chat approval for those.

## Surprises

- `Ownership::Unenforced` test seams and the process-global drain deadline were used more widely than the plan assumed.
- The macOS migration notice was the one host-side `eprintln!` with a spec-pinned stderr contract. It needed a hook rather than tracing.

## Promote candidates

- The "in-package module first, enforced by a scanner test, then physical crate move" pattern for future crate extractions.
- The re-exec test helper with a body-ran sentinel, as a shared test-support utility.
