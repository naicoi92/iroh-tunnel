# Config Relay Flags Implementation Plan

> Executed via `/opsx-apply`, which chooses the executor.
> Spec reference: this change's `specs/` (`specs/cli-entrypoints/spec.md`, `specs/config-management-commands/spec.md`) and `design.md`.

**Goal:** Let operators write relay settings with the CLI: `[node]` relays and token via a new `<role> config node`, and the access per-service override via `access config set -r`.

**Architecture:** Two terminal-free host operations in `crates/host/src/config/` (`set_node` new, `set_service` extended) are built on `mutate_deciding`, so locking, schema validation, atomic save and retired-field cleanup are reused. The CLI layer (`src/cli.rs`) maps flags to intent enums (`RelayEdit`, `TokenEdit`) and reads token sources before the lock. `src/render.rs` renders the outcomes, which never carry token text.

**Tech Stack:** Rust, clap 4.6 derive, toml_edit, cargo test.

## Global Constraints

- Relay URLs must start with `https://`; the error text is the schema's `invalid relay_url '<url>'` (`crates/host/src/config/dto.rs`).
- Error classes:
  - clap parse errors → exit 2 with the deepest subcommand help;
  - `SetError::MissingForCreate` stays a usage error with the `config set` help (unchanged);
  - every other new failure is `CliError::Config` → exit 2, `error: <message>`, no help.
- The token value never appears in stdout, stderr, any `Debug` output (including the parsed `Cli`) or any outcome type.
- `--relay-token`, `--relay-token-file`, `--clear-relays` have no short form; `-r` is `relay-url` everywhere.
- Report formats:
  - headers `Updated node settings in <path>:` / `Node settings unchanged (<path>).`;
  - lines `  <field>: <old> -> <new>`;
  - list `[a, b]`; empty node list `(default)`; absent service override `(node)`;
  - token `<redacted>` / `(none)`; replaced token `<redacted> -> <redacted> (changed)`.
- No schema/DTO change; no new dependency.

## Review Focus

1. Token leaking through an error path or `Debug` → Task 3 `parsed_cli_debug_redacts_token`; Task 4 `config_node_never_prints_token` and the "Token file with extra line" leak assertion.
2. `[node]` written as an inline table (`node = { relay_urls = [] }`) → Task 1 `inline_node_table`.
3. `--relay-token-file -` with a closed stdin must fail as empty, never hang → Task 4 `config_node_stdin_closed_is_empty_error` (`Stdio::null()`).
4. Re-running an identical `config node` must report unchanged and still fail on an invalid file → Task 1 `same_token_is_unchanged`, `invalid_service_blocks_noop_edit`.
5. Clearing an explicit `relay_urls = []` must remove the key → Task 1 `clear_removes_explicit_empty_array`; Task 2 `clear_override_removes_explicit_empty_array`.

---

## Contracts

