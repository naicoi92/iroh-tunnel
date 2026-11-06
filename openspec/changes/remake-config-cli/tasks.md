<!--
File ownership: group 1 owns crates/host/src/config/** only; groups 2-6 own src/** and tests/**;
group 7 owns docs/**, README.md, CHANGELOG.md. One writer per worktree; groups run in order
(2 depends on 1; 3-6 depend on 2; 7 depends on 3-6). Platforms: logic is OS-neutral; config
mutation/check tests also run in tests/config_lifecycle_windows.rs and config_paths_windows.rs.
Traceability: CMC = config-management-commands, CLI = cli-entrypoints,
FIM = frontend-independent-management, TC = tunnel-configuration, WB = workspace-boundaries.
-->

## 1. Host: set and check operations (crates/host)

- [x] 1.1 Add `SetService`, `SetOutcome { Created, Updated { changes }, Unchanged }`, `FieldChange` and the typed `SetError::MissingForCreate { name, flags }` to `config/types.rs`, with unit tests for their `Display`/`Error`. The display text names the CLI flags; no secrets. (FIM: Configuration operations return outcomes; FIM: Configuration mutations keep store guarantees)
- [x] 1.2 Implement `set_service` in `config/set_service.rs` on `mutate_deciding`, and replace `add_service.rs`:
  - create applies the defaults (tcp, 127.0.0.1) and returns `MissingForCreate` for a missing address or access node_id;
  - update edits only the given keys in place;
  - a bare port keeps the existing host;
  - no difference gives `Edit::Keep(Unchanged)`.

  TDD per scenario. (CMC: Set creates a service entry; Creating a service requires its address and peer; Set updates only the given attributes; Set reports what changed; FIM scenarios Add returns the affected entry, Update returns the changes, Unchanged writes nothing, Invalid mutation changes nothing)
- [x] 1.3 Implement `check_config` in `config/check_config.rs`. It does `require_existing`, then read and `validate_role_doc`, with no lock and no write. Tests cover a valid file, an invalid file, a missing file, and a held lock with a legacy `multiplex` key left untouched. (CMC: Check validates a config without writing; FIM: Check returns the validated path)
- [x] 1.4 Update the `config/mod.rs` exports:
  - drop `add_service`/`AddService`;
  - export `set_service`, `check_config` and the new types;
  - port the host unit tests and fixtures that used `add_web` to `set_service`;
  - `cargo test -p iroh-tunnel-host` passes;
  - `cargo tree -p iroh-tunnel-host` shows no clap.

  (WB: Host performs no terminal interaction; CMC: Mutations are validated and all-or-nothing)

## 2. CLI: argument model (src/cli.rs)

- [x] 2.1 Add the CLI value types:
  - `ServiceAddr` value parser (`PORT | HOST:PORT`, last-colon split, bracketed IPv6, hostname charset, rejects empty, 0 and unbracketed IPv6);
  - `ProtocolArg` and `ColorArg` `ValueEnum`s.

  Unit tests cover each grammar case. (CLI: Addresses accept PORT or HOST:PORT; CLI: Top-level options and help scenario Invalid color)
- [x] 2.2 Define `ServeServiceArgs` (`-n -t -p`) and `AccessServiceArgs` (`-n -l -N -p`) once, with descriptions:
  - flatten them into quick mode with `-r` and `-k`, marking name and address (and node id) required via `mut_arg` with `subcommand_negates_reqs`;
  - flatten them into a role-generic `config set` with `-c`;
  - `-p` gets `[default: tcp]` in quick mode only.

  (CLI: One argument vocabulary across quick mode and config; CLI: Quick mode forwards a single service from flags)
- [x] 2.3 Reshape `ConfigAction` and `RoleCmd` to be generic over the role's set args:
  - `Set`, `Rm { -n }`, `List`, `Show { --reveal }`, `Edit`, `Path`, `Check`, `Keygen { -c, -y, --print conflicts_with_all }`;
  - drop `Add`, `Remove`, `KeygenMode`;
  - `-c` on every config subcommand.

  (CLI: Config actions dispatch; CMC: Config file target resolution; CMC: Random keygen touches no file; TC: Random key generation prints an identity without touching configuration)
- [x] 2.4 Write the help content:
  - a description on every arg and command;
  - `after_help` Examples for serve and access `config set`, `config rm`, `config keygen` and both quick-mode roots (all-in-one and standalone);
  - `arg_required_else_help` on `it`, roles, `its`/`ita`, `config` and `service`.

  (CLI: Help documents every option; CLI: Bare commands print their help; CLI: Operator guidance on target protection unchanged)
- [x] 2.5 Extend the `cli.rs` unit tests:
  - walk every subcommand of `it`/`its`/`ita` and assert the short-option table `c n t l N p r k y`;
  - assert that `reveal print system user` have no short;
  - assert there are no empty arg descriptions;
  - assert that shared flags have the same spelling, value name and description in quick mode and `config set` (only the `-p` default note may differ);
  - `config set` has no `-r`.

  (CLI: Consistent short options across roles; CLI: Help documents every option scenario No blank descriptions; CLI: One argument vocabulary scenario Identical help text)

## 3. CLI: usage errors with help (src/app.rs)

- [x] 3.1 Add `UsageError { command_path, message }` in `src/usage.rs` and a renderer that prints `error: <msg>`, a blank line, then the help of `command_path` to stderr, exit 2. Unit tests cover the rendered text. (CLI: Usage errors print the failing command's help)
- [x] 3.2 Switch the three entry points to `try_parse`:
  - `DisplayHelp`/`DisplayVersion` exit 0;
  - help-on-missing exits 2;
  - every other clap error prints clap's message plus the help of the deepest subcommand matched in argv, exit 2, with stdout empty.

  (CLI: Usage errors print the failing command's help scenarios Unknown option on a leaf command, Unknown subcommand, Invalid value, Help goes to stderr; CLI: Bare commands print their help)
- [x] 3.3 Remove the post-parse quick-mode required checks and `quick_protocol`, since clap now enforces them. Quick mode builds the in-memory service from `ServiceAddr` (bare port maps to 127.0.0.1; hostnames are passed through). Name, node-id and https-relay rules stay config errors with no help. (CLI: Quick mode input validation; CLI: Usage errors print the failing command's help scenario Schema rule failure has no help)
- [x] 3.4 In `execute`, downcast `UsageError` before the `CliError` mapping, and map host `SetError::MissingForCreate` to a `UsageError` at the role's `config set` path. Every other error keeps today's exit-code mapping. (CLI: Exit status mapping; CMC: Creating a service requires its address and peer; WB: Host owns persistence and platform policy scenario Creation requirements decided by host)

## 4. CLI: config handlers and rendering (src/config_cmd.rs, src/render.rs)

- [x] 4.1 Add the `config set` handler and renderers:
  - `Created service '<n>' in <p>.`;
  - `Updated service '<n>' in <p>:` with lines `  <field>: <old> -> <new>` (labels `target`/`listen`/`protocol`/`node_id`);
  - `Service '<n>' unchanged (<p>).`

  Also add the `config rm` handler (renamed from remove). (CMC: Set reports what changed; CMC: Remove a service entry)
- [x] 4.2 Add the `config check` handler (`OK: <path> is a valid <role> config.`) and the `keygen --print` dispatch. Update the missing-config guidance text from `config add` to `config set`. (CMC: Check validates a config without writing; CMC: Read commands on a missing config share one guidance error; TC: Failed macOS migration never creates a substitute identity)

## 5. Integration tests (tests/)

- [x] 5.1 Migrate `config_lifecycle.rs`, `config_lifecycle_windows.rs`, `config_paths.rs`, `config_paths_windows.rs`, `config_multiplex_compat.rs` and `configured_multiplex.rs` to `set -n`/`rm -n`/`keygen --print`. Keep every existing scenario: parallel writers, killed writer, lock timeout, read-only target, permissions, symlink, invalid config, secret-free diagnostics. (CMC: Concurrent writers are serialized; Saves are atomic and owner-only; Mutations are validated and all-or-nothing; Config edits preserve formatting and drop retired fields; Remove on a missing config creates nothing)
- [ ] 5.2 Add the `config set` and `check` e2e cases in `config_lifecycle.rs`:
  - create with defaults, access create, hostname target;
  - create without `-t`/`-l`/`-N`, which gives help and no file;
  - bare-port update keeps the host, and HOST:PORT update;
  - UDP settings kept on update, and an invalid protocol switch rejected;
  - change report, and unchanged with the mtime kept;
  - `-n BAD` gives no help;
  - `check` valid, invalid, missing, and under a held lock.

  (CMC ADDED requirements, all scenarios)
- [ ] 5.3 Add a usage-error and help e2e file `tests/cli_usage.rs`:
  - unknown option on a leaf, unknown subcommand, invalid enum value;
  - bare `it`/`its`/`ita`/role/`config`/`service`;
  - wrong-role flag on `config set`, and `-r` on `config set`;
  - malformed addresses, invalid color;
  - removed syntax (`config add`, `config remove`, `keygen random`);
  - Examples present on every listed command;
  - a runtime config error without `Usage:`.

  (CLI ADDED requirements; CLI: Config actions dispatch scenario Removed syntax)
- [ ] 5.4 Update `quick_run.rs` and `role_binaries.rs`:
  - bad protocol lists the possible values;
  - missing flag shows help;
  - bare-port target forwards;
  - `-t localhost:PORT` against an echo bound on every loopback address `localhost` resolves to;
  - bracketed IPv6 listen;
  - insecure relay with no help;
  - help-listing assertions include `-r/--relay-url`.

  (CLI: Quick mode forwards a single service from flags; Quick mode input validation; Addresses accept PORT or HOST:PORT; Top-level options and help)
- [x] 5.5 Rename and re-bless `cli_golden`: the `config_add_*` golden becomes `config_set_created_*`, `config_remove_*` becomes `config_rm_*`, and `keygen_random_*` becomes `keygen_print_*`. Add goldens for an updated change report and for one usage error with help. Review the golden diff. (CMC: Set reports what changed; CLI: Usage errors print the failing command's help)

## 6. Verification gate

- [ ] 6.1 Run `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace`; all must be green on macOS. Also run `cargo check --tests --target x86_64-pc-windows-msvc` if the toolchain is present; otherwise note in apply.md that the Windows tests run in CI.
- [ ] 6.2 E2E smoke run of the real binary with an isolated HOME. Record each command and its output in apply.md:
  - `it serve`;
  - `it serve config set -n web`;
  - `it serve config set -n web -t 3000`;
  - `... -t 3001 -p udp`;
  - `it serve config rm -n web`;
  - `it serve config check`;
  - `it serve config keygen --print -c x`;
  - `it serve -n w -t localhost:<echo>` forwarding.

## 7. Docs and release notes

- [ ] 7.1 Update `README.md`, `docs/cli.md`, `docs/configuration.md`, `docs/services.md`, `docs/deployment.md` and `docs/troubleshooting.md` to the new syntax, the short options and the error/help behaviour. Grep gate: `rg -n "config add|config remove|keygen random" README.md docs examples src tests crates` returns nothing.
- [ ] 7.2 Add a CHANGELOG `Unreleased` entry with a **BREAKING** section listing the old and new commands (`config add` becomes `config set -n`, `config remove --name` becomes `config rm -n`, `keygen random` becomes `keygen --print`), plus the added `check`, usage-error help, hostname addresses and `-r`.
