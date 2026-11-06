# Remake config CLI Implementation Plan

> Executed via `/opsx-apply`, which chooses the executor.
> Spec reference: this change's `specs/` and `design.md`.

**Goal:** Replace `config add`/`remove` with `config set -n` (an upsert with a change report) and `config rm -n`, add `config check` and `keygen --print`, give quick mode and `config` one argument vocabulary, and print the failing command's help on every usage error.

**Architecture:**
- **Host crate (`iroh-tunnel-host`).** It gains the `set_service` and `check_config` use cases on the existing locked, validated mutation pipeline (`mutate_deciding`, `validate_role_doc`), and stays clap-free.
- **CLI crate (`src/`).** It owns the clap model. Shared `ServeServiceArgs` and `AccessServiceArgs` are flattened into both quick mode and `config set`. One command factory applies the per-context required, default and help adjustments, and is used for parsing, help rendering and tests alike. A `UsageError` renderer prints the error, then the help of the deepest matched subcommand.
- **Unchanged.** Persistence, exit codes and secret hygiene.

**Tech Stack:** Rust 2021 (MSRV 1.91), clap 4.6 derive (`ValueEnum`, generic `Args`/`Subcommand`, `Command::mut_arg`/`mut_subcommand`, `try_get_matches_from_mut`), toml_edit, anyhow plus `CliError`, cargo test.

## Global Constraints

- `iroh-tunnel-host` MUST NOT depend on clap, read stdin, print, or call `process::exit`. This is guarded by `tests/workspace_layout.rs::host_depends_neither_on_root_nor_clap`.
- No new crates or dependencies. `thiserror` is used only where it already is (the host crate). The engine's public API is unchanged.
- Exit codes:

  | Code | Meaning |
  |---|---|
  | 0 | success |
  | 2 | usage or config error |
  | 3 | permission |
  | 4 | iroh |
  | 5 | service |
  | 6 | conflict |
  | 1 | other |

- Diagnostics and the `set` change report never contain `secret_key` or `relay_token` values (RS-10).
- Every config mutation stays locked, schema-validated, atomic, owner-only (0600 on Unix), comment- and decor-preserving, and all-or-nothing.
- Short options:

  | Short | Long |
  |---|---|
  | `-c` | `--config` |
  | `-n` | `--name` |
  | `-t` | `--target` |
  | `-l` | `--listen` |
  | `-N` | `--node-id` |
  | `-p` | `--protocol` |
  | `-r` | `--relay-url` |
  | `-k` | `--key-file` |
  | `-y` | `--yes` |

  `--reveal`, `--print`, `--system` and `--user` have no short form.
- Help and errors go to stderr. A usage error leaves stdout empty.
- **Every commit is green.** Each task ends with `cargo build --workspace --all-targets && cargo test --workspace` passing. A task that changes observable CLI output updates the integration tests that pin it in the same task.
- Repo standards: `docs/agents/rust-coding-standards.md` (RS-01–RS-16), TDD, and `cargo clippy --workspace --all-targets -- -D warnings`.

## Review Focus

1. **Updating an entry whose stored host is IPv6.** `set -n web -t 3001` on `host = "::1"` keeps `::1`, and the report shows `[::1]:3000 -> [::1]:3001`. Test: Task 2 `set_bare_port_keeps_ipv6_host`.
2. **`set` on a file whose `services` is not an array of tables** (inline `services = []`). This is a config error, never a `MissingForCreate`, and the file is unchanged. Test: Task 2 `set_on_inline_services_array_is_config_error`.
3. **A usage error in a standalone binary** must render that binary's tree (`iroh-tunnel-serve config set`). Test: Task 7 `standalone_usage_error_shows_standalone_help`.
4. **Global flags and `=`-joined values before or between subcommands** (`it -v --color=never serve config set --bogus`). Detection of the deepest subcommand must skip them. Test: Task 7 `deepest_command_skips_global_flags`.
5. **Empty name** (`-n ''`) is a clap usage error. Test: Task 4 `empty_name_is_usage_error`.

---

## Contracts

