## Context

Config mutations go through one path. `crates/host/src/config/mutate.rs::mutate_deciding` takes the writer lock, loads the `toml_edit` document (empty if absent), and runs a closure that returns `Edit::Apply` or `Edit::Keep`. On `Apply` only, it validates the whole document through the role schema (`NodeConfig::validate`, `AccessConfig` service checks) and writes atomically and owner-only. A `Keep` path is not validated; `set_node` validates it itself (D7). `set_service` and `keygen` are built on it. The CLI layer (`src/cli.rs`, `src/config_cmd.rs`, `src/render.rs`) maps clap args to a host request struct, calls the host operation and renders the outcome.

The schema already covers everything relay-related: `[node] relay_urls` and `relay_token` in both roles, `[[services]] relay_urls` in access only, and https and token whitespace validation. This change adds no schema rule except duplicate-URL rejection, which is enforced in the request (see D5).

## Goals / Non-Goals

**Goals:**
- One host operation per scope: `set_node` for `[node]`, and an extended `set_service` for the access override.
- Reuse `mutate_deciding` so validation, locking, atomic save, formatting preservation and the unchanged-no-rewrite behaviour come for free.
- Keep the token out of every output and diagnostic.

**Non-Goals:**
- Changing `NodeConfig` or `AccessService` DTOs, or the run-time quick path (`quick_node`).
- Generic `[node]` editing (`max_concurrent_streams`, `secret_key`; `keygen` stays the key path).

## Decisions

**D1. Request shape for `config node` uses intent enums, not raw options.** The mapping lives in `NodeArgs::into_request`.
`SetNode { config, relays: RelayEdit, token: TokenEdit }` with `RelayEdit::{Keep, Replace(Vec<String>), Clear}` and `TokenEdit::{Keep, Replace(String), Delete}`. The CLI layer derives them:
- `-r` given → `Replace(urls)`; with no token flag → `TokenEdit::Delete`.
- `--clear-relays` → `RelayEdit::Clear` + `TokenEdit::Delete`.
- Only a token flag → `RelayEdit::Keep` + `TokenEdit::Replace`.

This keeps the "absent `-r` keeps, `-r` without token deletes" rule in one tested function (`NodeArgs::into_request` in `src/cli.rs`), and the host operation stays a plain applier.
Alternative: pass `Option<Vec>`/`Option<String>` flags to the host and let it infer. Rejected because the inference would then be split across layers.

**D2. Clap shape.**
`ConfigAction::Node(NodeArgs)` with `NodeArgs { config, relay_url: Vec<String> (short 'r', Append), relay_token: Option<String>, relay_token_file: Option<PathBuf>, clear_relays: bool }`.
- `relay_token` conflicts with `relay_token_file`.
- `clear_relays` conflicts with `relay_url`, `relay_token` and `relay_token_file`.
- A required, multiple `ArgGroup` over exactly the four edit flags, declared on the `Node` variant; `NodeArgs` uses `#[group(skip)]` so derive adds no implicit group containing `config`. There is no `arg_required_else_help` on the leaf. Bare `config node` and `config node -c x` then both produce the same missing-argument usage error (`error:` line plus the `config node` help, exit 2). `arg_required_else_help` would print help without the `error:` line, which is inconsistent with `config node -c x`.

All of these are clap usage errors and print the `config node` help (cli-entrypoints "Usage errors print the failing command's help").

`ConfigAction` derives `Debug`, so a raw `relay_token: Option<String>` field would leak the token through `{:?}`. `NodeArgs` therefore has a manual redacting `Debug`.
Access `config set` needs `-r` (repeatable) and `--clear-relays` (conflicting). They cannot go on `AccessServiceArgs`, because that struct is also flattened into quick mode, where `AccessQuickArgs` already defines `-r`. They therefore live in a separate `AccessSetRelayArgs`, which is flattened only into `ConfigAction::Set` through an associated type `ServiceArgs::SetExtra: Args`. Serve uses an empty `NoSetExtra`, so serve `config set -r` stays an unknown-argument usage error. `ServiceArgs` is a public trait in a public module, so both types are `pub` with private fields, as `ServeServiceArgs` already is.

