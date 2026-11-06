## Why

The `config` commands are awkward to use. Every value is a long flag (`config add --name web --protocol tcp --port 3000`), and an entry cannot be changed without removing and re-adding it. Requirements and defaults also disagree with quick mode: `--protocol` is mandatory here but defaults to `tcp` there, and access `--node-id` is optional at parse time but required later. Help has blank flag descriptions and no examples, and a wrong command prints a one-line usage or a bare `config error:` with no help. Aligning the surface with well-known CLIs (kubectl, tailscale, ngrok, git) and showing the failing command's help on every usage error makes the CLI usable without memorising it.

## What Changes

**Service entry commands (BREAKING)**
- From: `config add --name N --protocol P [--host H] --port N [--node-id ID]`; `config remove --name N`.
- To: `config set -n NAME` as an upsert. Serve takes `-t/--target [HOST:]PORT`; access takes `-l/--listen [HOST:]PORT` and `-N/--node-id`; both take `-p/--protocol tcp|udp`. Removal is `config rm -n NAME`.
- Creating an entry requires the address, plus `-N` for access, with defaults protocol `tcp` and host `127.0.0.1`. Updating an entry changes only the given attributes and reports each change as `old -> new`.
- Reason: in-place edits, and exactly the same arguments as quick mode, so that a quick invocation becomes a stored entry by inserting `config set`.
- Impact: breaking. `add`, `remove` and their `--name`/`--host`/`--port` flags are removed with no aliases. Scripts and docs must move to the new syntax.

**Print-only key generation (BREAKING)**
- From: `config keygen random` (a subcommand).
- To: `config keygen --print`, which conflicts with `-c` and `-y`.
- Impact: breaking for callers of `keygen random`.

**New `config check`**
- Validates the config file against the role schema without writing anything.

**Usage errors show help**
- From: clap errors print one usage line; post-parse validation errors (missing quick flags, missing access node id) print only `config error: ...`.
- To: every usage error prints `error: ...` followed by the full help of the failing subcommand, exit 2. A bare role or `config` prints help. Runtime and config-file errors stay error-only with unchanged exit codes.

**Help content and short options**
- Every command and flag has a description, its default and its possible values. `set`, `rm`, `keygen` and quick mode have an Examples section.
- One argument vocabulary: wherever a value is accepted, it uses the same long name, short letter, value syntax and help text (`-c -n -t -l -N -p -r -k -y`). `--reveal`, `--print`, `--system` and `--user` stay long-only.

**Quick mode alignment**
- `-t`/`-l` accept `PORT` (meaning `127.0.0.1:PORT`) or `HOST:PORT`, with HOST being an IP address or a hostname, exactly as in `config set`.
- `-p` accepts only `tcp|udp`, with possible values shown in help.
- `--relay-url` gains `-r`.
- Impact: additive. Existing `IP:PORT` invocations keep working, and hostnames become accepted.

**Non-goals**
- Setting `[node]` values from the CLI.
- Changes to the config file format.
- Changes to `run`, `status`, `service` or `update` beyond the shared error/help behaviour.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `config-management-commands`: replaces the add/remove requirements with `set` (create, update, change report) and `rm`; adds `check`; renames `keygen random` to `keygen --print`. File-target, missing-config, validation, persistence and redaction requirements are kept.
- `cli-entrypoints`: adds usage-error-with-help and help-on-bare-invocation requirements, help content (descriptions, defaults, possible values, examples) and the shared short-option table. Quick-mode address and protocol parsing change to `[HOST:]PORT` and `tcp|udp` value validation.
- `tunnel-configuration`: the random-key requirement moves from `keygen random` to `keygen --print`; the legacy-migration failure requirement names `config set` instead of `config add`.
- `frontend-independent-management`: host configuration operations become set (with `Created` / `Updated` / `Unchanged` outcomes and a typed missing-for-create error), remove and check, replacing add.
- `workspace-boundaries`: the mutation-ownership scenario names `config set` / `config rm` instead of `config add` / `config remove`.

## Impact

- **Code:**
  - `src/cli.rs`: argument model, help text and short options.
  - `src/app.rs`: parse entry, usage-error rendering and dispatch.
  - `src/config_cmd.rs` and `src/render.rs`: set/rm/check output and the change report.
  - `crates/host/src/config/`: `add_service`/`remove_service` become an upsert with a change report, plus a validate-only check.
- **Tests:** CLI golden tests, `config_lifecycle*`, `config_paths`, `config_multiplex_compat`, `configured_multiplex` and `quick_run` move to the new syntax, with new cases for upsert, check, usage-error help and address parsing.
- **Docs:** README, `docs/cli.md`, `docs/configuration.md`, `docs/services.md`, `docs/deployment.md`, `docs/troubleshooting.md`, plus a CHANGELOG entry marked BREAKING with old-to-new examples.
- **Dependencies:** none added. clap features already in use cover `ValueEnum` and help customisation; any feature flag change goes in design.
- **Compatibility:** breaking for the `config add/remove` and `keygen random` syntax only. Config files, exit-code classes, service units and quick-mode `IP:PORT` invocations are unaffected.
- **Security:** no change to secret handling. `show` stays redacted by default, `keygen --print` writes the secret to stdout only, and diagnostics and the `set` change report never echo `secret_key` or `relay_token`. The report covers only service fields, which hold no secrets.