| Name | new / modified | Shape | public |
|---|---|---|---|
| `config::SetService` | new | `pub struct SetService { pub config: Option<PathBuf>, pub name: String, pub host: Option<String>, pub port: Option<u16>, pub protocol: Option<Protocol>, pub node_id: Option<String> }` (`Debug, Clone, PartialEq, Eq`). `None` = not passed. `host` is used only when `port` is `Some`. | public (host API) |
| `config::SetOutcome` | new | `pub enum SetOutcome { Created { service: String, path: PathBuf }, Updated { service: String, path: PathBuf, changes: Vec<FieldChange> }, Unchanged { service: String, path: PathBuf } }` | public |
| `config::FieldChange` | new | `pub struct FieldChange { pub field: ServiceField, pub old: String, pub new: String }`. Addresses are rendered by `display_addr`. | public |
| `config::ServiceField` | new | `pub enum ServiceField { Address, Protocol, NodeId }`, with `pub fn label(self, role: Role) -> &'static str` returning `target` (serve) / `listen` (access), `protocol` or `node_id` | public |
| `config::SetError` | new | `#[derive(thiserror::Error, Debug)] pub enum SetError { #[error("creating service '{name}' requires {}", .flags.join(" and "))] MissingForCreate { name: String, flags: Vec<&'static str> } }`. Flags are `"--target"` / `"--listen"`, then `"--node-id"`. Returned inside `anyhow::Error`. | public |
| `config::set_service` | new | `pub fn set_service(role: Role, req: &SetService) -> anyhow::Result<SetOutcome>` | public |
| `config::check_config` | new | `pub fn check_config(role: Role, config: Option<&Path>) -> anyhow::Result<PathBuf>` | public |
| `config::display_addr` | new (host-internal) | `pub(crate) fn display_addr(host: &str, port: u16) -> String`. IPv6 is bracketed. This is a host copy of the engine's private `format_local_addr`; the engine API is untouched. | internal |
| `config::add_service`, `config::AddService`, `fixtures::add_web` | modified (removed in Task 5) | Deleted. Replaced by `set_service` and `fixtures::set_web(&Path) -> SetService` (port 8080, host `None`). | public |
| `config::require_existing` guidance | modified | Text changes from `config add` to `config set` (keeps naming `config keygen`). | public CLI text |
| CLI `ServiceAddr` | new | `#[derive(Clone, Debug, PartialEq, Eq)] pub struct ServiceAddr { pub host: Option<String>, pub port: NonZeroU16 }`; `pub fn parse_service_addr(s: &str) -> Result<ServiceAddr, String>` (grammar in Task 4) | internal |
| CLI `ProtocolArg` | new | `#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)] pub enum ProtocolArg { Tcp, Udp }`; `impl From<ProtocolArg> for config::Protocol` | internal |
| CLI `ColorArg` | new | `#[derive(ValueEnum, Clone, Copy, Debug)] pub enum ColorArg { Auto, Always, Never }`. Replaces `color: String` in `Cli`, `ServeCli`, `AccessCli` and `into_cli`. | internal |
| CLI `ServeServiceArgs` | new | `#[derive(Args, Debug)] pub struct ServeServiceArgs { #[arg(short='n', long, value_parser = NonEmptyStringValueParser::new())] pub name: Option<String>, #[arg(short='t', long, value_name="[HOST:]PORT", value_parser = parse_service_addr)] pub target: Option<ServiceAddr>, #[arg(short='p', long, value_enum)] pub protocol: Option<ProtocolArg> }` | internal |
| CLI `AccessServiceArgs` | new | Same as `ServeServiceArgs`, with `listen: Option<ServiceAddr>` (`-l`, `value_name="[HOST:]PORT"`) in place of `target`, plus `#[arg(short='N', long, value_parser = NonEmptyStringValueParser::new())] pub node_id: Option<String>` | internal |
| CLI `ServeQuickArgs` / `AccessQuickArgs` | modified | `{ #[command(flatten)] service: ServeServiceArgs \| AccessServiceArgs, #[arg(short='r', long)] relay_url: Option<String>, #[arg(short='k', long)] key_file: Option<PathBuf> }` | public CLI |
| CLI trait `ServiceArgs` | new | `pub trait ServiceArgs: clap::Args + clap::FromArgMatches { const ROLE: Role; fn into_set(self, config: Option<PathBuf>) -> anyhow::Result<SetService>; }`. Implemented for both service-args structs. A missing name never reaches `into_set`, because the factory makes `-n` required; `into_set` returns an internal error if it does. | internal |
| CLI `RoleCmd<S: ServiceArgs>`, `ConfigAction<S: ServiceArgs>` | modified | `ConfigAction<S>` variants: `Set { #[command(flatten)] service: S, #[arg(short, long)] config: Option<PathBuf> }`, `Rm { #[arg(short='n', long)] name: String, #[arg(short, long)] config }`, `List { config }`, `Show { config, #[arg(long)] reveal: bool }`, `Edit { config }`, `Path { config }`, `Check { config }`, `Keygen { config, #[arg(short, long)] yes: bool, #[arg(long, conflicts_with_all=["config","yes"])] print: bool }`. `Role::Serve { quick: ServeQuickArgs, cmd: Option<RoleCmd<ServeServiceArgs>> }`; Access likewise. Fallback if clap derive rejects the generic enum: a `macro_rules!` that stamps `ServeRoleCmd` and `AccessRoleCmd` from one body (decided in the Task 5 Step 1 spike). | public CLI |
| CLI `Binary` | new | `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Binary { AllInOne, Serve, Access }`. It replaces `Packaging`; `Binary::is_standalone()` covers the old `Packaging::Standalone` use. | internal |
| CLI `build_command` | new | `pub fn build_command(binary: Binary) -> clap::Command`. Takes `Cli`/`ServeCli`/`AccessCli::command()`, then via `mut_subcommand`/`mut_arg`: (1) quick-mode roots get `name`, `target` (or `listen` and `node_id`) `required(true)` and `protocol` `default_value("tcp")`; (2) `config set` gets `name` `required(true)` and the protocol help suffix `" (default on create: tcp)"`. Finally `.build()`. | internal |
| CLI `parse_cli` | new | `pub fn parse_cli(binary: Binary, argv: impl IntoIterator<Item = OsString>) -> Result<Cli, clap::Error>`. `build_command(binary).try_get_matches_from_mut(argv)` then `C::from_arg_matches_mut`, with standalone results converted by `into_cli`. | internal |
| CLI `UsageError` | new | `pub struct UsageError { pub command_path: Vec<String>, pub message: String }` with a hand-written `Display` (= message) and `std::error::Error`. `command_path` lists subcommand names below the binary root, e.g. `["serve","config","set"]`. | internal |
| CLI `clap_error_summary` | new | `pub fn clap_error_summary(e: &clap::Error) -> String`. Takes the rendered text up to the first blank line, strips the leading `error: `, and drops any `Usage:` paragraph and the `For more information` tail. Keeps continuation lines such as `  --target <[HOST:]PORT>` and `  [possible values: tcp, udp]`. | internal |
| CLI `print_usage_error_to` / `print_usage_error` | new | `pub fn print_usage_error_to(w: &mut dyn Write, binary: Binary, path: &[String], message: &str) -> i32` writes `error: {message}\n\n{help}` and returns 2. `help` is `render_help()` of the subcommand at `path` in `build_command(binary)`. `print_usage_error` passes stderr. | internal |
| CLI `deepest_command_path` | new | `pub fn deepest_command_path(root: &clap::Command, argv: &[OsString]) -> Vec<String>`. Grammar in Task 7. | internal |
| CLI surface `config set` | new | `serve config set -n NAME [-t [HOST:]PORT] [-p tcp\|udp] [-c FILE]`; `access config set -n NAME [-l [HOST:]PORT] [-N ID] [-p tcp\|udp] [-c FILE]` | public CLI |
| CLI surface `config rm` / `config check` | new | `<role> config rm -n NAME [-c FILE]`; `<role> config check [-c FILE]` | public CLI |
| CLI surface `config keygen --print` | modified | Replaces `keygen random`. Conflicts with `-c` and `-y`. | public CLI |
| CLI surface `config add`, `config remove` | modified (removed) | Unknown subcommand, which is a usage error. | public CLI |
| CLI quick mode | modified | Serve: `-n NAME -t [HOST:]PORT [-p tcp\|udp] [-r URL] [-k FILE]`. Access: `-n NAME -l [HOST:]PORT -N ID [-p] [-r] [-k]`. Hostnames are accepted; a bare PORT means `127.0.0.1`. | public CLI |
| CLI `--color` | modified | `auto\|always\|never`, `[default: auto]` | public CLI |
| Output `config set` | new | Created: `Created service '<n>' in <p>.\n`. Updated: `Updated service '<n>' in <p>:\n` followed by `  <label>: <old> -> <new>\n` per change, in the order address, protocol, node_id. Unchanged: `Service '<n>' unchanged (<p>).\n` | public CLI |
| Output `config check` | new | `OK: <p> is a valid <role> config.\n` | public CLI |
| Config file format | — | Unchanged. | — |

---

## Task 1: Host set types — implements 1.1

