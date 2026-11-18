## Why

Relay settings cannot be written with the CLI (issue #240). Both roles take `-r/--relay-url` at run time, but no `config` command can persist `[node] relay_urls`, `[node] relay_token` or the access per-service `relay_urls` override. Operators of self-hosted relays have to fall back to `config edit` or hand-edit TOML. Giving each relay scope its own explicit flag makes relay setup scriptable and keeps it inside the validated, atomic config mutation path.

## What Changes

**New `config node` command (both roles)**
- Edits the `[node]` table of the invoking role's config only.
- `-r/--relay-url URL` (repeatable) replaces `relay_urls` with the given list, in order.
- `--relay-token TOKEN` or `--relay-token-file PATH|-` (mutually exclusive) replaces `relay_token`.
- `-r` without a token flag deletes `relay_token`. A token flag without `-r` keeps `relay_urls`, and is rejected when the resulting `relay_urls` is empty.
- `--clear-relays` empties `relay_urls` and deletes `relay_token` (back to the default n0 relays). It conflicts with `-r` and the token flags.
- Reports `Updated node settings in <path>:` with `old -> new` lines, or an unchanged message without rewriting. The token is always redacted.

**`config set` relay flag**
- From: `config set -r` is a usage error in both roles (scenario "Relay flag not accepted by config set").
- To: `access config set -n NAME -r URL [-r URL ...]` replaces that service's `relay_urls` override; `--clear-relays` removes the override. `serve config set` still rejects `-r`.
- Reason: The access schema already has a per-service override that the CLI cannot reach.
- Impact: Additive for access; serve unchanged.

**Validation**
- Relay URLs must be `https://` (existing schema rule). Duplicate URLs in one list are rejected. Failures exit 2 naming the value and leave the file unchanged.

Non-goals: run-time `-r` behaviour, `max_concurrent_streams` in `config node`, per-service relay tokens.

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `cli-entrypoints`: `config node` joins the config subcommands; `access config set` accepts `-r`, and the "Relay flag not accepted by config set" scenario is narrowed to serve.
- `config-management-commands`: new requirements for `config node` (replace and keep rules, token deletion, clear, report, redaction) and for the access per-service relay override in `config set`; the `config set` change report gains a `relay_urls` label.

## Impact

- Code: `src/cli.rs` (new `ConfigAction::Node`, access `config set` relay args), `src/config_cmd.rs` and `src/render.rs` (handler, report), `crates/host/src/config/` (new `set_node` operation, `set_service` relay field, types).
- Tests: `config_set_has_no_relay_flag` in `src/cli.rs` is replaced; golden help in `tests/cli_golden.rs` / `tests/cli_usage.rs` is updated; new integration tests in `tests/config_lifecycle.rs`.
- Docs: `docs/configuration.md` (Relays section, command reference).
- Public CLI: additive; no existing invocation changes meaning. Serve `config set -r` stays exit 2.
- Security: `--relay-token TOKEN` exposes the token in shell history and process lists. Its help says so and points to `--relay-token-file`. The token never appears in output or diagnostics.
- No dependency, schema or migration change.