| Name | new / modified | Shape | public |
|---|---|---|---|
| `RelayEdit` | new | `pub enum RelayEdit { #[default] Keep, Replace(Vec<String>), Clear }`, derives `Debug, Clone, PartialEq, Eq, Default`; in `crates/host/src/config/types.rs` | yes (host crate API) |
| `TokenEdit` | new | `pub enum TokenEdit { Keep, Replace(String), Delete }`, derives `Clone, PartialEq, Eq`; manual `Debug` printing `Replace(<redacted>)` | yes (host crate API) |
| `SetNode` | new | `pub struct SetNode { pub config: Option<PathBuf>, pub relays: RelayEdit, pub token: TokenEdit }`, derives `Debug, Clone, PartialEq, Eq` | yes (host crate API) |
| `SetNodeOutcome` | new | `pub enum SetNodeOutcome { Updated { path: PathBuf, changes: Vec<NodeChange> }, Unchanged { path: PathBuf } }` | yes (host crate API) |
| `NodeChange` | new | `pub enum NodeChange { RelayUrls { old: Vec<String>, new: Vec<String> }, RelayToken(TokenChange) }`; order relay_urls, then relay_token | yes (host crate API) |
| `TokenChange` | new | `pub enum TokenChange { Set, Replaced, Deleted }` | yes (host crate API) |
| `set_node` | new | `pub fn set_node(role: Role, req: &SetNode) -> anyhow::Result<SetNodeOutcome>` in `crates/host/src/config/set_node.rs` | yes (host crate API) |
| `relay_list` helpers | new | `crates/host/src/config/relay_list.rs`, `mod relay_list;` in `mod.rs`: `pub(super) fn ensure_unique(urls: &[String]) -> anyhow::Result<()>` (`CliError::Config("duplicate relay_url '<url>'")`), `pub(super) fn display_list(urls: &[String], empty: &str) -> String`, `pub(super) fn read_list(table: &dyn toml_edit::TableLike) -> Option<Vec<String>>` (`None` = key absent), `pub(super) fn write_list(table: &mut dyn toml_edit::TableLike, urls: &[String])` (in place, keeps decor) | no (host-internal) |
| `node_table_mut` | new | `pub(crate) fn node_table_mut<'a>(doc: &'a mut DocumentMut, path: &Path) -> anyhow::Result<&'a mut dyn toml_edit::TableLike>` in `store.rs`, extracted from `set_node_secret_key`, which then calls it | no |
| `SetService` | modified | adds `pub relays: RelayEdit` | yes (host crate API) |
| `ServiceField` | modified | adds `RelayUrls`; label `relay_urls` for both roles | yes (host crate API) |
| `<role> config node` | new | `config node [-c PATH] [-r URL]... [--relay-token TOKEN \| --relay-token-file PATH\|-] [--clear-relays]`; required multiple group over the four edit flags; no `arg_required_else_help`; `--clear-relays` conflicts with the other three; the two token flags conflict | yes (CLI) |
| access `config set` relay args | new | `-r/--relay-url URL` (repeatable, append), `--clear-relays` (conflicts with `-r`) | yes (CLI) |
| `NodeArgs` | new | `pub struct NodeArgs { pub(crate) config: Option<PathBuf>, pub(crate) relay_url: Vec<String>, pub(crate) relay_token: Option<String>, pub(crate) relay_token_file: Option<PathBuf>, pub(crate) clear_relays: bool }`, `#[derive(Args)]` with `#[group(skip)]`; manual `Debug` redacting `relay_token`; in `src/cli.rs` | yes (lib `cli` module) |
| `ServiceArgs::SetExtra` | new | `type SetExtra: clap::Args + clap::FromArgMatches + std::fmt::Debug;`; `fn into_set(self, extra: Self::SetExtra, config: Option<PathBuf>) -> anyhow::Result<SetService>` | yes (lib `cli` module) |
| `AccessSetRelayArgs` / `NoSetExtra` | new | `pub struct AccessSetRelayArgs { relay_url: Vec<String>, clear_relays: bool }`; `pub struct NoSetExtra {}`; both `#[derive(Args, Debug)]`, private fields | yes (lib `cli` module, required by E0446) |
| `ConfigAction::Set` | modified | `Set { #[command(flatten)] service: S, #[command(flatten)] extra: S::SetExtra, config: Option<PathBuf> }` | yes (lib `cli` module) |
| `ConfigAction::Node` | new | `Node(NodeArgs)` | yes (lib `cli` module) |
| `NodeArgs::into_request` | new | `pub(crate) fn into_request(self, token: Option<String>) -> SetNode` in `src/cli.rs` (token already resolved) | no |
| `config_cmd::set` | modified | `pub fn set<S: ServiceArgs>(args: S, extra: S::SetExtra, config: Option<PathBuf>) -> Result<()>` | no |
| `config_cmd::node` | new | `pub fn node(role: Role, args: NodeArgs) -> Result<()>` | no |
| `read_token_source` | new | `fn read_token_source(path: &Path, stdin: impl std::io::Read) -> anyhow::Result<String>` in `src/config_cmd.rs`; `-` reads `stdin` | no |
| `render_node` | new | `pub(crate) fn render_node(outcome: &SetNodeOutcome) -> String` in `src/render.rs` | no |

---

## Task 1: Host `set_node` — implements 1

**Covers:** 1.1, 1.2, 1.3, 1.4
**Difficulty:** Normal
**Blocked by:** none

**Files:**
- Create: `crates/host/src/config/set_node.rs`, `crates/host/src/config/relay_list.rs`
- Modify: `crates/host/src/config/types.rs`, `crates/host/src/config/mod.rs`, `crates/host/src/config/store.rs` (extract `node_table_mut`)

