## Context

**Current shape.**
- `src/cli.rs` defines the clap tree for the all-in-one binary (`Cli`) and for the two standalone binaries (`ServeCli`, `AccessCli`). Both standalone types convert into `Cli`, so all three share one dispatcher in `src/app.rs`.
- `src/config_cmd.rs` holds the CLI handlers. Each one calls a terminal-free operation in `crates/host/src/config/` (`add_service`, `remove_service`, `keygen`, `random_identity`, `show_config`, `prepare_edit`, ...) and prints the result through `src/render.rs`.
- Every mutation goes through `mutate` / `mutate_deciding`. The steps are: take the lock, load the doc (empty when absent), apply the closure, strip retired fields, validate the doc against the role schema, then write atomically and owner-only. The file is unchanged on any failure.

**Error path today.**
- Parsing uses `Cli::parse()`, so a clap error exits through clap itself, prints a one-line usage and exits 2.
- Post-parse validation (quick-mode required flags, access `--node-id`) returns `CliError::Config`. `execute` prints that as `error: config error: ...` with exit 2, and no usage or help.
- `CliError` is the exit-code taxonomy: 2 config, 3 permission, 4 iroh, 5 service, 6 conflict, 1 other.

**Constraints.**
- `iroh-tunnel-host` must not depend on clap or touch the terminal (workspace-boundaries).
- Secret hygiene (RS-10): diagnostics never echo `secret_key` or `relay_token`.
- Exit classes are preserved.
- Config persistence guarantees are preserved: locked, atomic, owner-only and validated writes.
- Three binaries share one surface.

## Goals / Non-Goals

**Goals:**
- Replace `add`/`remove` with `set -n NAME` (upsert, with a change report) and `rm -n NAME`. Add `check`. Rename `keygen random` to `keygen --print`.
- Make every usage error, whether clap-detected or post-parse, print the error plus the full help of the failing subcommand, exit 2. A bare role or `config` prints help.
- Give every option a description, its default and its possible values; add Examples sections and the shared short-option table.
- One argument vocabulary: quick mode and `config set` share the same arg definitions (`-n -t -l -N -p`), so long, short, value syntax and help cannot diverge.

**Non-Goals:**
- `[node]` setters.
- Config format changes.
- Changes to `run`, `status`, `service` or `update` other than the shared error/help behaviour.
- New crates.
- A rewrite of the persistence layer.

## Decisions

### D1. Usage errors are a distinct error kind rendered by the CLI crate
- **What.** Introduce a CLI-crate-only `UsageError { command_path: Vec<String>, message: String }` (in `src/`, not in host).
  - Post-parse checks that are about how the command was typed return it: quick-mode required flags, and creation-time required attributes of `set`.
  - `execute` downcasts it before mapping to `CliError`. It then prints `error: <message>`, a blank line and the rendered help of the subcommand at `command_path`, and exits 2.
- **Clap errors.** `try_parse()` replaces `parse()`.
  - For `ErrorKind::DisplayHelp` / `DisplayVersion`, keep clap's behaviour: print and exit 0.
  - For `DisplayHelpOnMissingArgumentOrSubcommand` (bare role or `config`), print help and exit 2, as clap does.
  - For every other kind, print clap's own error line (its first paragraph, without the "For more information" tail) followed by the full help of the deepest subcommand matched in argv, and exit 2.
- **Finding the subcommand.** Walk `Cli::command()` along the leading non-flag tokens of argv while they name subcommands. The standalone binaries walk from their own root command.
- **Why a separate kind, not `CliError::Config`.** "Config error" means the file or values are wrong. A usage error means "you typed it wrong; here is how", and only the latter should show help. Keeping it out of host also respects the no-clap boundary: host returns domain errors, and the CLI decides they are usage errors only where the cause is the invocation itself.
- **Alternatives rejected.** (a) Clap's `error.render()` with `help_template` tweaks: it cannot attach help to post-parse errors. (b) Printing help for every `CliError::Config`: that buries real file errors, an anti-goal.

### D2. Requirements: parse-time where clap can know, creation-time in the use case
- Clap enforces:
  - required `-n/--name` on `set` / `rm`;
  - value enums (`protocol`, `color`);
  - address syntax;
  - conflicts (`keygen --print` vs `-c` / `-y`; quick flags vs subcommands);
  - quick-mode required flags, via `required = true` together with `subcommand_negates_reqs` / `args_conflicts_with_subcommands`, so missing `-n` / `-t` become clap errors and D1 shows their help.