**Files:**
- Modify: `crates/host/src/config/types.rs`, `crates/host/src/config/mod.rs`
- Create: `crates/host/src/config/display_addr.rs`

**Interfaces:**
- Produces: `SetService`, `SetOutcome`, `FieldChange`, `ServiceField`, `SetError`, `display_addr`.

- [ ] **Step 1:** Write failing unit tests:
  - `missing_for_create_names_flags`: `SetError::MissingForCreate{name:"web".into(), flags: vec!["--listen","--node-id"]}.to_string() == "creating service 'web' requires --listen and --node-id"`.
  - `service_field_labels`: labels match the Contracts for both roles.
  - `display_addr_brackets_ipv6`: `("::1",3000)` gives `[::1]:3000`; `("db.internal",5432)` gives `db.internal:5432`.
- [ ] **Step 2:** Run `cargo test -p iroh-tunnel-host config::`. Expected: FAIL to compile (types missing).
- [ ] **Step 3:** Add the types exactly as in Contracts. Keep `AddService`.
- [ ] **Step 4:** Run `cargo build --workspace --all-targets && cargo test --workspace`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(host): add set_service request/outcome types`.

## Task 2: Host `set_service` — implements 1.2

**Files:**
- Create: `crates/host/src/config/set_service.rs`
- Modify:
  - `config/mod.rs` (export)
  - `config/fixtures.rs` (add `set_web`; keep `add_web` until Task 5)
  - `config/mutate.rs` (add `find_service_index(doc) -> Result<Option<usize>>`, which errors on a non-array-of-tables `services`)

**Interfaces:**
- Consumes: Task 1.
- Produces: `set_service`.

- [ ] **Step 1:** Write failing unit tests in `set_service.rs`. Each uses a tempdir file, and the existing `web` entry comes from `VALID_SERVE` (tcp `127.0.0.1:8080`) unless stated otherwise.
  - `create_applies_defaults`: `api` with port 3000, no protocol. Expect `Created{service:"api",..}`, and the file has `protocol = "tcp"`, `host = "127.0.0.1"`, `port = 3000`.
  - `create_access_requires_listen_and_node_id`: no port, no node_id. Downcasts to `SetError::MissingForCreate{flags: ["--listen","--node-id"]}`; the file bytes are unchanged.
  - `create_serve_requires_target`: `["--target"]`. When the file was absent, the config file still does not exist. The parent dir and `.lock` may exist, which is documented.
  - `update_port_keeps_host_and_protocol`: `web` = udp `10.0.0.5:3000`, set port 3001. Exactly one change `FieldChange{field:Address, old:"10.0.0.5:3000", new:"10.0.0.5:3001"}`; protocol stays `udp`.
  - `update_host_and_port`: host `Some("127.0.0.1")` with port 4000 replaces both.
  - `set_bare_port_keeps_ipv6_host`: `host = "::1"` port 3000, then port 3001. Change `[::1]:3000 -> [::1]:3001`.
  - `absent_host_key_compares_as_default`: the entry has no `host` key; set `host Some("127.0.0.1")` with the same port. Expect `Unchanged`, and no `host` key is written.
  - `update_keeps_udp_settings_comments_and_decor`: a udp entry with a UDP flow-setting key, a `# keep` comment line, and `port = 3000 # why`. Set port 3001. All three survive, giving `port = 3001 # why`.
  - `update_to_tcp_with_udp_settings_is_rejected`: `CliError::Config`, file unchanged.
  - `equal_values_are_unchanged_and_not_written`: set the identical port. Expect `Unchanged`; the bytes and `metadata().modified()` are identical.
  - `name_only_on_existing_is_unchanged`.
  - `set_on_inline_services_array_is_config_error`: the file is `services = []`. Run once with port 3000 and once with name only. Both give `CliError::Config` containing `[services] must be an array of tables`; neither is `SetError`. The file is unchanged.
  - `invalid_name_is_config_error`: name `BAD` gives `CliError::Config`.
  - `create_preserves_comments_and_entries`.
- [ ] **Step 2:** Run `cargo test -p iroh-tunnel-host config::set_service`. Expected: FAIL.
- [ ] **Step 3:** Implement `set_service` inside `mutate_deciding(role, &resolve_target(&resolve_config_path(role, req.config.as_deref())?), |doc| …)`:
  - Call `find_service_index` first; a shape error returns `CliError::Config`.
  - **Absent entry:**
    1. Collect the missing flags: `--target`/`--listen` if `port` is `None`; for access, `--node-id` if `node_id` is `None`.
    2. If any are missing, return `SetError::MissingForCreate`.
    3. Otherwise push a table with keys `name`, `protocol` (default `tcp`), `host` (default `127.0.0.1`), `port`, and `node_id` for access.
    4. Return `Edit::Apply(Created)`.
  - **Present entry:** for each passed field, compare against the effective current value. The address compares `(host or "127.0.0.1", port)`; `host: None` keeps the current host. When a value differs, write it into the existing item, copying the old `Decor` (as `redact_secrets` does), and record a `FieldChange` via `display_addr`. No changes returns `Edit::Keep(Unchanged)`; otherwise return `Edit::Apply(Updated)`.
- [ ] **Step 4:** Run `cargo build --workspace --all-targets && cargo test --workspace`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(host): add set_service upsert`.

## Task 3: Host `check_config` — implements 1.3

**Files:**
- Create: `crates/host/src/config/check_config.rs`
- Modify: `config/mod.rs`, and `config/mutate.rs` (`validate_role_doc` visible to `check_config`)

**Interfaces:**
- Produces: `check_config`.

- [ ] **Step 1:** Write failing tests:
  - `check_valid_returns_path`.
  - `check_invalid_is_config_error_and_unchanged`.
  - `check_missing_uses_require_existing_guidance`: the error contains `config keygen`. The `config set` wording is asserted in Task 5.
  - `check_invalid_diagnostic_has_no_secret`: the file has `relay_token = "leak-me" oops` on line 2. The error contains `line 2` and does not contain `leak-me`.
  - `check_ignores_held_lock_and_never_writes`: hold `ConfigLock::acquire(&path)` and put `multiplex = true` in an entry. `check_config` returns `Ok` within 1 s and the bytes are unchanged.
- [ ] **Step 2:** Run `cargo test -p iroh-tunnel-host config::check_config`. Expected: FAIL.
- [ ] **Step 3:** Implement: `let path = resolve_target(&resolve_config_path(role, config)?); let text = require_existing(role, &path)?; validate_role_doc(role, &text, &path)?; Ok(path)`. No lock, no second read.
- [ ] **Step 4:** Run `cargo build --workspace --all-targets && cargo test --workspace`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(host): add config check operation`.