**Interfaces:**
- Produces: `RelayEdit`, `TokenEdit`, `SetNode`, `SetNodeOutcome`, `NodeChange`, `TokenChange`, `set_node`, `relay_list` helpers, `node_table_mut`.

- [ ] **Step 1: Write failing tests.** In `relay_list.rs`: `ensure_unique_rejects_duplicate` (err contains `duplicate relay_url 'https://a.example'`), `display_list_formats` (`[a, b]`, and the `empty` argument for `[]`). In `set_node.rs` `#[cfg(test)] mod tests`, using `tempfile` like sibling modules, each writes `c.toml`, calls `set_node`, and asserts on the re-parsed file and the outcome:
  - `replace_relay_list`: `["https://old.example"]` + `Replace([a, b])` → `relay_urls == [a, b]`; `changes == [RelayUrls{old:[old], new:[a,b]}]`.
  - `relays_without_token_delete_token`: token `t1`, `Replace([c])` + `Delete` → no `relay_token` key; changes end with `RelayToken(Deleted)`.
  - `relays_and_token_replaced`: `Replace([a])` + `Replace("t2")` → `relay_token == "t2"`, `RelayToken(Replaced)`.
  - `token_only_keeps_relays`: relays `[a]`, no token, `Keep` + `Replace("t3")` → relays unchanged, `RelayToken(Set)`.
  - `token_only_with_existing_duplicates_is_accepted`: relays `[a, a]`, `Keep` + `Replace("t")` → Ok (duplicates are checked only on `Replace` input).
  - `clear_relays_keeps_secret_key`: `Clear` + `Delete` → neither key; `secret_key` unchanged.
  - `clear_removes_explicit_empty_array`: `relay_urls = []`, `Clear` + `Delete` → `Updated`, key absent.
  - `services_and_comments_preserved`: a service and `# operator comment` → both present after `Replace([a])`.
  - `missing_file_created_with_node_only`: absent path + `Replace([a])` → file parses to only `[node]` with `relay_urls`.
  - `clear_on_missing_file_is_unchanged`: absent path + `Clear` + `Delete` → `Unchanged`, `!path.exists()`.
  - `retired_multiplex_dropped`: access service with `multiplex = true` → key gone after `Replace([a])`.
  - `duplicate_url_rejected`: `Replace([a, a])` → err contains `duplicate relay_url 'https://a.example'`; bytes unchanged.
  - `token_without_relays_rejected`: no relays + `Keep` + `Replace("t")` → err contains `relay_token needs relay_urls`; bytes unchanged.
  - `insecure_url_rejected`: `Replace(["http://relay.example"])` → err contains `invalid relay_url 'http://relay.example'`; bytes unchanged.
  - `invalid_service_blocks_node_edit`: service `BAD` + `Replace([a])` → err; bytes unchanged.
  - `invalid_service_blocks_noop_edit`: service `BAD`, relays already `[a]`, `Replace([a])` + `Delete` → err (not `Unchanged`); bytes unchanged.
  - `unchanged_keeps_bytes_and_mtime`: relays `[a]`, no token, `Replace([a])` + `Delete` → `Unchanged`; bytes and mtime equal.
  - `same_token_is_unchanged`: relays `[a]`, token `T`, `Replace([a])` + `Replace("T")` → `Unchanged`.
  - `inline_node_table`: `node = { relay_urls = [], secret_key = "" }` + `Replace([a])` → Ok; `secret_key` key still present.
  - `token_edit_debug_redacts`: `format!("{:?}", TokenEdit::Replace("s3cret".into()))` lacks `s3cret`.
- [ ] **Step 2: Run** `cargo test -p iroh-tunnel-host` → FAIL to compile (items missing).
- [ ] **Step 3: Add the types** to `types.rs` and the helpers to `relay_list.rs` per Contracts. Extract `node_table_mut` in `store.rs`; `set_node_secret_key` calls it, with no behaviour change.
- [ ] **Step 4: Implement `set_node`** as a thin `mutate_deciding` closure over three private functions in `set_node.rs`:
  - `fn current(doc: &DocumentMut) -> (Option<Vec<String>>, Option<String>)`: reads `[node]` without creating it.
  - `fn plan(current, req: &SetNode) -> anyhow::Result<NodePlan>`: `ensure_unique` on `Replace` input only; token-needs-relays check; diff by presence and value; returns target keys plus `Vec<NodeChange>`.
  - `fn apply(doc, path, &NodePlan) -> anyhow::Result<()>`: `node_table_mut`, then `write_list` or remove, then set or remove the token.

  When `changes` is empty, call `mutate::validate_role_doc(role, &doc.to_string(), path)` and return `Edit::Keep(Unchanged)`. Otherwise `apply` and return `Edit::Apply(Updated)`.
