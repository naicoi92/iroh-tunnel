## Design Summary

Small change. Host service status reports the role's owner as a fact. Today, when the role is not running, that fact reads "unavailable" with the message `"<role> is not running"`. That mixes a normal state (nothing runs, so there is no owner) with a real gap (the role runs but its owner record cannot be read). A frontend can tell them apart only by comparing strings. This change makes "no owner because the role is not running" a known, empty owner. "Unavailable" is kept for owner metadata that cannot be determined.

- **Why now:** issue #165. It must land before a TUI or Tauri frontend reads this field, because changing it later breaks those consumers.
- **Goals:** a frontend can tell "not running" from "owner unreadable" without parsing text. A host unit test covers both cases.
- **Non-goals:** changing CLI output (the existing goldens must pass unchanged); changing how liveness is probed; changing the owner record file.
- **Anti-goals:** reporting a not-running role as an error or as unavailable; failing a status call because owner metadata is missing.

Living spec involved: `frontend-independent-management`.
- *Service status separates facts without inventing truth*: "unavailable facts explicit". This change applies it more precisely and does not change its intent.
- *Optional status facts do not cause failure*, scenario *Owner metadata unavailable* (role live, owner unreadable → unavailable, succeeds). Unchanged.
- **Change to specified behaviour:** a new scenario adds that a role that is not running reports its owner as known and absent, not as unavailable. Today that case is not specified.

## Alternatives Considered

### Alternative A: Known-and-absent owner
- **Approach**: The owner fact holds an optional owner. Not running → known, none. Running and readable → known, the record. Running and unreadable, or probe failed → unavailable.
- **Pros**: The state lives in the type, so there is no string convention. Small change. No CLI impact, because no CLI path reads this field.
- **Cons**: Changes the host crate's public API, though it has no external consumers yet.
- **Why not chosen**: Chosen.

### Alternative B: Keep the shape, document the string
- **Approach**: Leave the field unchanged and document that `"<role> is not running"` means "no owner".
- **Pros**: No code change.
- **Cons**: Frontends must string-match, which goes against the spec's "unavailable facts explicit". It is fragile if the wording changes.
- **Why not chosen**: It keeps the ambiguity the issue asks to remove.

### Alternative C: Drop the owner field
- **Approach**: Remove the field; frontends read the owner from the liveness fact, which already carries an optional owner.
- **Pros**: No duplicated data.
- **Cons**: Loses the explicit "owner unreadable" signal next to liveness. It is a larger public API change than the issue asks for. The spec lists owner metadata as its own field.
- **Why not chosen**: It is bigger than needed and conflicts with the spec's separate-fields requirement.

## Agreed Approach

Alternative A, chosen by the user in the Inbox thread for #165. It removes the string convention at the lowest cost and matches the spec's requirement that unavailable facts be explicit.

## Key Decisions

- Not running → owner known and absent (not unavailable).
- Running with an unreadable or missing owner record → owner unavailable (unchanged).
- Liveness probe failed → owner unavailable (unchanged).
- CLI output does not change; the existing goldens pass as they are.
- The decision is recorded on issue #165 (comment posted only with the user's permission).

## Open Questions

None.