## Task 4: Shared args, value types, command factory, quick mode — implements 2.1, 2.2, 3.3

**Files:**
- Create: `src/cli_args.rs` (`ServiceAddr`, `parse_service_addr`, `ProtocolArg`, `ColorArg`, `ServeServiceArgs`, `AccessServiceArgs`)
- Modify:
  - `src/lib.rs`
  - `src/cli.rs` (quick args flatten the shared structs; `color: ColorArg`; `Binary`; `build_command`; `parse_cli`)
  - `src/app.rs`:
    - the entry points use `parse_cli(binary, args_os())`, mapping `Err(e)` to `e.exit()` for now;
    - quick mode builds from `ServiceAddr`, where a `None` host becomes `127.0.0.1`;
    - remove the post-parse required checks and `quick_protocol`;
    - `Packaging` becomes `Binary`.
  - `tests/quick_run.rs`: the `"tcp or udp"` assertions at lines 753 and 780-781 become `"possible values"`.
  - `tests/role_binaries.rs`: help assertions expect `-r, --relay-url`.

**Interfaces:**
- Produces: `ServiceAddr`, `ProtocolArg`, `ColorArg`, `ServeServiceArgs`, `AccessServiceArgs`, `Binary`, `build_command`, `parse_cli`.

- [ ] **Step 1:** Write failing unit tests.
  - `parse_service_addr` (in `cli_args.rs`):
    - `"3000"` → `{host:None, port:3000}`.
    - `"10.0.0.5:3000"` → host `10.0.0.5`.
    - `"db.internal:5432"` → host `db.internal`.
    - `"[::1]:8080"` → host `::1`.
    - Each of `"0"`, `"70000"`, `":3000"`, `"host:"`, `"::1:8080"`, `"not-an-address"`, `"a b:1"`, `"[::1]"` → `Err` whose message contains `port` or `address`.
  - Factory tests (in `cli.rs`):
    - `empty_name_is_usage_error`: `parse_cli(Binary::Serve, ["its","-n","","-t","1"])` is `Err`.
    - `quick_requires_name_and_target`: `parse_cli(Binary::Serve, ["its","-n","web"])` gives `Err` of kind `MissingRequiredArgument`.
    - `quick_protocol_defaults_to_tcp`.
    - `role_command_does_not_need_quick_flags`: `parse_cli(Binary::AllInOne, ["it","serve","config","path"])` is `Ok`.
- [ ] **Step 2:** Run `cargo test --lib`. Expected: FAIL.
- [ ] **Step 3:** Implement as in Contracts.
  - **`parse_service_addr` grammar:**
    1. All digits → port.
    2. Leading `[` → `[IPV6]:PORT`, with the inside parsed as `Ipv6Addr`.
    3. Otherwise split at the last `:`. The host must be non-empty, contain no `:`, and be either an `Ipv4Addr` or made only of `[A-Za-z0-9.-]`.
    4. The port must parse as `NonZeroU16`.
  - **Shared doc comments:**

    | Flag | Help text |
    |---|---|
    | `-n` | "Service name (lowercase letters, digits, '-'; must match on both sides)." |
    | `-t` | "Local service to forward to: PORT (on 127.0.0.1) or HOST:PORT." |
    | `-l` | "Local address to listen on: PORT (on 127.0.0.1) or HOST:PORT." |
    | `-N` | "Serve NodeId (public identity, not authorization; protect the target service)." |
    | `-p` | "Forwarding protocol." |

  - Role and standalone roots keep `args_conflicts_with_subcommands = true` and add `subcommand_negates_reqs = true`.
- [ ] **Step 4:** Run `cargo build --workspace --all-targets && cargo test --workspace`. Expected: PASS, including the updated `quick_run` and `role_binaries`.
- [ ] **Step 5:** Commit `feat(cli): shared service args, [HOST:]PORT and command factory`.

## Task 5: Config command tree, handlers, output; remove add — implements 2.3, 4.1, 4.2, 1.4, 5.1, 5.5

**Files:**
- Modify:
  - `src/cli.rs`: `ServiceArgs` trait, generic `RoleCmd<S>`/`ConfigAction<S>`; remove `Add`, `Remove`, `KeygenMode`, `AddServiceArgs`.
  - `src/app.rs`: `dispatch_role_cmd<S: ServiceArgs>`, `dispatch_config<S: ServiceArgs>`.
  - `src/config_cmd.rs`: `set`, `rm`, `check`; `keygen --print` calls `keygen_random`; drop `add`.
  - `src/render.rs`: `render_set(role, &SetOutcome)`, `render_checked(role, &Path)`; drop `render_added`.
  - `crates/host/src/config/{mod.rs, types.rs, fixtures.rs, list_services.rs, remove_service.rs, require_existing.rs}`: delete `add_service.rs`, `AddService` (in `types.rs`) and `add_web`, keeping `Mutation` for `remove_service`; tests use `set_web`; guidance text becomes `config set`.
  - Integration tests:
    - `tests/config_lifecycle.rs`, `tests/config_lifecycle_windows.rs`, `tests/config_paths.rs`, `tests/config_paths_windows.rs`, `tests/config_multiplex_compat.rs`, `tests/configured_multiplex.rs`: migrate the syntax.
    - `tests/cli_golden.rs` and `tests/fixtures/golden/*`: rename the goldens.

**Interfaces:**
- Consumes: Tasks 2-4.
- Produces: `RoleCmd<S>`, `ConfigAction<S>`, `ServiceArgs`; `config_cmd::set<S: ServiceArgs>(args: S, config: Option<PathBuf>) -> Result<()>`; `config_cmd::check(role, config: Option<&Path>) -> Result<()>`.

- [ ] **Step 1 (spike):** Add `RoleCmd<S>` with `#[derive(Subcommand)]` and a bound `S: ServiceArgs`, then run `cargo build`. If derive rejects the generic, switch to the `macro_rules!` fallback from Contracts and note it in apply.md.
- [ ] **Step 2:** Write failing unit tests in `render.rs`:
  - `render_created` → `Created service 'web' in /x/s.toml.\n`.
  - `render_updated_lists_changes` → `Updated service 'web' in /x/s.toml:\n  target: 127.0.0.1:3000 -> 127.0.0.1:3001\n  protocol: tcp -> udp\n`.
  - `render_unchanged` → `Service 'web' unchanged (/x/s.toml).\n`.
  - `render_checked` → `OK: /x/s.toml is a valid serve config.\n`.
  - `access_label_is_listen`.