- [ ] **Step 5: Export** from `mod.rs`: `mod relay_list; mod set_node; pub use set_node::set_node;` plus the new types in the `types` re-export.
- [ ] **Step 6: Run** `cargo test -p iroh-tunnel-host` → PASS; `cargo clippy -p iroh-tunnel-host --all-targets -- -D warnings` → clean.
- [ ] **Step 7: Commit** `feat(config): add set_node for [node] relay settings`.

---

## Task 2: Host per-service relay override — implements 2

**Covers:** 2.1, 2.2, 2.3, 2.4
**Difficulty:** Normal
**Blocked by:** Task 1

**Files:**
- Modify: `crates/host/src/config/set_service.rs`, `crates/host/src/config/types.rs` (`SetService`, `ServiceField` only), `crates/host/src/config/fixtures.rs` (the full `SetService` literal at line ~11: add `relays: RelayEdit::Keep`)

**Interfaces:**
- Consumes: `RelayEdit`, `relay_list` helpers (Task 1).
- Produces: `SetService.relays`, `ServiceField::RelayUrls`.

- [ ] **Step 1: Write failing tests** in `set_service.rs` tests:
  - `set_override_on_existing`: access `web` without override, `Replace([a, b])` → `web.relay_urls == [a, b]`; `[node]` text unchanged; change `FieldChange{field: RelayUrls, old: "(node)", new: "[https://a.example, https://b.example]"}`.
  - `replace_override`: `[old]` → `Replace([a])` → `[a]`; old rendered `[https://old.example]`.
  - `clear_override`: `Clear` → no `relay_urls` key; new rendered `(node)`.
  - `clear_override_removes_explicit_empty_array`: `relay_urls = []` + `Clear` → key absent, `Updated`.
  - `override_kept_without_flags`: `Keep` with `port: Some(9090)` → `relay_urls` unchanged.
  - `create_with_override`: new `web` with port, node id and `Replace([a])` → entry has `relay_urls`.
  - `insecure_override_rejected`: `Replace(["http://a.example"])` → err contains `invalid relay_url 'http://a.example' on service 'web'`; bytes unchanged.
  - `duplicate_override_rejected`: `Replace([a, a])` → err contains `duplicate relay_url`; bytes unchanged.
  - `relay_only_on_missing_service_needs_address`: missing `web`, only `Replace([a])` → `SetError::MissingForCreate` naming `--listen` and `--node-id`.
  - `relay_urls_label`: `ServiceField::RelayUrls.label(Role::Access) == "relay_urls"`.
- [ ] **Step 2: Run** `cargo test -p iroh-tunnel-host set_service` → FAIL.
- [ ] **Step 3: Implement.**
  - Add the field and variant, and update the `changes` doc comment to "address, protocol, node_id, relay_urls".
  - In `create`, `ensure_unique` and `write_list` on `Replace`.
  - In `update`, after `node_id`, use `read_list` and `display_list(.., "(node)")`; on `Replace` call `ensure_unique` then `write_list` if different; on `Clear` remove a present key; push the change.
- [ ] **Step 4: Run** `cargo test -p iroh-tunnel-host` → PASS; clippy clean. (`src/cli.rs` still constructs `SetService` without `relays`, so the root crate will not compile until Task 3; run host-only commands here.)
- [ ] **Step 5: Commit** `feat(config): set access per-service relay override`.

---

## Task 3: CLI `config node` and access `config set -r` — implements 3

**Covers:** 3.1, 3.2, 3.3, 3.4, 3.5, 3.6
**Difficulty:** Hard
**Blocked by:** Task 1, Task 2

**Files:**
- Modify: `src/cli.rs`, `src/app.rs`, `src/config_cmd.rs`, `src/render.rs`

