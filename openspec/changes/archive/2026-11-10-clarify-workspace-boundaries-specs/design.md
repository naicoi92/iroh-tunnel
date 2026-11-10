## Context

`brainstorm.md` fixes the scope and the eight decisions. This file covers only how they are carried out: where Purpose text lives, how the engine dependency rule is checked, and how requirements with no baseline are restated.

Current state:
- `crates/engine/Cargo.toml` declares every entry in `[dependencies]` and `[dev-dependencies]` as `<name>.workspace = true`, with no build dependencies and no target-specific tables.
- `tests/workspace_layout.rs` already checks the workspace shape through `cargo metadata`. The root package has `toml` as a dev-dependency.
- OpenSpec merges `## Purpose` from a delta only when it creates a capability. For an existing capability, Purpose has to be edited in `openspec/specs/<capability>/spec.md` directly.

## Goals / Non-Goals

**Goals:** carry out brainstorm decisions 1–8 with the smallest set of edits that `openspec validate` and the test suite can check.

**Non-Goals:** product code changes, dependency migration for host or root, and reformatting requirements that this change does not touch.

## Decisions

### D1. Purpose edits go into the main specs inside the same change branch

The 14 Purpose sections are written directly in `openspec/specs/<capability>/spec.md` as part of the implementation commits, not in deltas. Each Purpose is one to three sentences: what the capability covers and where its boundary is with neighbouring capabilities. It is derived from that spec's own requirements and claims nothing they do not say.
- Alternative: a `## Purpose` in each delta. Rejected: archive ignores it for existing capabilities, so the warning would persist.
- Consequence: archiving this change must keep these main-spec edits. Archive merges the deltas on top of them, and the deltas never touch `## Purpose`.

### D2. The engine dependency rule is checked by parsing the manifest, not `cargo metadata`

`cargo metadata` resolves the inherited version and loses whether an entry was declared with `workspace = true`. The test therefore parses `crates/engine/Cargo.toml` with `toml` and checks every table that declares dependencies:
- top level: `dependencies`, `dev-dependencies`, `build-dependencies`;
- under each `target.<cfg>`: the same three tables.

Every entry in those tables must be a table with `workspace = true`. An entry that is not, including a bare version string or a `path`, fails the test, and the failure message names it.
- Alternative: compare engine dependency names with the keys of `[workspace.dependencies]`. Rejected: an engine entry could repeat a name with its own version and still pass.

### D3. A requirement with no baseline is restated as an absolute property or a cross-reference

A scenario whose THEN says "as before the refactor" is rewritten to state the observable result directly, using the existing CLI requirement as the source of the expected value. Each `frontend-independent-management` scenario keeps its WHEN and states the result:

| Scenario | Restated THEN |
|---|---|
| keygen rotation on a non-TTY without `--yes` | fails, the error mentions `--yes`, the stored key is unchanged (per `config-management-commands` "Key rotation needs confirmation") |
| scope prompt | the prompt offering `user` and `system` is written to stderr (per `system-service-management`) |
| `PATH` resolution | the registration's command line references the `iroh-tunnel` binary resolved from `PATH` |
| `service status` ordering | each CLI note is written before the native output of the command it introduces |

In `workspace-boundaries`:
- "Run-scoped drain context" keeps run isolation, drops the `DRAIN_DEADLINE` history and references `role-ownership-shutdown` for exit codes 130 and 1.
- In "Configuration documents carry no filesystem behaviour", "as before" is dropped wherever the sentence already states the result.
- "Validated types" lists the concrete accepted inputs and keeps the rule that validation failures happen at config load.

In `workspace-boundaries`, baseline words such as "existing role stop deadline", "existing five-second bound", "previous rendered-value change detection", "same target paths" and "widened `pub` item" are replaced by:
- the concrete value: 30-second Quarantine, five-second flush join, `/usr/local/bin/iroh-tunnel`, one trailing newline after raw status JSON;
- or the owning requirement: `role-ownership-shutdown`, `connection-path-status`, `cli-entrypoints`, `install-release-packaging`, `self-update`.

Two edits restate rules more precisely than their old text, without changing behaviour:
- Runtime-task joins: channel, connection and Quarantine tasks are bound by the drain deadline, while the status flush task keeps its own five-second join, which matches `run_serve.rs`.
- Public API: every `pub` item has rustdoc, which `#![deny(missing_docs)]` already enforces. A change that adds or widens a `pub` item names a caller in another workspace package.

### D4. Rules whose meaning changes are replaced; rewordings keep their names

OpenSpec archive applies a MODIFIED block by full replacement and refuses one that drops or renames a current scenario.
- The two rules whose meaning changes (the dependency rule and the validated-types rule) are therefore REMOVED and ADDED under new names. Each REMOVED entry names its replacement.
- Requirements that are only reworded stay MODIFIED. Their scenario names are kept even when a name still reads "… unchanged" (for example "Forced exit codes are unchanged"), and only the WHEN/THEN text is rewritten.
- Alternative: REMOVED plus ADDED for every reworded requirement to get clean scenario names. Rejected: it multiplies churn in the delta for a cosmetic gain.

### D5. Only the dependency test is new; no other requirement gains tests

Removed requirements lose nothing that is currently tested: their tests were removed in `7651f43`. The restated scenarios describe behaviour already covered by the capability tests they reference.

### D6. Exit-code scenarios use commands verified to exit 2

While drafting, `iroh-tunnel serve run --config <file>` exited with code 1 for both a malformed TOML file and a file with `port = 0`, and the validation error did not name the path. The same files give code 2 with the path under `serve config check` and `serve config list`. `run` therefore disagrees with `cli-entrypoints` "Exit status mapping" and `tunnel-configuration` "Malformed TOML".
- Fixing `run` changes product behaviour, which this change excludes, so the restated scenarios use `config check` and `config list`.
- The `run` exit-code defect is reported for a separate issue; it is not fixed here.

## Risks / Trade-offs

- [A Purpose claims more than the requirements say] → Write each Purpose from its spec's own requirement list. The final review packet includes every changed main spec.
- [MODIFIED blocks drift from main-spec text during the apply] → Each MODIFIED block copies the full current requirement and edits only the agreed text. `openspec validate` on the change, and an archive dry run if one is available, catch header mismatches.
- [The test is too strict for a future legitimate engine dependency] → Intended: the dependency is added to `workspace.dependencies` first, which is the rule.
