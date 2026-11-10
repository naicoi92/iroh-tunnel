# Clarify workspace-boundaries specs Implementation Plan

> Executed via `/opsx-apply`, which chooses the executor.
> Spec reference: this change's `specs/` and `design.md` (with `brainstorm.md` and `proposal.md`).

**Goal:** Make `workspace-boundaries` and `frontend-independent-management` checkable against the current tree, give all 14 placeholder specs a real Purpose, and enforce the engine dependency rule with a test.

**Architecture:** The delta specs carry the requirement changes and are applied at archive. This plan adds one layout test, edits 14 `## Purpose` sections in the main specs directly (design D1), trims one `CONTRIBUTING.md` paragraph, and runs E2E checks of the restated exit-code scenarios. No production code changes.

**Tech Stack:** Rust 1.91 workspace (`toml` dev-dependency of the root package), OpenSpec 1.16 CLI.

## Global Constraints

- MSRV and toolchain for every gate: `cargo +1.91`, always with `--locked`.
- No product behaviour change and no new dependency: `toml` and `serde_json` are already root dev-dependencies.
- OpenSpec prose in English; requirement and scenario names in the deltas are not edited during apply.
- Purpose text claims nothing beyond its spec's own requirements (design D1), one to three sentences.
- Do not edit files under `openspec/changes/archive/`.

## Review Focus

1. A target-specific engine table (`[target.'cfg(windows)'.dependencies]`) holding a directly versioned entry must fail the test. Pinned by Task 1 Step 1, case `target_tables_are_checked`.
2. An entry written as `foo = { workspace = true, features = [...] }` is valid and must pass, because features are allowed with inheritance. Pinned by Task 1 Step 1, case `workspace_entry_with_features_passes`.
3. An entry with `workspace = false`, or an inline table with only `version`, must fail; checking for the key alone is not enough. Pinned by Task 1 Step 1, case `workspace_false_is_rejected`.
4. A Purpose that leaks into the `## Requirements` heading or drops the blank line breaks parsing. Pinned by Task 2 Step 3, `openspec validate --specs` with no errors.
5. A removed CONTRIBUTING heading leaves a dangling intra-repo link. Pinned by Task 3 Step 2 grep.

---

## Contracts

| Name | new / modified | Shape | public |
|---|---|---|---|
| `fn direct_engine_dependencies(manifest: &toml::Table) -> Vec<String>` in `tests/workspace_layout.rs` | new | Returns `"<table path>.<name>"` for every entry in `dependencies`, `dev-dependencies` and `build-dependencies`, top level and under each `target.<cfg>`, that is not a table with `workspace = true`. Table path examples: `dependencies`, `target.'cfg(unix)'.dev-dependencies`. | no (test helper) |
| `CONTRIBUTING.md` heading "Workspace layout and Rust API migration" | modified | Renamed to "Workspace layout"; the migration-map paragraph is removed. | no |

---

## Task 1: Engine dependency rule test — implements 1

**Covers:** 1.1, 1.2, 1.3
**Difficulty:** Easy
**Blocked by:** none

**Files:**
- Modify: `tests/workspace_layout.rs`

**Interfaces:**
- Produces: `direct_engine_dependencies(manifest: &toml::Table) -> Vec<String>` (see Contracts).

- [ ] **Step 1: Write the failing tests** in `tests/workspace_layout.rs`:
  - `engine_dependencies_are_inherited_from_the_workspace`: read `crates/engine/Cargo.toml` (via `read_source`), parse with `toml::from_str::<toml::Table>`, assert `direct_engine_dependencies(&manifest).is_empty()`, with the message listing the offenders.
  - `direct_engine_dependencies_reports_every_offender`: parse an inline manifest that declares:
    - under `[dependencies]`: `foo = "1"`, `off = { workspace = false }` and `ok = { workspace = true, features = ["x"] }`;
    - under `[dev-dependencies]`: `bar = { path = "../bar" }`;
    - under `[target.'cfg(unix)'.dependencies]`: `baz = { version = "1" }`.

    Sort the result, then assert it equals:

    ```rust
    ["dependencies.foo", "dependencies.off", "dev-dependencies.bar", "target.'cfg(unix)'.dependencies.baz"]
    ```

    The order is not part of the contract; the test pins only the set.
  - Name the three Review Focus cases with comments inside this test: `target_tables_are_checked`, `workspace_entry_with_features_passes` (`ok` is absent from the output) and `workspace_false_is_rejected`.