**Interfaces:**
- Consumes: `set_node`, `SetNode`, `SetNodeOutcome`, `NodeChange`, `TokenChange`, `RelayEdit`, `TokenEdit`, `SetService.relays`, `ServiceField::RelayUrls`.
- Produces: CLI contracts `<role> config node` and access `config set` relay args; `NodeArgs`, `ServiceArgs::SetExtra`, `AccessSetRelayArgs`, `NoSetExtra`, `config_cmd::{set, node}`, `read_token_source`, `render_node`.

- [ ] **Step 1: Write failing unit tests.**

  In `src/cli.rs` tests:
  - Replace `config_set_has_no_relay_flag` with three tests:
    - `serve_config_set_has_no_relay_flag`: serve set has no `relay-url`.
    - `access_config_set_has_relay_flags`: `relay-url` with short `r`; `clear-relays` long-only.
    - `config_node_exists_everywhere`: on `it serve`, `it access`, `its` and `ita`, `config node` has `relay-url`, `relay-token`, `relay-token-file` and `clear-relays`, and the `relay-token` help contains `shell history` and `--relay-token-file`.
  - Extend `long_only_options` with `relay-token`, `relay-token-file`, `clear-relays`.
  - `node_request_mapping`, table-driven over `NodeArgs::into_request`: `(-r [a], None, false) → (Replace([a]), Delete)`; `(-r [a], Some(t), false) → (Replace([a]), Replace(t))`; `([], Some(t), false) → (Keep, Replace(t))`; `([], None, true) → (Clear, Delete)`.
  - `parse_conflicts`: `parse_cli` errors with `ErrorKind::ArgumentConflict` for the four pairs of "Conflicting relay flags" and for access `set -n web -r A --clear-relays`.
  - `bare_config_node_is_missing_argument`: `config node` and `config node -c x` both error with `ErrorKind::MissingRequiredArgument`.
  - `parsed_cli_debug_redacts_token`: `format!("{:?}", parse_cli(b, [.., "config", "node", "-r", "https://a.example", "--relay-token", "s3cret"]))` lacks `s3cret`, for `Binary::AllInOne` (serve and access) and `Binary::Serve`.

  In `src/config_cmd.rs` tests:
  - `token_source_trims_one_newline`: `"secret-token\n"` → `"secret-token"`.
  - `token_source_trims_crlf`: `"secret-token\r\n"` → `"secret-token"`.
  - `token_source_keeps_extra_newline`: `"secret-token\n\n"` → `"secret-token\n"`.
  - `token_source_reads_stdin_for_dash`: path `-` with a `Cursor` → its content.
  - `token_source_empty_errors`: `""` → err contains `--relay-token-file`, the path and `empty`.

  In `src/render.rs` tests:
  - `render_node_updated` → exactly `"Updated node settings in /x/s.toml:\n  relay_urls: [https://old.example] -> [https://a.example]\n  relay_token: <redacted> -> (none)\n"`.
  - `render_node_token_replaced` → line `  relay_token: <redacted> -> <redacted> (changed)`.
  - `render_node_set_from_none` → line `  relay_token: (none) -> <redacted>`; an empty list renders `(default)`.
  - `render_node_unchanged` → `"Node settings unchanged (/x/s.toml).\n"`.
- [ ] **Step 2: Run** `cargo test -p iroh-tunnel --lib` → FAIL (compile errors for missing items).
- [ ] **Step 3: Implement the clap shapes** per Contracts and design D2.
  - `NodeArgs` gets `#[group(skip)]`, so clap derive does not build an implicit group that would also contain `config`. The `ConfigAction::Node` variant carries `#[command(group(ArgGroup::new("edit").required(true).multiple(true).args(["relay_url", "relay_token", "relay_token_file", "clear_relays"])))]`, so `config node -c x` still fails with `MissingRequiredArgument`.
  - `NodeArgs` has a manual `Debug` that prints `relay_token: Some(<redacted>)`.
  - `--relay-token` help: `Relay token (visible in shell history and process lists; prefer --relay-token-file)`.
  - `--relay-token-file` help: `File holding the relay token; '-' reads stdin`.
  - Update the `Config` doc to `(set/node/rm/list/show/edit/path/check/keygen)`.