- [ ] **Step 3:** Run `cargo test --lib render`. Expected: FAIL.
- [ ] **Step 4:** Implement:
  - the tree, dispatch, handlers and renderers;
  - deletion of the add path in host and CLI;
  - `build_command` extended for both roles' `config set`: `name` gets `required(true)`, and `protocol` help gets the suffix `" (default on create: tcp)"`;
  - in `execute`, `SetError` maps to exit 2, with the message printed as `error: <msg>`. Task 7 upgrades this to help.

  Add unit tests `set_without_name_is_usage_error` (`parse_cli(AllInOne, ["it","serve","config","set","-t","3000"])` is `Err(MissingRequiredArgument)`) and `missing_for_create_exits_2` (an `execute`-level test via the binary in `tests/config_lifecycle.rs`: `serve config set -n web` exits 2).
- [ ] **Step 5:** Migrate the existing integration tests. Keep every assertion and rewrite only the invocations:

  | Old | New |
  |---|---|
  | `config add --name N --protocol P [--host H] --port X [--node-id I]` | `config set -n N -t\|-l [H:]X [-p P] [-N I]` |
  | `config remove --name N` | `config rm -n N` |
  | `keygen random` | `keygen --print` |

  Renames:
  - `concurrent_adds_serialize_without_losing_updates` → `concurrent_sets_serialize_without_losing_updates`, in both the Unix and Windows files.
  - `config_paths::keygen_random_*` → `keygen_print_*`.

  Update the guidance assertion to `config set` in `config_lifecycle.rs` (`read_commands_without_config_share_one_guidance_error_and_create_nothing`) and in `config_lifecycle_windows.rs` (`read_commands_without_config_share_one_error_and_create_nothing`). Add `check` to both.
- [ ] **Step 6:** Update the goldens:
  - Rename the tests: `config_add_message_is_unchanged` → `config_set_created_message`; `config_remove_message_is_unchanged` → `config_rm_message`; `keygen_random_prints_key_and_node_id_lines` → `keygen_print_prints_key_and_node_id_lines`.
  - Add `config_set_updated_report` and `config_check_ok_message`.
  - `git rm` the old `config_add_*`, `config_remove_*` and `keygen_random_*` fixture files.
  - Run `GOLDEN_BLESS=1 cargo test --test cli_golden`, then `cargo test --test cli_golden`.
  - Check `git status tests/fixtures/golden`: only the renamed and new files changed; the `serve_startup`, `status`, `config_list_*`, `config_path_*` and `config_show_*` goldens are byte-identical.
- [ ] **Step 7:** Run `cargo build --workspace --all-targets && cargo test --workspace`. Expected: PASS. Run `cargo check --tests --target x86_64-pc-windows-msvc` if that target is installed; otherwise record in apply.md that the Windows test files are compile-checked by CI.
- [ ] **Step 8:** Commit `feat(cli)!: config set/rm/check and keygen --print replace add/remove/random`.

## Task 6: Help content and CLI invariants — implements 2.4, 2.5

**Files:**
- Modify: `src/cli.rs` (descriptions, `after_help` examples, `arg_required_else_help`) and `tests/quick_run.rs` (fix any assertion on bare `serve` output).

**Interfaces:**
- Consumes: `build_command` (Task 4), tree (Task 5).

- [ ] **Step 1:** Write failing unit tests in `cli.rs`. All of them walk `build_command(b)` for each `Binary`, recursively.
  - `short_options_are_consistent_and_conflict_free`, extended: wherever a letter appears it maps to its long name per the Global Constraints table; no command has a duplicate short.
  - `long_only_options`: `reveal`, `print`, `system` and `user` have no short.
  - `no_blank_descriptions`: every arg except `help` and `version` has a non-empty `get_help()`.
  - `shared_flags_match_between_quick_and_set`: for serve (`name target protocol`) and access (`name listen node_id protocol`), the quick root and `config set` agree on long, short, value name and help (with `" (default on create: tcp)"` stripped). Defaults may differ only on `protocol`.
  - `config_set_has_no_relay_flag`.
  - `config_set_requires_name`: `parse_cli(AllInOne, ["it","serve","config","set","-t","3000"])` gives `MissingRequiredArgument`.
  - `examples_on_listed_commands`: `after_help` contains `Examples:` for `serve config set`, `serve config rm`, `serve config keygen`, `access config set`, and the serve and access roots, in all three binaries.
  - `keygen_print_conflicts`: `["it","serve","config","keygen","--print","-y"]` gives `ArgumentConflict`.
  - `bare_invocations_request_help`: `it`, `it serve`, `its`, `ita config` and `it serve service` give the kind `DisplayHelpOnMissingArgumentOrSubcommand`.
- [ ] **Step 2:** Run `cargo test --lib`. Expected: FAIL.
- [ ] **Step 3:** Implement.
  - Fill in every missing doc comment.
  - Set `arg_required_else_help(true)` on `Cli`, `ServeCli`, `AccessCli`, the `serve`/`access` roles, `config` and `service`.
  - Add the Examples (`it` in all-in-one, `its`/`ita` in standalone):

    | Command | Examples |
    |---|---|
    | serve `config set` | `it serve config set -n web -t 3000`, `it serve config set -n web -t 3001 -p udp` |
    | access `config set` | `it access config set -n web -l 8080 -N <NODE_ID>` |
    | `rm` | `it serve config rm -n web` |
    | `keygen` | `it serve config keygen`, `it serve config keygen --print` |
    | quick serve | `it serve -n web -t 3000` |
    | quick access | `it access -n web -l 8080 -N <NODE_ID>` |