**D3. Token sources.**
`--relay-token-file PATH` reads the file; `-` reads all of stdin. One trailing `\n`/`\r\n` is trimmed, and nothing else is trimmed: the schema rejects any other whitespace with `invalid relay_token`. An unreadable or empty source fails with a config error naming the flag and path (never the contents), following `read_key_file`. Reading happens in the CLI layer before the lock, so a blocked stdin does not hold the writer lock.

**D4. Token without relays.**
After applying `RelayEdit`, if `TokenEdit::Replace` is set and the resulting `relay_urls` is empty, `set_node` returns a config error: `relay_token needs relay_urls; set them with -r`. The error is exit 2 without help, because it depends on file state and is not a parse error. It runs inside the closure, so the file is unchanged. This rule is enforced only by `config node`; the schema still accepts a stored token without URLs, so hand-edited files keep loading.

**D5. Duplicate URLs.**
Rejected in the host operation (both `set_node` and `set_service`) before writing: `duplicate relay_url '<url>'`, exit 2 without help. Only a `RelayEdit::Replace` input is checked; a kept list is not re-checked, and the rule is not added to `validate()`, so existing hand-written files with duplicates keep loading and still accept token-only edits.

**D6. Writing `[node]`.**
- Get or create the `[node]` table as `set_node_secret_key` does.
- `relay_urls` is written as an inline array via in-place value replacement, which keeps the decor and comments.
- `RelayEdit::Clear` removes the `relay_urls` key rather than writing `[]`, since absent equals empty in the schema. `TokenEdit::Delete` removes `relay_token`.
- An absent `[node]` with only `Keep`/`Delete` edits does not create an empty table.

Per-service `Clear` removes the service's `relay_urls` key. `Replace` on a service writes an inline array.

**D7. Outcome and report.**
`SetNodeOutcome::{Updated { path, changes: Vec<NodeChange> }, Unchanged { path }}`, with `NodeChange::{RelayUrls { old, new }, RelayToken(TokenChange)}` and `TokenChange::{Set, Replaced, Deleted}`. Token values are never carried in the outcome, so they cannot be rendered.

Rendering:
- `Updated node settings in <path>:`
- `  relay_urls: [a, b] -> [c]`, with `(default)` for an empty list.
- `  relay_token: (none) -> <redacted>` / `<redacted> -> <redacted> (changed)` / `<redacted> -> (none)`.
- `Node settings unchanged (<path>).`

The diff compares key presence as well as value: `Clear`/`Delete` on a present key (even `relay_urls = []`) is a change that removes the key. If nothing differs, the closure validates the unchanged document with `validate_role_doc` (as `mutate_deciding` validates only on `Apply`), so an invalid file still fails with exit 2. It then returns `Edit::Keep`, leaving bytes and mtime untouched.
Per-service: `ServiceField::RelayUrls` with label `relay_urls`, old/new rendered the same way (`(node)` for an absent override, meaning fallback to `[node]`).

**D8. Missing file.**
`config node` on a missing config creates the file containing only `[node]` (via `mutate_deciding`'s empty-document path), consistent with `config set`. When the edit stores nothing (`--clear-relays`, or Keep/Delete on absent keys), the closure returns `Edit::Keep` with `Unchanged`, so no file is created. Retired-field cleanup (`multiplex`) still runs on every real write, as for every other mutation.

## Risks / Trade-offs

- [Token in argv via `--relay-token`] → Help text warns about history and process lists and recommends `--relay-token-file`; docs repeat it.
- [`-r` silently deleting a token surprises a user who only wanted to add a relay] → The report always prints the `relay_token: <redacted> -> (none)` line; docs state the rule next to the flag.
- [`ServiceArgs::SetExtra` adds a generic to a shared trait] → Confined to `src/cli.rs`; it is the smallest seam that keeps quick-mode `-r` and set-mode `-r` from colliding while giving serve no `-r`.
- [Duplicate check differs between CLI writes and hand edits] → Documented; hand-edited duplicates stay accepted as today.

## Migration Plan

Additive CLI change; no config migration. Rollback is reverting the commit, and files written by the new commands stay valid for older binaries because the schema is unchanged.
