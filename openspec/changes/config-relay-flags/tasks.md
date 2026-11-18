## 1. Host: `set_node` operation for `[node]` relay settings

Blocked by: none

Files: `crates/host/src/config/set_node.rs` (new), `crates/host/src/config/relay_list.rs` (new), `crates/host/src/config/types.rs`, `crates/host/src/config/mod.rs`, `crates/host/src/config/store.rs` (extract `node_table_mut` from `set_node_secret_key`).

- [x] 1.1 Add request/outcome types in `types.rs`: `SetNode { config, relays: RelayEdit, token: TokenEdit }`, `RelayEdit::{Keep, Replace(Vec<String>), Clear}`, `TokenEdit::{Keep, Replace(String), Delete}`, `SetNodeOutcome::{Updated { path, changes }, Unchanged { path }}`, `NodeChange::{RelayUrls { old, new }, RelayToken(TokenChange)}`, `TokenChange::{Set, Replaced, Deleted}`. The outcome never carries token text (design D1, D7).
- [x] 1.2 Write failing unit tests in `set_node.rs` for the scenarios "Replace relay list", "Relay change without token deletes the token", "Relay and token replaced together", "Token only keeps relays", "Clear relays", "Services and comments preserved", "Missing file", "Clear on a missing file", "Retired fields dropped", "Duplicate relay URL", "Token without relays", "Insecure relay URL", "Node edit on a file with an invalid service" and "Nothing changed" (bytes and mtime kept).
- [x] 1.3 Implement `set_node(role, &SetNode) -> Result<SetNodeOutcome>` on `mutate_deciding`. It gets the `[node]` table without creating an empty one for no-op edits, replaces values in place keeping decor, removes keys on Clear/Delete, rejects duplicates and token-without-relays inside the closure, and returns `Edit::Keep` when nothing differs (design D4–D6, D8).
- [x] 1.4 Export `set_node` and the new types from `config/mod.rs`; `cargo test -p iroh-tunnel-host` passes.

## 2. Host: per-service relay override in `set_service`

Blocked by: 1

Files: `crates/host/src/config/set_service.rs`, `crates/host/src/config/types.rs` (`SetService`, `ServiceField` only), `crates/host/src/config/fixtures.rs`.

- [ ] 2.1 Extend `SetService` with `relays: RelayEdit` (serve requests always `Keep`), and `ServiceField::RelayUrls` with label `relay_urls`. Reuse `RelayEdit` from group 1.
- [ ] 2.2 Write failing unit tests for "Set override on existing service", "Replace override", "Clear override", "Override kept without relay flags", "Create with override", "Insecure override", "Duplicate override URL" and "Relay override report" (old shown as `(node)` when absent, list as `[a, b]`).
- [ ] 2.3 Implement: on create, insert `relay_urls` when `Replace`; on update, replace or remove the key and push a `FieldChange`; reject duplicates before writing. `[node]` is never touched.
- [ ] 2.4 `cargo test -p iroh-tunnel-host` passes.

## 3. CLI: `config node` command and access `config set` relay flags

Blocked by: 1, 2

Files: `src/cli.rs`, `src/app.rs`, `src/config_cmd.rs`, `src/render.rs`.

- [ ] 3.1 Add `ConfigAction::Node(NodeArgs)` (`config`, `relay_url: Vec<String>` (-r, append), `relay_token`, `relay_token_file`, `clear_relays`; redacting `Debug`) with the conflicts and the required-one-of group from design D2. `--relay-token` help warns about shell history and process lists and recommends `--relay-token-file`. Update the `Config` about text to list `node`.
- [ ] 3.2 Add the `ServiceArgs::SetExtra` associated type, with `AccessSetRelayArgs { relay_url (-r, append), clear_relays }` for access and an empty args struct for serve, flattened only into `ConfigAction::Set`. Thread it into `into_set` so access maps to `RelayEdit` and serve always `Keep`.
- [ ] 3.3 Implement `NodeArgs::into_request` (pure flag-to-intent mapping, design D1) and token source reading (file, `-` stdin, trim one trailing `\n`/`\r\n`, empty/unreadable → config error naming the flag and path, design D3), read before the lock. Unit-test the mapping table and the CRLF / extra-newline / empty cases.
- [ ] 3.4 Add `config_cmd::node` and `render::render_node` (`Updated node settings in <path>:` with `relay_urls` / `relay_token` lines, `(default)`, `(none)`, `<redacted>`, `(changed)`; `Node settings unchanged (<path>).`). Extend `render_set` for `relay_urls`. Add unit tests in `render.rs`, and wire `ConfigAction::Node` in `src/app.rs`.
- [ ] 3.5 Add an `Examples:` block for `config node` in `examples()` (both roles), and an access `config set` example with `-r`.
- [ ] 3.6 Replace unit test `config_set_has_no_relay_flag` in `src/cli.rs` with: serve `config set` has no `relay-url`, access `config set` has `relay-url` (short `r`) and `clear-relays`, `config node` exists on all three binaries. Extend the short/long-only option test with `relay-token`, `relay-token-file`, `clear-relays` (scenarios "Short letters do not collide", "Long-only options").

## 4. Integration tests, golden output and docs

Blocked by: 3

Files: `tests/cli_usage.rs`, `tests/config_relay.rs` (new, portable), `tests/cli_golden.rs`, `tests/fixtures/golden/*`, `tests/config_lifecycle.rs`, `docs/configuration.md`.

- [ ] 4.1 `tests/cli_usage.rs`: change `relay_flag_rejected_on_set` to serve only with `-r https://relay.example` on `it` and `its` (scenario "Relay flag not accepted by config set"). Add tests for "Conflicting relay flags" (all four pairs), "Conflicting override flags on access config set", "Node command without an edit flag" (no file created) and "Node command on every executable".
- [ ] 4.2 `tests/config_relay.rs` (portable, runs on Unix and Windows) end-to-end through the binary, plus a `config node` case in `tests/config_lifecycle.rs::invalid_mutations_fail_and_leave_file_bytes_unchanged`: "Role isolation", "Token from file", "Token from stdin", "Unreadable token file", "Empty token source", "Token with whitespace", "Insecure relay URL" (exit 2, no `Usage:`, bytes unchanged), and a token-leak check asserting `t1`/`t2`/`secret-token` never appear in stdout or stderr ("Change report", "Token replaced").
- [ ] 4.3 `tests/cli_golden.rs`: add `config_node_updated_report`, `config_node_unchanged` and `config_set_relay_override_report`; bless with `GOLDEN_BLESS=1` and review the new fixture files by hand.
- [ ] 4.4 `docs/configuration.md`: document `config node` in "Managing the file with `config`", and the replace/keep/delete rule, `--clear-relays`, the token sources with the argv warning, and the access `config set -r` override in "Relays".
- [ ] 4.5 Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace`; all pass.