- [ ] **Step 4:** Run `cargo build --workspace --all-targets && cargo test --workspace`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(cli): complete help text, examples and bare-command help`.

## Task 7: Usage errors with help — implements 3.1, 3.2, 3.4

**Files:**
- Create: `src/usage.rs`
- Modify: `src/app.rs` (error handling in the entry points and `execute`) and `tests/quick_run.rs` (`rejected` helper expectations, if stderr text changes)

**Interfaces:**
- Consumes: `build_command`, `parse_cli`, `Binary`, host `SetError`.
- Produces: `UsageError`, `clap_error_summary`, `print_usage_error_to`/`print_usage_error`, `deepest_command_path`.

- [ ] **Step 1:** Write failing unit tests in `usage.rs`.
  - `summary_keeps_continuation_lines`:
    - For `parse_cli(Serve, ["its","-n","web"])`, the summary contains `the following required arguments were not provided` and `--target`, and contains neither `Usage:` nor `For more information`.
    - For `-p sctp`, it contains `possible values: tcp, udp`.
  - `renders_error_then_help`: `print_usage_error_to(&mut buf, AllInOne, &["serve","config","set"], "x")` writes a buffer that starts with `error: x\n\n` and contains `Usage: iroh-tunnel serve config set`. It returns 2.
  - `standalone_usage_error_shows_standalone_help`: `(Serve, ["config","set"])` gives `Usage: iroh-tunnel-serve config set`.
  - `deepest_command_skips_global_flags`:
    - `it -v --color=never serve config set --bogus` → `["serve","config","set"]`.
    - `it --color never serve config bogus` → `["serve","config"]`.
    - `it serve -n config` → `["serve"]`; `config` is a flag value.
    - `its -- config` → `[]`.
    - `it serve --color never config bogus` → `["serve","config"]`.
- [ ] **Step 2:** Run `cargo test --lib usage`. Expected: FAIL.
- [ ] **Step 3:** Implement.
  - **`deepest_command_path`.** At each command level, walk the tokens after `argv[0]`:
    - `--` stops the walk;
    - `--long=value` is skipped;
    - `--long` or `-s` naming a value-taking arg of the current command **or any ancestor** (global args such as `--color`) skips the next token as well;
    - other `-`-tokens are skipped;
    - the first non-flag token descends if it names a subcommand, and otherwise stops the walk.
  - **Entry points.** On a `parse_cli` error:
    - kinds `DisplayHelp` and `DisplayVersion` call `e.exit()`;
    - `DisplayHelpOnMissingArgumentOrSubcommand` prints `e.render()` to stderr and exits 2;
    - all other kinds call `exit(print_usage_error(binary, &deepest_command_path(&build_command(binary), &argv), &clap_error_summary(&e)))`.
  - **`execute`.**
    1. `downcast_ref::<UsageError>()` → `print_usage_error`.
    2. `downcast_ref::<SetError>()` → `UsageError` at `["serve"|"access","config","set"]` (all-in-one) or `["config","set"]` (standalone), using `Binary`.
    3. Anything else uses the existing `CliError` mapping.
- [ ] **Step 4:** Run `cargo build --workspace --all-targets && cargo test --workspace`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(cli): print the failing command's help on usage errors`.

## Task 8: Config set/check e2e (Unix and Windows) — implements 5.2

**Files:**
- Modify: `tests/config_lifecycle.rs` and `tests/config_lifecycle_windows.rs` (both use their existing helpers)

- [ ] **Step 1:** Write tests in `config_lifecycle.rs`. Each asserts the exit code, stdout, stderr, and the file bytes and `modified()`.
  - `set_creates_serve_entry_with_defaults`: `serve config set -c c.toml -n web -t 3000` exits 0, prints `Created service 'web' in <c.toml>.`, and the file has tcp and 127.0.0.1.
  - `set_creates_access_entry`: `-l 0.0.0.0:8080 -N <id> -p udp`.
  - `set_accepts_hostname_target`.
  - `set_create_on_missing_file`.
  - `set_without_target_shows_set_help_and_writes_nothing`: exit 2, stdout empty. stderr starts with `error: creating service 'web' requires --target` and contains `Usage: iroh-tunnel serve config set`. The config file does not exist.
  - `access_set_with_name_only_names_listen_and_node_id`.
  - `set_bare_port_update_keeps_host`.
  - `set_host_port_update_replaces_both`.
  - `set_update_keeps_udp_settings`.
  - `set_udp_to_tcp_with_udp_settings_fails_unchanged`.
  - `set_reports_changes`.
  - `set_unchanged_keeps_bytes_and_mtime`:
    1. Sleep 1.1 s after the first write so coarse-mtime filesystems can tell writes apart.
    2. Record the bytes, `modified()` and (Unix) `ino()`.
    3. Run the unchanged `set`.
    4. Assert all three are unchanged.
  - `set_name_only_on_existing_is_unchanged`.
  - `set_invalid_name_has_no_help`: exit 2; stderr does not contain `Usage:`.
  - `check_valid`.
  - `check_invalid_exits_2`.
  - `check_under_held_lock_does_not_wait_or_write`: hold `serve.toml.lock` with `File::lock()`, as in `lock_held_by_another_writer_times_out_with_a_clear_error`, on a file containing `multiplex = true`. `config check` exits 0 in under 2 s and the bytes are unchanged.
  - Extend `every_config_subcommand_honors_config_and_ignores_the_default` with `set`, `rm` and `check`.
- [ ] **Step 2:** Add Windows equivalents to `config_lifecycle_windows.rs`: `set_creates_and_updates_entry`, `set_unchanged_keeps_bytes`, `check_valid_and_invalid`, and `set`/`rm`/`check` in `every_config_subcommand_honors_config`. Make no mode assertions.
- [ ] **Step 3:** Run `cargo test --test config_lifecycle`. Expected: PASS. Fix any failure test-first in the owning module.
- [ ] **Step 4:** Commit `test: e2e coverage for config set and check`.

## Task 9: Usage and help e2e — implements 5.3

**Files:**
- Create: `tests/cli_usage.rs` (OS-neutral: no `#[cfg(unix)]`, no mode assertions; isolated HOME, as `cli_golden::World` does)

- [ ] **Step 1:** Write the tests. Unless stated otherwise, each asserts exit 2, empty stdout, and stderr starting with `error:`.
  - `unknown_option_on_leaf_shows_leaf_help`: `it serve config set -n web --bogus` mentions `--bogus` and shows `Usage: iroh-tunnel serve config set`.
  - `unknown_subcommand_shows_group_help`: `it serve config bogus` shows `Usage: iroh-tunnel serve config` and lists `set`, `rm` and `check`.
  - `missing_quick_flag_shows_standalone_help`: `its -n web` shows `--target` in the error paragraph and `Usage: iroh-tunnel-serve`.
  - `invalid_protocol_lists_values`: `it serve config set -n web -t 3000 -p sctp`. The **first paragraph** of stderr contains `possible values: tcp, udp`, and stderr contains `Usage: iroh-tunnel serve config set`.
  - `wrong_role_flag_on_set`: both directions.
  - `relay_flag_rejected_on_set`.
  - `rm_without_name_shows_rm_help`: the first paragraph contains `--name`; stderr shows `Usage: iroh-tunnel serve config rm`.
  - `set_without_name_shows_set_help`.
  - `bare_invocations_print_help`: `it`, `its`, `ita`, `it serve`, `it access`, `ita config`, `it serve service`. Each exits 2 with help on stderr. `it serve` stderr does not contain `is required`.
  - `malformed_addresses`: `:3000`, `host:`, `::1:8080`, `not-an-address`, `0`, `host:70000`. The first paragraph mentions `--target`, and help follows.
  - `invalid_color_lists_values`: the first paragraph contains `possible values: auto, always, never`.
  - `removed_syntax_is_usage_error`: `config add`, `config remove`, `config keygen random`.
  - `keygen_print_with_config_is_usage_error`: no file is created.
  - `examples_in_help`: exit 0 with `--help` for `serve config set`, `serve config rm`, `serve config keygen`, `serve`, `access` and `access config set`. Each contains `Examples:`; `access config set` also contains `-l` and `-N`.
  - `protocol_values_in_help`: `it serve --help` and `it serve config set --help`.
  - `runtime_config_error_has_no_usage`: `config list` on broken TOML exits 2; stderr contains `line` and no `Usage:`.