- [ ] **Step 2: Run them to verify they fail**
  Run: `cargo +1.91 test --locked --all-features --test workspace_layout engine_dependencies`
  Expected: compile error, `cannot find function direct_engine_dependencies`.

- [ ] **Step 3: Implement `fn direct_engine_dependencies(manifest: &toml::Table) -> Vec<String>`** in `tests/workspace_layout.rs`. One inner `fn` collects the offenders of a single dependency table under a prefix. An entry is valid only when it is a `toml::Value::Table` whose `workspace` value is `Value::Boolean(true)`. Visit the three top-level kinds, then each `target.*` table's three kinds. Only the inheritance marker is checked: Cargo itself rejects an entry that combines `workspace = true` with `version`, `path` or `git`, so the helper does not repeat that check. Add a doc comment citing `workspace-boundaries` "Engine dependencies are inherited from the workspace".

- [ ] **Step 4: Run them to verify they pass**
  Run: the command from Step 2.
  Expected: `2 passed` (both test names contain `engine_dependencies`).

- [ ] **Step 5: Run the clippy and fmt gates**
  Run: `cargo +1.91 fmt --all -- --check && cargo +1.91 clippy --workspace --all-targets --all-features --locked -- -D warnings`
  Expected: exit 0.

- [ ] **Step 6: Commit**
  ```bash
  git add tests/workspace_layout.rs
  git commit -m "test(workspace): require engine dependencies to inherit from the workspace"
  ```

---

## Task 2: Purpose for the 14 placeholder specs — implements 2

**Covers:** 2.1, 2.2, 2.3, 2.4
**Difficulty:** Normal
**Blocked by:** none

**Files:**
- Modify: `openspec/specs/{cli-entrypoints,config-management-commands,connection-path-status,frontend-independent-management,install-release-packaging,iroh-endpoint-relay,role-ownership-shutdown,self-update,system-service-management,tcp-tunnel-forwarding,tunnel-configuration,udp-datagram-forwarding,windows-service-trust,workspace-boundaries}/spec.md` (the `## Purpose` body only)

**Interfaces:** none.

- [ ] **Step 1: Confirm the baseline**
  Run: `openspec validate --specs 2>&1 | grep -c "Purpose section is still a placeholder"`
  Expected: `14`.

- [ ] **Step 2: Write each Purpose.** For each spec, read its `### Requirement:` headers, then replace the `TBD` line with one to three sentences that state:
  - what the capability governs, in the terms of `docs/GLOSSARY.md`;
  - for neighbouring capabilities that could be confused with it, which one owns the adjacent concern. For example, `cli-entrypoints` owns the command surface and exit codes, while `config-management-commands` owns `config` subcommand behaviour.

  Keep `## Purpose`, one blank line, the text, one blank line, then `## Requirements`. Commit in three batches, matching tasks.md 2.1–2.3:
  ```bash
  git add openspec/specs/<the batch's four or five specs>
  git commit -m "docs(openspec): write Purpose for <batch capability names>"
  ```

- [ ] **Step 3: Validate**
  Run: `openspec validate --specs 2>&1 | grep -c "Purpose section is still a placeholder"; openspec validate --specs 2>&1 | tail -1`
  Expected: `0`, and `Totals: 16 passed, 0 failed (16 items)`.

---

## Task 3: CONTRIBUTING migration-map link — implements 3

**Covers:** 3.1, 3.2
**Difficulty:** Easy
**Blocked by:** none

**Files:**
- Modify: `CONTRIBUTING.md` (section starting at the heading "Workspace layout and Rust API migration", currently line 188)