- [ ] **Step 4: Implement `ServiceArgs::SetExtra`.**
  - Serve: `type SetExtra = NoSetExtra`, relays always `RelayEdit::Keep`.
  - Access: `type SetExtra = AccessSetRelayArgs` with `#[arg(short = 'r', long = "relay-url")] relay_url: Vec<String>` and `#[arg(long, conflicts_with = "relay_url")] clear_relays: bool`.
  - `set_request` takes `relays: RelayEdit`.
- [ ] **Step 5: Implement the handlers.**
  - `read_token_source` errors: `--relay-token-file <path>: cannot read` (with the io error as context, never the contents) and `--relay-token-file <path>: source is empty`. It strips one `\r\n` or `\n`.
  - `config_cmd::node` resolves the token before calling `set_node`: the flag value, or `read_token_source(path, std::io::stdin().lock())`. It then prints `render_node`.
  - `config_cmd::set` gains the `extra` parameter.
  - In `src/app.rs`, wire `ConfigAction::Node(args) => config_cmd::node(role, args)` and pass `extra` in the `Set` arm.
- [ ] **Step 6: Examples** in `examples()`:
  - `config node`: `"{prefix} config node -r https://relay.example.com -r https://relay2.example.com --relay-token-file token.txt"` and `"{prefix} config node --clear-relays"`.
  - access `set`: append `"{prefix} config set -n web -r https://relay.example.com"`.
- [ ] **Step 7: Run** `cargo test --workspace --lib --bins` → PASS; `cargo clippy --all-targets -- -D warnings` → clean.
- [ ] **Step 8: Commit** `feat(cli): add config node and access config set relay flags`.

---

## Task 4: Integration tests, golden output, docs — implements 4

**Covers:** 4.1, 4.2, 4.3, 4.4, 4.5
**Difficulty:** Normal
**Blocked by:** Task 3

**Files:**
- Create: `tests/config_relay.rs` (portable, not platform-gated; the real-binary relay flows run on Unix and Windows)
- Modify: `tests/cli_usage.rs`, `tests/config_lifecycle.rs` (extend `invalid_mutations_fail_and_leave_file_bytes_unchanged` only), `tests/cli_golden.rs`, `docs/configuration.md`
- Create: `tests/fixtures/golden/config_node_updated_report.{stdout,stderr}.txt`, `config_node_unchanged.{stdout,stderr}.txt`, `config_set_relay_override_report.{stdout,stderr}.txt`

- [ ] **Step 1: `tests/cli_usage.rs`.**
  - Rewrite `relay_flag_rejected_on_set` to serve only: `["serve","config","set","-n","web","-t","3000","-r","https://relay.example"]` on `it`, and `["config","set","-n","web","-t","3000","-r","https://relay.example"]` on `its`. Assert `assert_usage()`, the first paragraph contains `-r`, and stderr contains `Usage: iroh-tunnel serve config set` / `Usage: iroh-tunnel-serve config set`.
  - Add `node_conflicts_show_node_help`: the four pairs, each `assert_usage()` plus `Usage: iroh-tunnel serve config node`.
  - Add `access_set_override_conflict_shows_set_help`: `assert_usage()` plus `Usage: iroh-tunnel access config set`.
  - Add `bare_config_node_shows_help`: `config node` and `config node -c <tmp>/c.toml`, each `assert_usage()` with `Usage: iroh-tunnel serve config node`, and `!c.toml.exists()`.
  - Add `config_node_help_on_every_binary`: `--help` exits 0 and lists the four flags, on `it serve`, `it access`, `its`, `ita`.
- [ ] **Step 2: `tests/config_relay.rs`** (real binary via `env!("CARGO_BIN_EXE_iroh-tunnel")`). Every child process gets an absolute temp `IROH_TUNNEL_TEST_CONFIG_DIR`, `IROH_TUNNEL_STATE_DIR`, `IROH_TUNNEL_MACHINE_LOCK_DIR`, `HOME` and `XDG_CONFIG_HOME`, as `tests/cli_usage.rs::World` does, so no test touches the real default config on any OS. Token-source tests pass an explicit `-c`.
  - `config_node_role_isolation`: `serve config node -r https://a.example` without `-c` writes the serve default under the temp config dir, and the access default path (from `access config path`) does not exist.
  - `config_node_token_from_file`: `secret-token\n`.
  - `config_node_token_file_crlf`: `secret-token\r\n` → stored `secret-token`.
  - `config_node_token_from_stdin`: `Stdio::piped()` writes `secret-token`.
  - `config_node_stdin_closed_is_empty_error`: `Stdio::null()` → exit 2, `empty`.
  - `config_node_unreadable_token_file`: stderr names `--relay-token-file` and `missing.txt`, no `Usage:`, bytes unchanged.
  - `config_node_empty_token_file`.
  - `config_node_token_with_whitespace`: `--relay-token "has space"` → `invalid relay_token`.
  - `config_node_token_file_extra_line`: `secret-token\n\n` → exit 2; `secret-token` absent from stdout and stderr.
  - `config_node_insecure_url`: exit 2, `invalid relay_url 'http://relay.example'`, no `Usage:`, bytes unchanged.
  - `config_node_never_prints_token`: the "Change report" and "Token replaced" flows with `t1`/`t2`; neither appears in any stdout or stderr.