- [ ] **Step 2:** Run `cargo test --test cli_usage`. Expected: PASS. Fix any failure test-first.
- [ ] **Step 3:** Commit `test: usage-error and help e2e coverage`.

## Task 10: Quick mode e2e — implements 5.4

**Files:**
- Modify: `tests/quick_run.rs`

- [ ] **Step 1:** Write the tests.
  - `quick_bare_port_target_forwards`: echo on `127.0.0.1:<p>`, `-t <p>`.
  - `quick_hostname_target_forwards`: echo listeners on `127.0.0.1:<p>` and, if bindable, `[::1]:<p>`; run with `-t localhost:<p>`.
  - Change `quick_ipv6_listener_forwards_bytes` to `-l [::1]:<p>`.
  - Extend `invalid_quick_inputs_fail_without_config_or_key_writes`: addresses `not-an-address`, `127.0.0.1:65536`, `127.0.0.1:0`; `-n BAD` stderr has no `Usage:`; `--relay-url http://localhost:1` stderr contains `https://` and no `Usage:`.
  - `quick_to_stored_entry_matches`: `it serve config set -c c.toml -n web -t 3000` stores host `127.0.0.1`, port 3000, tcp. That is what `it serve -n web -t 3000` uses (asserted through `quick_bare_port_target_forwards`).
- [ ] **Step 2:** Run `cargo test --test quick_run --test role_binaries`. Expected: PASS.
- [ ] **Step 3:** Commit `test: quick mode addresses, hostname and help coverage`.

## Task 11: Verification gate — implements 6.1, 6.2

- [ ] **Step 1:** Run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace`. Expected: all pass, with 0 warnings. Run `cargo check --tests --target x86_64-pc-windows-msvc` if installed; otherwise note in apply.md that Windows is covered by CI.
- [ ] **Step 2:** Smoke test with `HOME=$(mktemp -d)` and `B=target/debug/iroh-tunnel`. Record each command, its exit code and its output in apply.md:
  - `$B serve`
  - `$B serve config set -n web`
  - `$B serve config set -n web -t 3000`
  - `$B serve config set -n web -t 3001 -p udp`, run twice (the second run must report `unchanged`)
  - `$B serve config check`
  - `$B serve config rm -n web`
  - `$B serve config keygen --print -c x.toml`
  - quick `$B serve -n w -t localhost:<echo>` with `$B access -n w -l <p> -N <id>`, plus a byte round-trip

  Expected: outputs as in Contracts.
- [ ] **Step 3:** Fix anything found test-first, then commit. No commit if nothing was found.

## Task 12: Docs and CHANGELOG — implements 7.1, 7.2

**Files:**
- Modify: `README.md`, `docs/cli.md`, `docs/configuration.md`, `docs/services.md`, `docs/deployment.md`, `docs/troubleshooting.md`, `CHANGELOG.md`

- [ ] **Step 1:** Update the docs:
  - examples use `config set -n`, `rm -n`, `keygen --print` and `check`;
  - add the short-option table and the "usage errors print help" note to `docs/cli.md`;
  - document `[HOST:]PORT`, including hostnames.
- [ ] **Step 2:** Add a CHANGELOG `## [Unreleased]` entry: `### BREAKING` with an old → new table, and `### Added` with `config check`, usage-error help, hostname addresses and `-r`.
- [ ] **Step 3:** Run `rg -n "config add|config remove|keygen random" README.md docs examples src tests crates`. Expected: no output.
- [ ] **Step 4:** Commit `docs: config set/rm/check syntax and BREAKING changelog`.

---

## Scenario → test

**cli-entrypoints**
- Unknown option on a leaf command → `cli_usage::unknown_option_on_leaf_shows_leaf_help`
- Unknown subcommand → `cli_usage::unknown_subcommand_shows_group_help`
- Missing quick-mode flag → `cli_usage::missing_quick_flag_shows_standalone_help`, `usage::summary_keeps_continuation_lines`
- Invalid value → `cli_usage::invalid_protocol_lists_values`
- Schema rule failure has no help → `config_lifecycle::set_invalid_name_has_no_help`, `quick_run::invalid_quick_inputs_fail_without_config_or_key_writes` (`-n BAD`)
- Wrong-role flag on config set → `cli_usage::wrong_role_flag_on_set`
- Relay flag not accepted by config set → `cli_usage::relay_flag_rejected_on_set`, `cli::config_set_has_no_relay_flag`
- Runtime error has no help → `cli_usage::runtime_config_error_has_no_usage`
- Help goes to stderr → stdout-empty assertion in every `cli_usage` error test
- Bare role → `cli_usage::bare_invocations_print_help`, `cli::bare_invocations_request_help`
- Bare config → same
- Bare executables and service group → same
- Protocol values shown → `cli_usage::protocol_values_in_help`
- No blank descriptions → `cli::no_blank_descriptions`
- Examples present → `cli_usage::examples_in_help`
- Examples on every listed command → `cli_usage::examples_in_help`, `cli::examples_on_listed_commands`
- Identical help text → `cli::shared_flags_match_between_quick_and_set`
- Quick to stored → `quick_run::quick_to_stored_entry_matches`
- Bare port in quick mode → `quick_run::quick_bare_port_target_forwards`
- Hostname in quick mode → `quick_run::quick_hostname_target_forwards`
- Bracketed IPv6 → `quick_run::quick_ipv6_listener_forwards_bytes`, `cli_args` parser test `[::1]:8080`
- Invalid port → `cli_usage::malformed_addresses`, `cli_args` parser tests
- Malformed addresses → `cli_usage::malformed_addresses`
- Help lists options and aliases → `role_binaries::help_lists_role_options_aliases_and_defaults`
- Version on standalone binaries → `role_binaries::help_lists_role_options_aliases_and_defaults` (runs `-V`)
- Invalid color → `cli_usage::invalid_color_lists_values`
- Short letters do not collide → `cli::short_options_are_consistent_and_conflict_free`
- Long-only options → `cli::long_only_options`
- TCP quick forwarding → `quick_run::quick_tcp_forwards_bytes_to_fixed_target`
- UDP quick forwarding → `quick_run::quick_udp_forwards_datagrams_to_fixed_target`
- IPv6 addresses → `quick_run::quick_ipv6_target_forwards_bytes`, `quick_run::quick_ipv6_listener_forwards_bytes`
- Short and long flags equivalent → `quick_run::quick_long_and_short_flags_reach_same_fixed_target`
- Rejected input classes → `quick_run::invalid_quick_inputs_fail_without_config_or_key_writes`
- Bad protocol → `quick_run::quick_config_errors_use_existing_exit_classification` (now `possible values`)
- Bad address or missing value → `quick_run::invalid_quick_inputs_fail_without_config_or_key_writes`
- Insecure relay URL → `quick_run::invalid_quick_inputs_fail_without_config_or_key_writes` (`https://`, no `Usage:`)
- Config path → `quick_run::configured_subcommands_allow_implicit_tcp_default`
- Keygen random conflicts → `cli::keygen_print_conflicts`, `cli_usage::keygen_print_with_config_is_usage_error`
- Removed syntax → `cli_usage::removed_syntax_is_usage_error`
- Usage error → `cli_usage::missing_quick_flag_shows_standalone_help` (exit 2)
- Config error → `cli_usage::runtime_config_error_has_no_usage` (exit 2)
- Service error → `role_binaries::standalone_service_install_is_refused_instead_of_writing_a_broken_unit`