**Interfaces:** see Contracts, `CONTRIBUTING.md` heading.

- [ ] **Step 1: Edit.** Rename the heading to `## Workspace layout`, keep the intro sentence and the three package bullets, and delete the paragraph that begins "Old `iroh_tunnel::<module>::<item>` paths".
- [ ] **Step 2: Check for dangling links**
  Run: `grep -rnE '\]\([^)]*#workspace-layout-and-rust-api-migration\)' --include='*.md' . | grep -v 'openspec/changes/archive/'; grep -n "migration-map.md" README.md CONTRIBUTING.md docs/*.md`
  Expected: no output from either command.
- [ ] **Step 3: Commit**
  ```bash
  git add CONTRIBUTING.md
  git commit -m "docs(contributing): drop the archived migration-map link"
  ```

---

## Task 4: Change-level verification — implements 4

**Covers:** 4.1, 4.2, 4.3, 4.4
**Difficulty:** Easy
**Blocked by:** Task 1, Task 2, Task 3

**Files:**
- No source files. The evidence is recorded in `openspec/changes/clarify-workspace-boundaries-specs/apply.md` by the apply flow.

**Interfaces:** none.

- [ ] **Step 1: OpenSpec**
  Run: `openspec validate clarify-workspace-boundaries-specs --strict && openspec validate --specs`
  Expected: `is valid`; the second command shows no Purpose warning and 0 failed.
- [ ] **Step 2: Workspace gates**
  Run: `cargo +1.91 fmt --all -- --check && cargo +1.91 clippy --workspace --all-targets --all-features --locked -- -D warnings && cargo +1.91 test --workspace --all-features --locked && RUSTDOCFLAGS="-D warnings" cargo +1.91 doc --workspace --no-deps --locked`
  Expected: exit 0. On macOS the test count is at least the current main count plus 2.
- [ ] **Step 3: E2E exit-code checks with the real binary**
  ```bash
  cargo +1.91 build --locked --bin iroh-tunnel
  T=$(mktemp -d)
  printf '[[services]]\nname = "db"\nprotocol = "tcp"\nport = 0\n' > "$T/zero.toml"
  ./target/debug/iroh-tunnel serve config check --config "$T/zero.toml"; echo "exit=$?"
  printf 'not = [valid' > "$T/bad.toml"
  ./target/debug/iroh-tunnel serve config list --config "$T/bad.toml"; echo "exit=$?"
  ```
  Then check that `secret_key` is validated at key resolution, not at parsing. `run` stops at the key, so no role starts:

  ```bash
  printf '[node]\nsecret_key = "%%%%%%"\n\n[[services]]\nname = "db"\nprotocol = "tcp"\nport = 5432\n' > "$T/key.toml"
  IROH_TUNNEL_STATE_DIR="$T/state" ./target/debug/iroh-tunnel serve run --config "$T/key.toml"; echo "exit=$?"
  ```

  Expected:
  - first command: stderr contains `$T/zero.toml` and `invalid port`, then `exit=2`;
  - second command: stderr names `$T/bad.toml`, then `exit=2`;
  - third command: stderr contains `invalid secret_key` and does not contain `failed to parse config`. The exit code is not asserted; see design D6.

  Record all three outputs.
- [ ] **Step 4: Follow-up issue (needs user approval in chat).** Draft the issue for design D6, quoting the reproduction: `serve run --config` with `port = 0` or malformed TOML exits 1 instead of 2, and the validation message lacks the path. Open it with `gh issue create -R naicoi92/iroh-tunnel` only after the user approves; otherwise leave the draft in `apply.md`.

---

## Scenario → test