- Whether `set` needs an address (and `-N` on access) depends on whether the entry exists, which only the host knows under the lock.
  - Host `set_service` returns a domain error `SetError::MissingForCreate { name: String, flags: Vec<&'static str> }`, carried in anyhow as a typed error.
  - The CLI maps it to `UsageError` with the `config set` command path. The message reads `creating service 'web' requires --target`.
- **Why.** Clap cannot express exists-dependent requirements, and checking existence outside the lock would race (TOCTOU) against concurrent writers.

### D3. `set` is one locked read-modify-write via `mutate_deciding`
- **Request.** Host gets `set_service(role, &SetService) -> Result<SetOutcome>`. `SetService` holds `name`, an optional address, and optional `protocol` and `node_id` (access), each `None` when not passed.
- **Under the lock:**
  - If the entry is absent, the missing creation fields produce `MissingForCreate`. Otherwise build the table with defaults (protocol `tcp`, host `127.0.0.1`), append it, and return `Created`.
  - If the entry is present, edit only the passed keys in the existing `toml_edit::Table`, which preserves the other keys, unknown UDP settings and comments. Record each `(field, old, new)` where the value differs. With no differences, return `Edit::Keep(Unchanged)` so nothing is written. Otherwise return `Updated { changes }`.
- Validation still runs through `validate_role_doc` for the whole document, so name rules, port range, node-id format and https relays are enforced as today.
- `add_service` is replaced by `set_service`; `remove_service` stays and is exposed as `rm`.
- **Change report.** The field labels are user-facing (`target`, `listen`, `protocol`, `node_id`), with values shown as written in TOML. It never includes `[node]` keys, so there is no secret exposure.
- **`set` with no attribute flags on an existing entry** is not an error. It prints `Service 'web' unchanged (<path>).` and exits 0 (resolves brainstorm open question 1). On a missing entry it is `MissingForCreate`.

### D4. Address value type `[HOST:]PORT`
- **Type.** A CLI-side `ServiceAddr { host: Option<String>, port: u16 }` parsed by a clap `value_parser`:
  - a bare `u16` gives `host: None`;
  - otherwise `HOST:PORT`, where HOST may be an IPv4 address, a bracketed IPv6 address (`[::1]:3000`) or a hostname (the config already allows hostname targets);
  - port 0 is rejected at parse time.
- **Quick mode** resolves `host: None` to `127.0.0.1` and passes `host` and `port` into the in-memory service config, exactly as a file entry does. The engine already binds and dials a `host:port` string, so hostnames work with no engine change. The old `SocketAddr` parse was the only reason quick mode accepted IP literals only.
- **`set`.** `host: None` keeps the existing host on update and uses the default on create.
- **Why one parser.** It gives one syntax and one help string, and quick mode and `set` cannot drift apart.

### D5. Protocol and color are `ValueEnum`
- `Protocol { Tcp, Udp }` in the CLI crate (`clap::ValueEnum`) maps to host `config::Protocol` and gives `[possible values: tcp, udp]` and `[default: tcp]` in help.
- **The default is set only for quick mode.** `config set` has none, so that an update can tell "not passed" from "tcp". The create-time default lives in the host use case, and help states `(default on create: tcp)`.
- `--color` becomes `ValueEnum { Auto, Always, Never }`.

### D6. `keygen --print` is a flag, not a subcommand
- `Keygen { config, yes, #[arg(long, conflicts_with_all = ["config", "yes"])] print }`. This removes `args_conflicts_with_subcommands` from `Keygen`.
- Behaviour and output are the existing `random_identity` and `render_identity`.

### D7. `config check`
- Host `check_config(role, config) -> Result<PathBuf>` resolves the path, calls `require_existing` (same missing-config guidance as `list` / `show`), reads, parses and validates with the same `validate_role_doc` used by mutations, and never takes the writer lock or writes.
- The CLI prints `OK: <path> is a valid <role> config.`; an invalid file is a config error (exit 2) with the existing secret-free diagnostic.