**config-management-commands**
- Serve create with defaults → `config_lifecycle::set_creates_serve_entry_with_defaults`, `set_service::create_applies_defaults`
- Access create → `config_lifecycle::set_creates_access_entry`
- Hostname target → `config_lifecycle::set_accepts_hostname_target`
- Create on a missing file → `config_lifecycle::set_create_on_missing_file`
- Existing comments preserved → `set_service::create_preserves_comments_and_entries`
- Serve create without target → `config_lifecycle::set_without_target_shows_set_help_and_writes_nothing`, `set_service::create_serve_requires_target`
- Access create without node id → `set_service::create_access_requires_listen_and_node_id`
- Access create with nothing but a name → `config_lifecycle::access_set_with_name_only_names_listen_and_node_id`
- Change port only → `config_lifecycle::set_bare_port_update_keeps_host`, `set_service::update_port_keeps_host_and_protocol`
- Replace host and port → `config_lifecycle::set_host_port_update_replaces_both`, `set_service::update_host_and_port`
- Unrelated keys kept → `config_lifecycle::set_update_keeps_udp_settings`, `set_service::update_keeps_udp_settings_comments_and_decor`
- Update producing an invalid entry → `config_lifecycle::set_udp_to_tcp_with_udp_settings_fails_unchanged`
- Change report → `config_lifecycle::set_reports_changes`, `render::render_updated_lists_changes`, `cli_golden::config_set_updated_report`
- Nothing changed → `config_lifecycle::set_unchanged_keeps_bytes_and_mtime`
- Name only on existing entry → `config_lifecycle::set_name_only_on_existing_is_unchanged`
- Valid config → `config_lifecycle::check_valid`, `cli_golden::config_check_ok_message`
- Invalid config → `config_lifecycle::check_invalid_exits_2`
- Check never writes → `config_lifecycle::check_under_held_lock_does_not_wait_or_write`, `check_config::check_ignores_held_lock_and_never_writes`
- Explicit path wins → `config_lifecycle::every_config_subcommand_honors_config_and_ignores_the_default`
- Access role uses the same flag → same test (access branch)
- Remove existing → `remove_service::removing_a_service_drops_its_table`, `cli_golden::config_rm_message`
- Remove unknown name → `remove_service::removing_an_unknown_service_is_a_config_error`
- Missing name → `cli_usage::rm_without_name_shows_rm_help`
- Missing file with missing parent → `config_lifecycle::remove_on_a_missing_config_creates_nothing`
- No config present → `config_lifecycle::read_commands_without_config_share_one_guidance_error_and_create_nothing` (adds `check`)
- Random key → `config_paths::keygen_print_prints_matching_key_and_node_id_without_touching_config`, `cli_golden::keygen_print_prints_key_and_node_id_lines`
- Print with config → `config_paths::keygen_print_rejects_config_path_instead_of_ignoring_it`, `cli_usage::keygen_print_with_config_is_usage_error`
- Invalid existing config → `config_lifecycle::invalid_mutations_fail_and_leave_file_bytes_unchanged`
- Invalid new value → `config_lifecycle::set_invalid_name_has_no_help`
- Comment preserved → `config_lifecycle::concurrent_sets_serialize_without_losing_updates` (comment assertion)
- Legacy multiplex → `config_multiplex_compat` (migrated)
- Eight parallel adds → `config_lifecycle::concurrent_sets_serialize_without_losing_updates`
- Lock held by another process → `config_lifecycle::lock_held_by_another_writer_times_out_with_a_clear_error`
- Killed writer → `config_lifecycle::killed_or_concurrent_saves_never_expose_a_truncated_file`
- Permissions tightened → `config_lifecycle::valid_edit_is_applied_atomically_and_owner_only`
- Read-only target → `config_multiplex_compat::config_write_error_is_reported_and_readonly_file_preserved`

**frontend-independent-management**
- Add returns the affected entry → `set_service::create_applies_defaults`
- Random key touches no file → `config_paths::keygen_print_creates_no_config_and_differs_each_run`
- Update returns the changes → `set_service::update_port_keeps_host_and_protocol`
- Unchanged writes nothing → `set_service::equal_values_are_unchanged_and_not_written`
- Check returns the validated path → `check_config::check_valid_returns_path`
- Invalid mutation changes nothing → `set_service::create_access_requires_listen_and_node_id`
- Concurrent mutations serialize → `config_lifecycle::concurrent_sets_serialize_without_losing_updates`

**tunnel-configuration**
- Unreadable legacy → `config_paths::macos_migration::copy_failure_never_creates_a_substitute_identity` (its `config add` invocation migrated to `config set` in Task 5)
- Existing config untouched → `config_paths::keygen_print_prints_matching_key_and_node_id_without_touching_config`
- Conflicting option → `config_paths::keygen_print_rejects_config_path_instead_of_ignoring_it`

**workspace-boundaries**
- CLI mutations go through host operations → `workspace_layout::host_depends_neither_on_root_nor_clap`; `config_cmd` calls only `iroh_tunnel_host::config::*` (code review)
- Creation requirements decided by host → `set_service::create_serve_requires_target` (decided inside `mutate_deciding`), `config_lifecycle::set_without_target_shows_set_help_and_writes_nothing`