workspace-boundaries (ADDED):
- Engine manifest inherits every dependency → `tests/workspace_layout.rs::engine_dependencies_are_inherited_from_the_workspace` (new, Task 1)
- Directly versioned engine dependency is rejected → `tests/workspace_layout.rs::direct_engine_dependencies_reports_every_offender` (new, Task 1)
- Unknown fields are ignored → `crates/host/src/config/dto.rs::unknown_fields_and_legacy_multiplex_still_load`
- Legacy multiplex keys load → `crates/host/src/config/dto.rs::unknown_fields_and_legacy_multiplex_still_load`; `tests/config_multiplex_compat.rs::config_commands_drop_legacy_multiplex_without_changing_other_settings`
- Hostname target is kept → `crates/host/src/config/dto.rs::hostname_target_still_loads`
- Schema validation fails at load → `crates/host/src/config/dto.rs::validation_rejects_zero_port` (stage); Task 4 Step 3 E2E (exit 2 and path)
- Secret key is checked at key resolution → `crates/host/src/config/resolve_secret_key.rs::{resolve_invalid_base64_errors, resolve_wrong_length_errors}` (rejection at resolution); Task 4 Step 3 E2E case 3 (a malformed key passes parsing and fails with `invalid secret_key`, not `failed to parse config`). No existing unit test parses a malformed key; the E2E step is the evidence for the "parsing succeeds" half.

workspace-boundaries (MODIFIED):
- Two runs in one process do not share a deadline → `crates/engine/src/runtime/run_context.rs::two_runs_do_not_share_a_deadline`
- Forced exit codes are unchanged → `tests/configured_multiplex.rs::{repeated_stop_after_grace_exits_130_and_cleans_status, stuck_shutdown_is_abandoned_by_watchdog_with_exit_1}`; `src/lifecycle.rs::exit_codes_are_130_for_signal_and_1_for_watchdog`
- Gates fail on a member-only defect → CI `lint` and `unit` jobs run `--workspace` commands; Task 4 Step 2 runs the same commands
- Doc gate fails on a member-only broken link → `#![deny(rustdoc::broken_intra_doc_links, rustdoc::private_intra_doc_links)]` in `crates/{host,engine}/src/lib.rs`; Task 4 Step 2 doc gate
- Public DTO API has no file methods → compile-level: no root test or caller uses such methods; checked by review of `crates/host/src/config/dto.rs` (unchanged by this change)
- No file methods remain on the documents internally → same as above (unchanged code)
- Run still persists a generated key → `crates/host/src/config/store.rs::resolve_and_persist_key_generates_and_persists`; `tests/config_lifecycle.rs::run_persists_a_generated_key_owner_only_and_reuses_it`
- Run adopts a concurrently persisted key → `crates/host/src/config/store.rs::resolve_and_persist_key_adopts_concurrently_persisted_key`
- Load diagnostics are unchanged → `crates/host/src/config/store.rs::{load_role_config_missing_file_names_the_path, parse_role_config_reports_path_and_line_without_secret}`; `crates/host/src/config/dto.rs::{serve_load_missing_file_errors, serve_load_bad_toml_errors}`
- Legacy fields still ignored and dropped on serialize → `crates/host/src/config/dto.rs::legacy_multiplex_values_are_ignored_and_dropped_on_serialize`
- Clean doc build → Task 4 Step 2 doc gate; CI `lint` doc step
- Broken link is rejected → the deny attributes above (unchanged by this change)
- No task survives the role → `crates/engine/src/serve/run_serve.rs::{quarantine_sinks_are_joined_on_role_stop, run_completion_reports_unquiesced_writer}`
- Quarantine sink outlives its connection → `crates/engine/src/serve/run_serve.rs::quarantine_sink_outlives_its_connection`
- Quarantine sinks stop with the role → `crates/engine/src/serve/run_serve.rs::quarantine_sinks_are_joined_on_role_stop`
- Slot released on sink failure → `crates/engine/src/serve/run_serve.rs::failed_quarantine_sink_releases_its_slot`; `tests/udp_tunnel.rs::quarantine_holds_the_slot_and_then_releases_it`
- Writer joins in time → `crates/engine/src/serve/run_serve.rs::run_completion_reports_unquiesced_writer`; `tests/serve_status_conn_path.rs::serve_status_file_reports_connected_peer_paths`
- Writer does not quiesce → `crates/host/src/runtime/skeleton.rs::status_file_kept_when_flush_join_times_out`
- Initial snapshot precedes admission → `crates/engine/src/serve/run_serve.rs::initial_status_saved_before_accept_loop`
- Unchanged observation is not rewritten → `crates/engine/src/serve/run_serve.rs::unchanged_status_is_not_rewritten`; `crates/host/src/status/schema.rs::observation_equality_ignores_nothing_rendered`
- Failed write is retried → `crates/engine/src/serve/run_serve.rs::failed_status_write_is_retried_next_tick`
- Debug of a loaded configuration → `crates/host/src/config/dto.rs::config_debug_redacts_secret_key`
- Error context does not echo secrets → `crates/host/src/config/dto.rs::error_chain_never_contains_secret_line`; `tests/config_lifecycle.rs::parse_errors_never_echo_secret_lines`
- Permission failure keeps exit code 3 → `tests/service_user.rs::system_install_without_privilege_stops_with_rerun_guidance`
- Ownership conflict keeps exit code 6 → `tests/role_ownership.rs::second_serve_is_refused_across_binaries_overrides_and_start_order`
- Invalid configuration keeps exit code 2 → `tests/config_paths.rs::invalid_config_reports_path_with_config_error_exit`; Task 4 Step 3 E2E
- Binary targets after the move → `tests/layout_cli.rs::{root_package_defines_three_binaries, all_packages_share_one_version}`
- Build workflow reads the workspace version → no automated test; checked by review of `.github/workflows/release.yml` (unchanged by this change)
- Docker build includes member packages → no automated test; `Dockerfile` copies the whole context (`COPY . .`) and installs `/usr/local/bin/iroh-tunnel` (unchanged by this change)
- Installer URLs unchanged → no automated test; `install.sh` and `install.ps1` are at the repository root (unchanged by this change)
- Former test bypass is not a production API → `tests/workspace_layout.rs::ownership_bypass_is_test_utils_gated`