- [ ] **Step 3: `tests/config_lifecycle.rs`:** extend `invalid_mutations_fail_and_leave_file_bytes_unchanged` with `["serve","config","node","-c",cfg,"-r","https://a.example"]`, same assertions as the other mutations.
- [ ] **Step 4: `tests/cli_golden.rs`.** Add three tests:
  - `config_node_updated_report`: seed `[node]\nrelay_urls = ["https://old.example"]\nrelay_token = "t1"\n`, run `serve config node -c cfg -r https://a.example`.
  - `config_node_unchanged`: the same command twice; golden on the second run.
  - `config_set_relay_override_report`: access create, then `config set -n web -r https://a.example`.

  Run `GOLDEN_BLESS=1 cargo test --test cli_golden`. Read the six new fixture files under `tests/fixtures/golden/` and confirm they match the Global Constraints formats and contain no `t1`.
- [ ] **Step 5: `docs/configuration.md`.**
  - "Managing the file with `config`": add `config node`.
  - "Relays": replace the quick-mode-only line with these rules:
    - `config node -r URL [-r URL ...]` replaces the list;
    - `-r` without a token flag removes `relay_token`;
    - `--relay-token` / `--relay-token-file PATH|-` replaces the token, with a warning about argv exposure;
    - `--clear-relays` returns to the n0 relays;
    - `ita config set -n NAME -r URL` / `--clear-relays` sets or removes the per-service override.
  - "Validation rules": add the duplicate-URL and token-needs-relays rules.
- [ ] **Step 6: Verify** `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test --workspace` → all pass (report only failures and the summary).
- [ ] **Step 7: Commit** `test(config): cover relay flags end to end; docs: document config node`.

---

## Scenario → test

cli-entrypoints:
- Short letters do not collide → `src/cli.rs::short_options_are_consistent_and_conflict_free` (new commands are included automatically)
- Long-only options → `src/cli.rs::long_only_options` (extended, Task 3)
- Config path → `tests/cli_golden.rs::config_path_prints_resolved_path`
- Keygen random conflicts → `src/cli.rs::keygen_print_conflicts`; `tests/cli_usage.rs::keygen_print_with_config_is_usage_error`
- Removed syntax → `tests/cli_usage.rs::removed_syntax_is_usage_error`
- Node command on every executable → `tests/cli_usage.rs::config_node_help_on_every_binary`; `src/cli.rs::config_node_exists_everywhere`
- Unknown option on a leaf command → `tests/cli_usage.rs::unknown_option_on_leaf_shows_leaf_help`
- Unknown subcommand → `tests/cli_usage.rs::unknown_subcommand_shows_group_help`
- Missing quick-mode flag → `tests/cli_usage.rs::missing_quick_flag_shows_standalone_help`
- Invalid value → `tests/cli_usage.rs::invalid_protocol_lists_values`
- Schema rule failure has no help → `tests/config_lifecycle.rs::set_invalid_name_has_no_help`
- Wrong-role flag on config set → `tests/cli_usage.rs::wrong_role_flag_on_set`
- Relay flag not accepted by config set → `tests/cli_usage.rs::relay_flag_rejected_on_set` (rewritten); `src/cli.rs::serve_config_set_has_no_relay_flag`
- Conflicting relay flags → `tests/cli_usage.rs::node_conflicts_show_node_help`; `src/cli.rs::parse_conflicts`
- Conflicting override flags on access config set → `tests/cli_usage.rs::access_set_override_conflict_shows_set_help`
- Node command without an edit flag → `tests/cli_usage.rs::bare_config_node_shows_help`; `src/cli.rs::bare_config_node_is_missing_argument`
- Runtime error has no help → `tests/cli_usage.rs::runtime_config_error_has_no_usage`
- Help goes to stderr → `assert_usage()` (stdout empty) in every `tests/cli_usage.rs` usage test, including the new ones

