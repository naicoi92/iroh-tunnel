## Design Summary

Issue #240: `config set` cannot write relay settings, so operators must use `config edit` or hand-edit TOML to point a node at a self-hosted relay. Both roles already accept `-r/--relay-url` at run time (quick mode), but no config command can persist relays or the relay token.

This change makes every relay setting writable from the CLI, with one rule for every relay value: **a flag that is given replaces the stored value; a flag that is absent keeps it.**

Two scopes exist in the schema (`tunnel-configuration`), and the CLI keeps them apart:

```
 Scope         TOML location              serve  access  Command
 ------------  -------------------------  -----  ------  ----------------------------
 global        [node] relay_urls          yes    yes     <role> config node -r ...
 global        [node] relay_token         yes    yes     <role> config node --relay-token ...
 per-service   [[services]] relay_urls    no     yes     ita config set -n NAME -r ...
```

Each role command writes only its own role's config file; nothing crosses roles.

Behaviour:

- `<role> config node` (new, both roles) edits the `[node]` table:
  - `-r/--relay-url URL`, repeatable; the given list replaces `relay_urls` entirely, in the order given (first = home relay).
  - `--relay-token TOKEN` or `--relay-token-file PATH` (`-` reads stdin) replaces `relay_token`; the two are mutually exclusive.
  - `-r` without a token flag deletes `relay_token`. The token belongs to the relay set it was issued for, so a new set without a token means no token.
  - A token flag without `-r` replaces only the token and keeps the URLs. If the resulting `relay_urls` is empty, the command is rejected (exit 2): a token without custom relays is never used.
  - `--clear-relays` empties `relay_urls` and deletes `relay_token`, which returns the node to the n0 default relays. It conflicts with `-r` and with the token flags.
- `access config set -n NAME` gains:
  - `-r/--relay-url URL`, repeatable; replaces the service's own `relay_urls` override.
  - `--clear-relays`, which empties the override so the service falls back to `[node]` relays. It conflicts with `-r`.
- `serve config set` gets no relay flag: serve services have no per-service relay field. Passing `-r` stays a usage error (exit 2) with the `config set` help.
- Every relay URL must be `https://`, as the schema already requires. A bad URL fails with exit 2 naming the value, and the file is unchanged.
- Reports follow the `config set` style (`Updated ...` with `old -> new` lines, or `unchanged` with no rewrite). The token is never printed: it is shown as `<redacted>`, `(none)` or `<redacted> (changed)`.
- `docs/configuration.md` documents both commands and the replace rule.

Goals:
- Persist every relay setting without `config edit`.
- Make it obvious which scope a flag touches.
- Never leak the relay token through command output.

Non-goals:
- Changing run-time `-r` behaviour.
- Adding `max_concurrent_streams` to `config node`; the command name leaves room for it later.
- Per-service relay tokens; the schema has one node-level token.

Anti-goals:
- One flag that silently means different scopes in different roles.
- Appending to or merging relay lists: a partial list must never stay behind.

Living spec changes stated explicitly:
- `cli-entrypoints` "Usage errors print the failing command's help", scenario "Relay flag not accepted by config set": currently pins exit 2 for `access config set ... -r`. It is **reversed** for access, which now accepts `-r`, and kept for serve.
- `cli-entrypoints` "Config actions dispatch": gains `config node`.
- `cli-entrypoints` "Consistent short options across roles": `-r` keeps mapping to `relay-url` in the new commands. `--relay-token`, `--relay-token-file` and `--clear-relays` have no short form.
- `config-management-commands` "Set reports what changed": its field labels gain `relay_urls` for access. The rule "the report SHALL never include `[node]` values" still holds for `config set`.
- `config-management-commands`: a new requirement covers `config node`. Existing guarantees still apply to it: "Mutations are validated and all-or-nothing", "Config edits preserve formatting", "Concurrent writers are serialized", "Saves are atomic and owner-only".
- `tunnel-configuration`: no change; https validation and the per-service override already exist.

## Alternatives Considered

### Alternative A: `-r` on `config set` always writes `[node]`
- **Approach**: Add `-r` to `config set` for both roles, always targeting `[node] relay_urls`.
- **Pros**: Symmetric with run-time `-r`; one flag in one place.
- **Cons**: `config set` requires `-n NAME`, yet `-r` would ignore the named service and change every service. The access per-service override would remain unreachable.
- **Why not chosen**: It mixes scopes inside one service-scoped command, which is the confusion the user wants to avoid.

### Alternative B: `-r` writes wherever it fits per role
- **Approach**: serve `config set -r` writes `[node]`; access `config set -r` writes the service override.
- **Pros**: No new command; reaches the access override.
- **Cons**: The same flag on the same verb means global in one role and per-service in the other. There is no way to set access's global relays or the token.
- **Why not chosen**: Ambiguous scope and incomplete coverage.

### Alternative C: Separate scopes: `config node` for global, `config set -r` for per-service (chosen)
- **Approach**: A new `config node` edits `[node]` (relays and token) in both roles. `config set -r` edits only the access service override.
- **Pros**: Scope is explicit in the command; covers every relay field; leaves room for other `[node]` settings later.
- **Cons**: One more subcommand on the public CLI surface.
- **Why not chosen**: n/a, this was chosen.

## Agreed Approach

Alternative C. The user decided that each role writes only its own config and that global versus per-service must be explicit. A dedicated `config node` command makes the global scope visible, and `config set` stays service-scoped, as its required `-n` already implies.

## Key Decisions

- (user) Scope split: `config node` for `[node]`, `access config set -r` for the per-service override; serve `config set` has no relay flag.
- (user) Multiple relays by repeating `-r`; the given list replaces the stored list entirely.
- (user) Relay token follows the same rule: given replaces, absent keeps. `-r` without a token flag deletes the token.
- (user) Token input via `--relay-token TOKEN` and `--relay-token-file PATH|-`. Both are supported and mutually exclusive.
- (user) `--clear-relays` returns to the default relays: on `config node` it also deletes the token; on `access config set` it removes the service override.
- (user) A token flag that would leave `relay_urls` empty is rejected with exit 2.
- (user) Reports use the `Updated`/`unchanged` style; the token is always redacted.
- (my call) `config node` with no flags at all is a usage error (exit 2) with its help, matching how bare commands behave. Showing `[node]` is already covered by `config show`.
- (my call) `config node` on a missing config file creates the file with only `[node]`, the same way `config set` creates a missing file.
- (my call) Duplicate URLs within one `-r` list are rejected (exit 2) rather than silently deduplicated, so the stored list is exactly what the user typed.
- (my call) `--relay-token TOKEN` help warns that the value is visible in shell history and process lists and recommends `--relay-token-file`.

## Open Questions

None blocking. The exact report wording and token-file parsing (e.g. trimming one trailing newline) are left to design.
