## Design Summary

**Intent.**
- Pull requests run only unit tests (`ci.yml`). The end-to-end suite (`e2e.yml`) runs only after merge, on pushes to `main`. That suite covers the real tunnel tests on Linux, macOS and Windows, the Windows Task Scheduler lifecycle, the self-hosted relay on Linux, and the GitHub Action round trips.
- A change whose risk sits in OS-specific end-to-end code can therefore only be proven after it lands on `main`, and a red result leaves `main` red until a fix-forward lands. The motivating case is PR #179, whose `cfg(windows)` integration tests could not be compiled on the developer's macOS host.
- The operator wants to be able to run the end-to-end suite on demand against an unmerged branch, before merging.

**What changes.**
- `e2e.yml` gains a `workflow_dispatch` trigger. It takes no inputs: the branch or ref to test is chosen with the dispatch's own ref (`gh workflow run e2e.yml --ref <branch>` or the Actions UI).
- A dispatched run executes the same jobs as a push run, on the dispatched ref's commit: `e2e (linux|macos|windows)` and `e2e (action in container)`.
- The aggregate check of a dispatched run is named **`e2e-ok (manual)`**, not `e2e-ok`. The release preflight reads only `e2e-ok`, so a manual run can never stamp the check that a release trusts.
- The push-to-`main` behaviour is unchanged: same jobs, an always-present `e2e-ok` on every merge commit, and no cancellation between commits.
- The `ci-cd-pipeline` spec's "none by manual dispatch" rule is relaxed for `e2e.yml` only. `ci.yml`, `cache.yml` and `release.yml` stay non-dispatchable.
- `CONTRIBUTING.md` and the `e2e.yml` header comment gain a short "run e2e before merge" note.

**Goals.**
- Run the full e2e matrix on any branch on demand.
- Keep the release gate exactly as strong as today.

**Non-goals.**
- Running e2e automatically on pull requests (cost and time).
- Dispatch inputs (PR number, OS subset).
- Changing what the e2e jobs test.
- Making other workflows dispatchable.

**Anti-goals.**
- A manual run that produces or overwrites `e2e-ok` on any commit.
- A dispatched run that cancels, or is cancelled by, a push run on the same commit.
- A dispatched run gaining extra permissions or secrets.

**Constraints.**
- The release preflight must continue to read only `e2e-ok` for the tagged SHA.
- The workflow inventory stays at four files.
- `actionlint` must pass in `ci.yml`'s `scripts` job.

## Alternatives Considered

### Alternative A: Plain `workflow_dispatch`, same `e2e-ok` name
- **Approach**: Add the trigger and change nothing else.
- **Pros**: The smallest diff.
- **Cons**: A dispatch on a commit later merged by fast-forward, or on `main` itself, stamps `e2e-ok` from a manual run. Release trust then depends on how the run was started.
- **Why not chosen**: It weakens the meaning of `e2e-ok`. The operator chose a distinct name.

### Alternative B: `workflow_dispatch` on `main` only
- **Approach**: Allow dispatch only when the ref is `main`, so e2e can be re-run on main.
- **Pros**: Release semantics are untouched.
- **Cons**: It does not let an unmerged branch be tested, which is the whole point.
- **Why not chosen**: It misses the goal.

### Alternative C: Temporary preview branch with a push trigger
- **Approach**: Push a throwaway branch carrying a one-off trigger edit.
- **Pros**: No spec change.
- **Cons**: It is a manual workaround each time, and the trigger edit is committed to a branch that never lands.
- **Why not chosen**: The operator wants a supported, repeatable path.

### Alternative D: Dispatch with a distinct aggregate name (chosen)
- **Approach**: As in the Design Summary.
- **Pros**: It reaches the goal, keeps `e2e-ok` meaning "push run on `main`", and needs no inputs.
- **Cons**: The aggregate job name depends on the event, and the spec changes.
- **Why chosen**: See Agreed Approach.

## Agreed Approach

Alternative D, chosen by the operator. Dispatch is allowed on any ref, without inputs, and the aggregate check of a manual run is `e2e-ok (manual)`. The release gate stays bound to push runs on `main`.

## Key Decisions

- **Dispatch scope (operator).** Any branch, chosen with the dispatch ref. There are no inputs.
- **Aggregate name (operator).** Manual runs report `e2e-ok (manual)`. Push runs keep `e2e-ok`.
- **Same jobs (my call).** A dispatched run executes exactly the jobs a push run does, so its result is a faithful preview of the post-merge run.
- **Concurrency (my call).** Runs are grouped per event and commit, so a manual run and the push run for the same SHA never cancel each other. Runs are still never cancelled across commits.
- **Permissions (my call).** Permissions are unchanged (`contents: read`). Dispatch is limited to people with write access, which is GitHub's default for `workflow_dispatch`.
- **Spec scope (my call).** Only `ci-cd-pipeline` changes: the inventory requirement allows manual dispatch for `e2e.yml`, and a new requirement covers manual end-to-end runs. The release-preflight requirement gains a scenario stating that `e2e-ok (manual)` is ignored.

## Open Questions

- None blocking. The exact mechanism for the event-dependent job name, an expression in `name:`, is settled in design.