config-management-commands:
- Invalid existing config → `tests/config_lifecycle.rs::invalid_mutations_fail_and_leave_file_bytes_unchanged` (extended with `config node`, Task 4 step 3)
- Invalid new value → `tests/config_lifecycle.rs::set_invalid_name_has_no_help`
- Node edit on a file with an invalid service → `set_node::tests::invalid_service_blocks_node_edit`, `set_node::tests::invalid_service_blocks_noop_edit`
- Replace relay list → `set_node::tests::replace_relay_list`
- Relay change without token deletes the token → `set_node::tests::relays_without_token_delete_token`
- Relay and token replaced together → `set_node::tests::relays_and_token_replaced`
- Token only keeps relays → `set_node::tests::token_only_keeps_relays`
- Clear relays → `set_node::tests::clear_relays_keeps_secret_key`, `set_node::tests::clear_removes_explicit_empty_array`
- Role isolation → `tests/config_relay.rs::config_node_role_isolation`
- Services and comments preserved → `set_node::tests::services_and_comments_preserved`
- Missing file → `set_node::tests::missing_file_created_with_node_only`
- Clear on a missing file → `set_node::tests::clear_on_missing_file_is_unchanged`
- Retired fields dropped → `set_node::tests::retired_multiplex_dropped`
- Token from file → `tests/config_relay.rs::config_node_token_from_file`
- Token from stdin → `tests/config_relay.rs::config_node_token_from_stdin`
- Unreadable token file → `tests/config_relay.rs::config_node_unreadable_token_file`
- Token file with CRLF → `tests/config_relay.rs::config_node_token_file_crlf`; `config_cmd::tests::token_source_trims_crlf`
- Token file with extra line → `tests/config_relay.rs::config_node_token_file_extra_line`; `config_cmd::tests::token_source_keeps_extra_newline`
- Empty token source → `tests/config_relay.rs::config_node_empty_token_file`, `config_node_stdin_closed_is_empty_error`; `config_cmd::tests::token_source_empty_errors`
- Argv token warning → `src/cli.rs::config_node_exists_everywhere`
- Token with whitespace → `tests/config_relay.rs::config_node_token_with_whitespace`
- Insecure relay URL → `set_node::tests::insecure_url_rejected`; `tests/config_relay.rs::config_node_insecure_url`
- Duplicate relay URL → `set_node::tests::duplicate_url_rejected`; `relay_list::tests::ensure_unique_rejects_duplicate`
- Duplicate override URL → `set_service::tests::duplicate_override_rejected`
- Token without relays → `set_node::tests::token_without_relays_rejected`
- Change report (node) → `render::tests::render_node_updated`; `tests/cli_golden.rs::config_node_updated_report`; `tests/config_relay.rs::config_node_never_prints_token`
- Token replaced → `render::tests::render_node_token_replaced`; `tests/config_relay.rs::config_node_never_prints_token`
- Nothing changed (node) → `set_node::tests::unchanged_keeps_bytes_and_mtime`; `tests/cli_golden.rs::config_node_unchanged`
- Set override on existing service → `set_service::tests::set_override_on_existing`
- Replace override → `set_service::tests::replace_override`
- Clear override → `set_service::tests::clear_override`, `set_service::tests::clear_override_removes_explicit_empty_array`
- Override kept without relay flags → `set_service::tests::override_kept_without_flags`
- Create with override → `set_service::tests::create_with_override`
- Insecure override → `set_service::tests::insecure_override_rejected`
- Change report (set) → `tests/cli_golden.rs::config_set_updated_report`
- Relay override report → `tests/cli_golden.rs::config_set_relay_override_report`; `set_service::tests::set_override_on_existing`
- Nothing changed (set) → `tests/config_lifecycle.rs::set_unchanged_keeps_bytes_and_mtime`
- Name only on existing entry → `tests/config_lifecycle.rs::set_name_only_on_existing_is_unchanged`
