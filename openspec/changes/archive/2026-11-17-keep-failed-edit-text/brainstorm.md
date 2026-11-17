## Design Summary

`config edit` (and the host `EditAttempt::commit` boundary behind it) loses the user's edited text when commit fails for a reason other than a conflict or an invalid document: a writer lock that is not acquired in time, a failure reading the live file, or a failure replacing it (issue #222). The scratch copy is removed before those steps run, and the error carries no copy of the text.

Living specs involved:

- `config-management-commands` "Rejected edits are kept in a sidecar" and "Edits conflicting with concurrent changes are rejected": the edited text goes to `<config>.rejected` (owner-only) and the error names that path. Unchanged by this change.
- `frontend-independent-management` "Edit is prepared and committed without holding the lock": commit keeps the `.rejected` sidecar on conflict or invalid input. Unchanged.
- `frontend-independent-management` "Abandoned edits leave no trace": "An edit attempt that fails or is dropped without commit MUST remove its scratch copy". **This change alters this requirement**: a failed commit removes the scratch copy only once the edited text is safe in the sidecar; when the sidecar cannot be written, the scratch copy is kept and named.

The agreed behaviour: when commit fails after the edited text was read, the text is written to `<config>.rejected` and the scratch copy is removed; the error names the sidecar. Only when writing the sidecar also fails is the scratch copy kept, and the error names it. Messages that point at a kept file remind the user to delete it once recovered, since it may contain secrets. Saved and unchanged edits, and attempts dropped without commit, still leave no scratch copy.

## Alternatives Considered

### Alternative A: Always keep the scratch copy on commit failure
- **Approach**: Do not remove the scratch copy when commit errors; name it in the error.
- **Pros**: Simplest code path; no extra write.
- **Cons**: Leaves a uniquely named `.edit-<pid>-<n>.toml` per failure, which accumulates and can hold secrets; recovery location differs from the conflict/invalid cases.
- **Why not chosen**: Contradicts "Abandoned edits leave no trace" in the common case and gives users two places to look.

### Alternative B: Reinterpret "fails" as editor failure only
- **Approach**: Keep the code of Alternative A and leave the spec text as is.
- **Pros**: No spec edit.
- **Cons**: The spec text still says a failed attempt removes its scratch copy; code and spec disagree.
- **Why not chosen**: Leaves a documented contradiction.

## Agreed Approach

Use the existing `.rejected` sidecar as the single recovery location for every commit failure, and fall back to keeping the scratch copy only when the sidecar cannot be written. Chosen by the user over A and B: it matches the existing sidecar contract, keeps one recovery path, and limits leftover files to the double-failure case.

## Key Decisions

- The sidecar is the recovery location for commit errors as well as rejections.
- The scratch copy survives only when the edited text exists nowhere else.
- Error messages say a file "still" holds the text only when that file exists.
- Messages that point at a kept file tell the user to delete it once recovered (it may contain secrets).
- Exit codes are unchanged: a lock timeout still exits 2.

## Open Questions

None.