frontend-independent-management (MODIFIED):
- Rotation without consent → `crates/host/src/config/keygen.rs::keygen_without_consent_returns_consent_needed`
- Rotation with consent → `crates/host/src/config/keygen.rs::keygen_with_consent_rotates_and_returns_new_node_id`
- First generation needs no consent → `crates/host/src/config/keygen.rs::first_keygen_needs_no_consent`; `tests/config_lifecycle.rs::first_keygen_does_not_prompt`
- CLI confirmation behaviour unchanged → `tests/config_lifecycle.rs::keygen_rotation_needs_confirmation`
- Install without scope from an unprivileged caller → `crates/host/src/service/install.rs::install_without_scope_returns_scope_choice_need`
- CLI keeps its scope prompt → `tests/service_user.rs::interactive_install_offers_user_or_system_and_system_is_refused_unprivileged`
- Management application registers the tunnel binary → `crates/host/src/service/install.rs::install_renders_supplied_tunnel_executable`
- CLI resolution unchanged → `src/cli_service.rs::cli_resolves_tunnel_binary_from_path_first`
- Ordering on a terminal → `crates/host/src/service/backend/{systemd,launchd}.rs::notes_reach_observer_before_native_command`
- Unprivileged system scope → `tests/service_user.rs::system_scope_operations_without_privilege_are_refused_with_guidance`
- Non-elevated Windows system scope → `tests/service_user_windows.rs::non_elevated_system_scope_is_permission_error` (Windows CI)
- BusyBox user scope unchanged → `crates/host/src/service/backend/busybox.rs::busybox_user_scope_request_emits_system_wide_note`
- Owner metadata unavailable → `crates/host/src/service/status.rs::status_reports_missing_owner_as_unavailable`
- Raw JSON passthrough → `tests/cli_golden.rs::status_json_forwards_file_bytes`
- Package-manager install is delegated → `crates/host/src/update/check_owner.rs::package_owned_target_returns_delegation`; `tests/update_ownership.rs::package_owned_file_is_not_replaced_by_default`