### D8. Help content lives on the clap definitions
- Every arg gets a doc comment. Defaults are shown by clap (`[default: …]`) or, for create-time defaults, written in the text.
- `after_help` / `after_long_help` carries an `Examples:` block on `set`, `rm`, `keygen` and both quick-mode roots. Examples are written per role, because serve and access have different `set` arguments.
- **Short options.** Short options follow the brainstorm table. The existing `short_options_are_consistent_and_conflict_free` test is extended:
  - walk every subcommand, `config` included;
  - assert the table (`c t l N p r k y n`);
  - assert that `--reveal`, `--print`, `--system` and `--user` have no short form.

### D9. Role-specific service args, shared by quick mode and `set`
- Define one `ServeServiceArgs { name, target, protocol }` and one `AccessServiceArgs { name, listen, node_id, protocol }`, each field carrying its long name, short letter, value parser and doc comment exactly once.
  - Quick mode flattens them plus its run-only flags: `-r/--relay-url` (the node's relay) and `-k/--key-file`.
  - `config set` flattens them plus `-c`.
- **Required-ness differs per context.** In quick mode, name and address (plus node id) are required. In `set`, only the name is required at parse time (D2). So the shared structs hold `Option`s, and quick mode marks them required at the command level with `mut_arg(..., |a| a.required(true))`, keeping one definition.
- **No `-r` on `config set`.** `-r` means the node's relay, which is outside a service entry (non-goal). Reusing the letter for the access per-service override would give one flag two meanings.
- `RoleCmd` is shared today. `config set` needs different args per role.
- Make `RoleCmd`, and with it `ConfigAction`, generic over a role-args type: `RoleCmd<S: Args>`, with `ServeSetArgs` / `AccessSetArgs` and the rest shared.
- **Why.** Wrong-role flags then become clap "unexpected argument" errors with the right help, instead of a runtime check, and each role's `--help` lists only its own flags.
- **Alternative rejected.** One struct with every flag plus runtime rejection: it shows misleading help and moves errors out of clap.

### Boundaries and invariants (DDD)
- **Bounded context unchanged.** Service configuration management lives in host `config`; the CLI adapter (`src/`) handles parsing, usage errors, help and rendering.
- **Host gains:**
  - `set_service`, `SetService`, `SetOutcome { Created | Updated { changes } | Unchanged }` and `SetError::MissingForCreate`;
  - `check_config`.
- **Host loses:** `add_service` and `AddService`.
- **Invariants kept:**
  - service names are unique per document;
  - each entry passes the role schema;
  - mutations are all-or-nothing under the lock;
  - an update never applies defaults;
  - no secret appears in any CLI output except `show --reveal` and the `keygen` outputs.

## Risks / Trade-offs

- **[Breaking CLI for scripts and docs]** → BREAKING CHANGELOG entry with old → new table. All repo docs, examples and tests are migrated in this change, and a grep gate in the plan checks that no `config add`, `config remove` or `keygen random` remains outside CHANGELOG history.
- **[Help on every usage error is long in narrow terminals]** → Accepted by the operator (Q1). The error line comes first, so it stays visible; help goes to stderr, so stdout remains clean for pipes.
- **[Deepest-subcommand detection can pick a parent when argv is very malformed]** → Fall back to the deepest successfully matched command. A test covers unknown flags at each level (root, role, config, set).
- **[Generic `RoleCmd<S>` adds type noise]** → Limited to one type parameter at the `config set` leaf. The standalone binaries reuse the same instantiation.
- **[Golden tests pin old help/error text]** → Re-bless `cli_golden` intentionally, and review the diff of the goldens as part of code review.
- **[`toml_edit` in-place update could reorder or reformat keys]** → Update the existing table's values in place, without rebuilding it. A test asserts that comments and unrelated keys (UDP settings) survive.
- **[Hostnames in quick mode resolve at bind/dial time]** → An unresolvable hostname fails at startup with the existing bind or dial error classification. A test covers `-t localhost:PORT` forwarding.
- **[Shared arg structs drift per context]** → The short-option test also asserts that each shared flag has the same spelling, value name and description in quick mode and `config set`. The only allowed difference is the `-p` default note.

## Migration Plan

- There is no data migration; config files are unchanged.
- Release as a minor bump with a BREAKING note under the project's pre-1.0 versioning.
- Rollback means reinstalling the previous binary; config files stay compatible both ways.

## Open Questions

- None blocking. The wording of the change report and the Examples sections is fixed in specs scenarios and the plan's Contracts section.
