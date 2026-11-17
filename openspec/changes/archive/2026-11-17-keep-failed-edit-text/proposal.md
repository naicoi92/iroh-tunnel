## Why

`config edit` can lose the user's edited text when commit fails for a reason other than a conflict or an invalid document: a lock timeout, a failure reading the live file, or a failure replacing it (issue #222). The scratch copy is removed before those steps and the error keeps no copy, although the edit flow promises the text is not lost.

## What Changes

**Commit failure of an edit attempt**
- From: the scratch copy is removed as soon as it is read; a later lock, read or write error drops the edited text.
- To: on any commit failure after the text was read, the text is written to `<config>.rejected` (owner-only) and the error names it; the scratch copy is removed only once the text is saved, unchanged or in the sidecar, and is kept and named when the sidecar cannot be written.
- Reason: no edit may be lost silently.
- Impact: non-breaking. Exit codes are unchanged (lock timeout still exits 2). Error text gains the recovery path and a reminder to delete it once recovered.

Non-goals: no change to validation, conflict detection, locking or the editor launch; no cleanup of scratch copies left by older versions.

Security: kept files may contain `secret_key` and `relay_token`; both are owner-only, and messages tell the user to delete them once recovered.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `frontend-independent-management`: "Abandoned edits leave no trace" now allows a failed commit to keep the scratch copy when the edited text could not be written to the sidecar.
- `config-management-commands`: the sidecar also receives the edited text when commit fails on a lock timeout or an I/O error.

## Impact

- `crates/host/src/config/edit.rs`, `crates/host/src/config/types.rs` (`EditAttempt` drop behaviour).
- `tests/config_lifecycle.rs` (real-binary edit tests).
- No public type or CLI argument changes.
