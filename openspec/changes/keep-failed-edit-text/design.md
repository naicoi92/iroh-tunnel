## Context

`EditAttempt::commit` (crates/host/src/config/edit.rs) reads the scratch copy, takes `ConfigLock`, compares the live file with the original, validates and calls `write_atomic`. `EditAttempt` removes its scratch copy on drop (crates/host/src/config/types.rs). Conflicts and invalid documents already go to `<config>.rejected` through `keep_rejected`.

## Goals / Non-Goals

**Goals:** every commit failure after the edited text was read leaves the text in the sidecar or, failing that, in the scratch copy, named in the error. Saved, unchanged and dropped attempts leave no scratch copy.

**Non-Goals:** changing locking, validation, exit codes or `EditOutcome`.

## Decisions

1. **Split `commit` into a fallible `apply` plus a recovery step.** `apply(&self, edited)` returns `Result<EditOutcome>`; `commit` reads the scratch copy first, then on `Err` calls `keep_rejected` and wraps the error with the recovery sentence. This keeps the domain decision (where the text is now) in one place instead of sprinkling `keep_rejected` before every `?`.
2. **Drop removes the scratch copy unless a flag says the text lives nowhere else.** `keep_scratch: bool` on `EditAttempt` is set by `commit` only when the sidecar write failed. A failure to read the scratch copy keeps it when it still exists (the text may be there) and names it; a scratch copy the editor removed is reported as gone.
3. **"Still in" only when true.** Messages naming the scratch copy check that it exists; otherwise they say the text could not be kept.
4. **Error wrapping keeps classification.** The recovery sentence is added with `anyhow::Context`, so `CliError` downcasting in `src/app.rs` still yields exit code 2 for the lock timeout.

## Risks / Trade-offs

- [Kept files hold secrets] → the sidecar is owner-only via `write_atomic`; an editor may replace the scratch copy with umask permissions, so a kept scratch copy is set back to 0600 on Unix (best effort) before the error names it; messages remind the user to delete kept files.
- [Sidecar written outside the lock on the error path] → on a lock timeout the lock is never held; `write_atomic` renames atomically, so the worst case is last-writer-wins on `<config>.rejected`, the same as two rejected edits in a row.
- [Sidecar overwritten by a later failure] → same as the existing rejection behaviour; the message names it each time.
- [Root ignores directory modes in tests] → the read-only-directory test skips when run as root.
