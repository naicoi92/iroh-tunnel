# Config DTO and Rustdoc Cleanup Implementation Plan

> Executed via `/opsx-apply`, which chooses the executor.
> Spec reference: this change's `specs/` and `design.md`.

**Goal:** Remove filesystem methods from the host config DTOs, make `cargo doc` warning-free and enforced, and document the `limit_shrink` deviation, with no observable behaviour change (issue #167).

**Architecture:** The `RoleDoc` default bodies move verbatim into three `pub(crate)` free functions in `crates/host/src/config/store.rs`; `RoleDoc` keeps `validate` and `node_mut`. Host callers switch to the store functions, and the root tests stop relying on the removed API. Rustdoc links are fixed under crate-level deny lints, and CI `lint` builds docs with warnings denied.

**Tech Stack:** Rust 1.91 (MSRV), serde/toml/toml_edit, anyhow/`CliError`, tracing, GitHub Actions.

## Global Constraints

- MSRV Rust 1.91; validate with `cargo +1.91`. No new dependencies.
- Error text unchanged: `failed to read config: <path>`; `failed to parse config: <path>{ (line N)}: <message>`; the warn log `secret_key empty, generated new key, saving to config`.
- Secret hygiene (RS-10): no diagnostic or `Debug` output may contain key material.
- Persistence contract unchanged: `persist_generated_key` locking, atomic owner-only write, concurrent-key adoption and retired-field stripping. Its body is not edited.
- New store functions are `pub(crate)`, never `pub`.
- Gates (all on `+1.91`):
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
  - `cargo test --workspace --all-features --locked`
  - `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked`
  - default-feature and release builds `cargo build --workspace --locked` and `cargo build --workspace --release --locked` (no `--all-features`).

## Review Focus

- **Keyless config, another process stores a key first.** Expected: adopt it, do not overwrite. Pinned by `resolve_and_persist_key_adopts_concurrently_persisted_key` (Task 1).
- **Syntax error on the `secret_key` line.** Expected: line number only, never the secret. Pinned by the existing `error_chain_never_contains_secret_line`, switched to `load_role_config` (Task 1).
- **System-service trust path (unix `trust.rs`, Windows `platform.rs`).** Expected: still parses from the verified handle, not by path. Pinned by the existing trust tests plus a compile-level check that `windows_load_config` uses `parse_role_config`; the Windows build is verified by CI `unit (windows)` (Task 1).
- **`service install` with an invalid config.** Expected: still refused with the same config error. Pinned by the existing `install.rs` tests (Task 1).
- **A link fixed with a path that only resolves under `cfg(test)` or `test-utils`.** Expected: the default-feature doc build is still clean. Pinned by running `cargo doc` without features (Task 2).

---

## Contracts

| Name | new / modified | Shape | public |
|---|---|---|---|
| `config::store::load_role_config` | new | `pub(crate) fn load_role_config<D: RoleDoc>(path: &Path) -> anyhow::Result<D>` | no |
| `config::store::parse_role_config` | new | `pub(crate) fn parse_role_config<D: RoleDoc>(content: &str, path: &Path) -> anyhow::Result<D>` | no |
| `config::store::resolve_and_persist_key` | new | `pub(crate) fn resolve_and_persist_key<D: RoleDoc>(cfg: &mut D, path: &Path) -> anyhow::Result<iroh::SecretKey>` | no |
| `config::RoleDoc` | modified | `pub(crate) trait RoleDoc: Serialize + DeserializeOwned { fn validate(&self) -> Result<()>; fn node_mut(&mut self) -> &mut NodeConfig; }` (default `load`/`from_toml`/`save`/`resolve_and_save_key` removed) | no |
| `ServeConfig::{load, save, resolve_and_save_key}` | modified (removed) | — | yes (removal from `iroh-tunnel-host`, `publish = false`) |
| `AccessConfig::{load, save, resolve_and_save_key}` | modified (removed) | — | yes (same) |
| CI `lint` job | modified | name `lint (fmt + clippy + doc)`; new step `cargo doc --workspace --no-deps --locked` with `RUSTDOCFLAGS: -D warnings` | n/a |

---

## Task 1: Config file effects move from DTOs to store functions — implements 1

**Covers:** 1.1, 1.2, 1.3, 1.4, 1.5, 1.6, 1.7, 1.8, 1.9
**Difficulty:** Normal
**Blocked by:** none

**Files:**
- Modify: `crates/host/src/config/store.rs`, `crates/host/src/config/dto.rs`, `crates/host/src/config/mod.rs`
- Modify: `crates/host/src/runtime/skeleton.rs`, `crates/host/src/runtime/strategy.rs`
- Modify: `crates/host/src/service/install.rs`, `crates/host/src/service/trust.rs`, `crates/host/src/service/platform.rs`
- Modify: `tests/config_multiplex_compat.rs`, `tests/configured_multiplex.rs`, `tests/config_lifecycle.rs`, `tests/relay_auth.rs` (comment only)

**Interfaces:**
- Produces: the three store functions from Contracts, re-exported as `pub(crate) use store::{load_role_config, parse_role_config, resolve_and_persist_key};` in `config/mod.rs`.

- [ ] **Step 1: Pre-change inventory.** Run:
  ```sh
  mkdir -p /tmp/cleanup-167 && set -o pipefail && cargo +1.91 test --workspace --all-features --locked -- --list --include-ignored 2>/tmp/cleanup-167/list-before.err | grep ': test$' | sort > /tmp/cleanup-167/tests-before.txt
  ```
  Expected: exit 0 and a non-empty file.
- [ ] **Step 2: Failing store tests** in a new `#[cfg(test)] mod tests` at the end of `store.rs` (it has none today), using `crate::config::{AccessConfig, ServeConfig, encode_secret_key, resolve_secret_key}` and `tempfile`:
  - `load_role_config_reads_and_validates`: write the `VALID_SERVE` fixture (`config::fixtures`), then `load_role_config::<ServeConfig>(&p)` is `Ok` and has 1 service.
  - `load_role_config_missing_file_names_the_path`: for `p = dir/missing.toml`, the error's `format!("{err}")` (top context) equals `format!("failed to read config: {}", p.display())`.
  - `parse_role_config_reports_path_and_line_without_secret`: text `"[node]\nsecret_key = \"SECRET-XYZ\" oops\n"` with path `serve.toml`. Assert full equality `format!("{err:#}") == format!("failed to parse config: serve.toml (line 2): {msg}")`, where `msg` is `toml::from_str::<toml::Value>(text).unwrap_err().message()` (the same parser, so the message is pinned to the source of truth without hardcoding toml's wording). Neither `{err:#}` nor `{err:?}` contains `SECRET-XYZ`.
  - `resolve_and_persist_key_generates_and_persists`: moved from `dto.rs::access_resolve_and_save_key_generates_and_persists`, same assertions.
  - `resolve_and_persist_key_no_save_when_present`: moved from `access_resolve_and_save_key_no_save_when_present`, same assertions.
  - `resolve_and_persist_key_adopts_concurrently_persisted_key`: write an empty `access.toml`, `let mut cfg = load_role_config::<AccessConfig>(&p)?`, then write the file with `[node]\nsecret_key = "<enc of key K>"`. Call `resolve_and_persist_key(&mut cfg, &p)` and assert: the returned key's `.public() == K.public()`, `cfg.node.secret_key == enc(K)`, and the file text is byte-identical to what was written.
  - `resolve_and_persist_key_drops_legacy_fields_for_both_roles`: the body of root `generated_identity_save_drops_legacy_fields_for_both_roles` via the store functions; assert no `multiplex` in the file and the persisted key equals `encode_secret_key(&returned)`.
- [ ] **Step 3: Confirm red.** Run `cargo test -p iroh-tunnel-host --all-features --locked --lib config::store`; expected: compile error, functions not found.
- [ ] **Step 4: Add the three functions** to `store.rs` with the Contracts signatures. Move the bodies of `RoleDoc::load`, `RoleDoc::from_toml` and `RoleDoc::resolve_and_save_key` verbatim: `load_role_config` calls `parse_role_config`, and `resolve_and_persist_key` calls `persist_generated_key::<D>`. Give each rustdoc with an `# Errors` section, and add the `mod.rs` re-export.
- [ ] **Step 5: Confirm green.** Run the Step 3 command; expected: PASS.
- [ ] **Step 6: Switch callers.**
  - `skeleton.rs`: `crate::config::resolve_and_persist_key(&mut cfg, &path)?`.
  - `trust.rs::load_config`: `C::load(path)` → `load_role_config::<C>(path)`, and `C::from_toml(&text, path)` → `parse_role_config::<C>(&text, path)`.
  - `platform.rs`: same in `load_config` (the non-unix/windows branch) and `windows_load_config`.
  - `install.rs`: `check_installable_config` uses `load_role_config::<ServeConfig>`/`<AccessConfig>`, and `check_installable_text` uses `parse_role_config` (drop its `use crate::config::RoleDoc`).
  - `strategy.rs`: update the doc to "validates via `RoleDoc`; files are handled by the config store".
- [ ] **Step 7: Shrink the DTOs.**
  - Delete the `impl ServeConfig { load, save, resolve_and_save_key }` and `impl AccessConfig { … }` blocks.
  - Delete the four default methods from `RoleDoc`, and rewrite the trait doc to describe only `validate`/`node_mut` (no `[RoleDoc::load]`-style links).
  - Remove now-unused imports (`Context`, `SecretKey`, `resolve_secret_key`, `encode_secret_key`) where they become unused.
  - Move the `AccessConfig` "pinned access identity" prose onto `resolve_and_persist_key`.
- [ ] **Step 8: Convert the `dto.rs` tests.**
  - Replace every `ServeConfig::load(x)`/`AccessConfig::load(x)` with `load_role_config::<ServeConfig>(x)`/`::<AccessConfig>(x)`.
  - Rename `save_roundtrip_writes_file` → `serialized_config_roundtrips`: `toml::to_string_pretty(&cfg)`, then `parse_role_config::<ServeConfig>(&text, Path::new("serve.toml"))`, then `assert_eq!` with `cfg`.
  - Delete the two moved `access_resolve_and_save_key_*` tests.
  - Add `legacy_multiplex_values_are_ignored_and_dropped_on_serialize`: for each legacy variant from root `legacy_multiplex_values_are_ignored_and_removed_on_save`, `parse_role_config::<AccessConfig>(base+legacy)` equals the base parse, `toml::to_string_pretty` has no `multiplex` key in `services[0]`, and the re-parse equals the base.
- [ ] **Step 9: Adapt root tests.**
  - `tests/config_multiplex_compat.rs`:
    - Delete `legacy_multiplex_values_are_ignored_and_removed_on_save` and `generated_identity_save_drops_legacy_fields_for_both_roles`.
    - In the kept tests `config_commands_drop_legacy_multiplex_without_changing_other_settings` (line ~121) and `keygen_drops_multiplex_from_inline_services` (line ~152), replace `AccessConfig::load(&path)?` with `toml::from_str::<AccessConfig>(&std::fs::read_to_string(&path)?)?`, keeping every assertion. This is a plain deserialize, no validation, which those assertions do not need.
    - Keep the `AccessConfig` import, drop `ServeConfig`, and change the module doc to "Exercise real commands; serialization rules are host unit tests".
  - `tests/configured_multiplex.rs`:
    - At lines ~80 and ~312: read with `toml::from_str::<toml::Value>(&std::fs::read_to_string(p)?)?` and access `["services"][0]["host"|"port"|"name"]` and `["node"]["secret_key"]`.
    - At lines ~242 and ~812: edit with `toml_edit::DocumentMut` (set `services[0].port`, and at ~812 `node.secret_key`) and `std::fs::write`.
    - Keep every assertion's meaning.
- [ ] **Step 9b: Stale prose.** Update the comments that name removed methods: `crates/host/src/service/platform.rs:35` (`resolve_and_save_key` → `resolve_and_persist_key`), `tests/relay_auth.rs:164`, and the `dto.rs:1` module header ("TOML config schema … and validation"; drop "load/save").
- [ ] **Step 10: Grep check.** `rg -n "resolve_and_save_key|(Serve|Access)Config::(load|from_toml)|RoleDoc::(load|save)" crates src tests` returns nothing.
- [ ] **Step 11: Strengthen the generated-key E2E.** In `tests/config_lifecycle.rs::run_persists_a_generated_key_owner_only_and_reuses_it`, after `run_once(1)` assert that the persisted secret's public key equals the printed NodeId: `assert_eq!(iroh_tunnel_host::config::resolve_secret_key(&first_key).unwrap().0.public().to_string(), first_id)` (the test returns `()`). `resolve_secret_key` is already `pub`.
- [ ] **Step 12: Run gates.** On `+1.91` run fmt, clippy, test and the default-feature and release builds from Global Constraints; expected: all pass. The rustdoc gate is not required here, because Task 2 fixes the existing links. Then run the E2E selector `cargo +1.91 test --all-features --locked --test config_lifecycle run_persists_a_generated_key_owner_only_and_reuses_it`; expected: PASS. That test is `cfg(unix)`; Windows coverage is the CI `unit (windows)` compile plus the `store.rs` unit tests.
- [ ] **Step 13: Post-change inventory.** Write `/tmp/cleanup-167/tests-after.txt` with the Step 1 command (same toolchain, `pipefail`, stderr to `list-after.err`), run `comm -3` on before/after, and confirm every difference is one of the mapped renames:

  | Before | After |
  |---|---|
  | `config::dto::tests::access_resolve_and_save_key_generates_and_persists` | `config::store::tests::resolve_and_persist_key_generates_and_persists` |
  | `config::dto::tests::access_resolve_and_save_key_no_save_when_present` | `config::store::tests::resolve_and_persist_key_no_save_when_present` |
  | `config::dto::tests::save_roundtrip_writes_file` | `config::dto::tests::serialized_config_roundtrips` |
  | root `legacy_multiplex_values_are_ignored_and_removed_on_save` | `config::dto::tests::legacy_multiplex_values_are_ignored_and_dropped_on_serialize` |
  | root `generated_identity_save_drops_legacy_fields_for_both_roles` | `config::store::tests::resolve_and_persist_key_drops_legacy_fields_for_both_roles` |
  | — (new) | `config::store::tests::{load_role_config_reads_and_validates, load_role_config_missing_file_names_the_path, parse_role_config_reports_path_and_line_without_secret, resolve_and_persist_key_adopts_concurrently_persisted_key}` |

  Save the map for `apply.md`.
- [ ] **Step 14: Commit.** `git commit -m "refactor(host): move config DTO file effects into store functions (#167)"`

---

## Task 2: Rustdoc links, lints and limit_shrink docs — implements 2

**Covers:** 2.1, 2.2, 2.3, 2.4
**Difficulty:** Easy
**Blocked by:** Task 1

**Files:**
- Modify: `crates/host/src/lib.rs`, `crates/engine/src/lib.rs`
- Modify: `crates/host/src/config/require_present.rs`, `crates/host/src/config/types.rs`
- Modify: `crates/host/src/paths/default_config_path.rs`, `crates/host/src/paths/default_state_dir.rs`
- Modify: `crates/host/src/runtime/run_quick_access.rs`, `crates/host/src/runtime/run_quick_serve.rs`
- Modify: `crates/host/src/service/restart.rs`, `crates/host/src/service/start.rs`, `crates/host/src/service/stop.rs`, `crates/host/src/service/uninstall.rs`, `crates/host/src/service/types.rs`
- Modify: `crates/host/src/status/schema.rs`, `crates/host/src/update/types.rs`, `crates/host/src/update/update.rs`
- Modify: `crates/engine/src/observation/types.rs`
- Modify: `crates/host/src/config/dto.rs` only if a doc warning remains there

**Interfaces:**
- Consumes: Task 1's final `RoleDoc` doc (no links to removed methods).

- [ ] **Step 1: Add the deny lints.** Add `#![deny(rustdoc::broken_intra_doc_links, rustdoc::private_intra_doc_links)]` below `#![deny(missing_docs)]` in both `lib.rs`.
- [ ] **Step 2: Confirm red.** Run `cargo +1.91 doc --workspace --no-deps --locked`; expected: FAIL with the unresolved-link errors (18 before Task 1; the 2 `RoleDoc` ones are gone after Task 1).
- [ ] **Step 3: Fix the links.**

  | Location | Change |
  |---|---|
  | `CliError::Config`/`Conflict` | `[`CliError::Config`](crate::error::CliError::Config)` (likewise `Conflict`) |
  | `[`keygen`]` in `config/types.rs` | `crate::config::keygen` |
  | `run_quick_serve`/`run_foreground` | `super::run_quick_serve` / `super::run_foreground` |
  | `[`install`]` in `service/{restart,stop,uninstall}.rs` | `super::install` |
  | `scope::decide_install_scope` | `super::decide_install_scope` |
  | `[`check_owner`]` | `super::check_owner` |
  | `[`update_owner`]` in `update/types.rs:81` | `[`check_owner`](super::check_owner)` (the real owner decision) |
  | `[`StatusWriter`]` / `[`StatusWriter::save_to`]` in `status/schema.rs` | plain code spans `` `StatusWriter` `` (crate-private) |
- [ ] **Step 4: `limit_shrink` docs.** Append to the doc in `crates/engine/src/observation/types.rs:78` and to `crates/host/src/status/schema.rs:74`: "Until #152 is resolved, a connection whose datagrams are never usable also counts every datagram here; see `docs/udp-integration-contract.md`."
- [ ] **Step 5: Confirm green.** Run `RUSTDOCFLAGS="-D warnings" cargo +1.91 doc --workspace --no-deps --locked` and `RUSTDOCFLAGS="-D warnings" cargo +1.91 doc --workspace --no-deps --locked --all-features`; expected: both exit 0 with no `warning:` lines.
- [ ] **Step 6: Negative check.** Temporarily add `/// See [`nope`].` to a public host item and confirm the Step 5 command fails with `unresolved link to `nope``; revert, and check with `git diff --stat` that only the intended files changed.
- [ ] **Step 7: Re-run gates.** Run fmt and clippy from Global Constraints; expected: pass.
- [ ] **Step 8: Commit.** `git commit -m "docs(host,engine): fix intra-doc links and deny broken links (#167)"`

---

## Task 3: CI doc gate and contributor docs — implements 3

**Covers:** 3.1, 3.2, 3.3
**Difficulty:** Easy
**Blocked by:** Task 2

**Files:**
- Modify: `.github/workflows/ci.yml`, `CONTRIBUTING.md`, `docs/agents/rust-coding-standards.md`

**Interfaces:**
- Consumes: the warning-free tree from Task 2.

- [ ] **Step 1: Update the `lint` job** in `ci.yml`:
  - Rename it to `name: lint (fmt + clippy + doc)` and change the header comment line to `lint          (fmt + clippy + doc)`.
  - After the clippy step, add:
    ```yaml
          - run: cargo doc --workspace --no-deps --locked
            env:
              RUSTDOCFLAGS: -D warnings
    ```
  - `main` has no branch protection; `ci-ok` is the single required check and depends on job ids, not names.
- [ ] **Step 2: Update the local gate lists.**
  - `CONTRIBUTING.md` "Code quality gates": add `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked`.
  - `docs/agents/rust-coding-standards.md` RS-16 code block: add the same command, and replace "No new lint gate is implied." with "The rustdoc gate denies broken and private intra-doc links."
- [ ] **Step 3: Lint the workflow.** Run `actionlint .github/workflows/ci.yml`. Expected: no output, exit 0. Confirm `ci-ok`'s `needs` still includes `lint`.
- [ ] **Step 4: Run the CI command locally.** Run the exact CI doc command on `+1.91`; expected: exit 0.
- [ ] **Step 5: Commit.** `git commit -m "ci: deny rustdoc warnings in the lint job (#167)"`

---

## Scenario → test

**workspace-boundaries**
- Public DTO API has no file methods → Task 1 Step 10 grep plus a compile check: the root tests no longer compile against the removed methods; `cargo doc` of the host public API shows no such method.
- No file methods remain on the documents internally → Task 1 Step 10 grep plus review of `dto.rs`/`RoleDoc`; `store.rs` functions are `pub(crate)` (clippy/compile).
- Run still persists a generated key → `config::store::tests::resolve_and_persist_key_generates_and_persists`, `resolve_and_persist_key_drops_legacy_fields_for_both_roles`; E2E (unix, real binary): `tests/config_lifecycle.rs::run_persists_a_generated_key_owner_only_and_reuses_it`, strengthened in Step 11 to check persisted key → NodeId.
- Run adopts a concurrently persisted key → `config::store::tests::resolve_and_persist_key_adopts_concurrently_persisted_key`.
- Load diagnostics are unchanged → `config::store::tests::{load_role_config_missing_file_names_the_path, parse_role_config_reports_path_and_line_without_secret}`, plus `config::dto::tests::{serve_load_missing_file_errors, serve_load_bad_toml_errors, error_chain_never_contains_secret_line}` (switched to `load_role_config`) and `tests/config_lifecycle.rs::parse_errors_never_echo_secret_lines`.
- Legacy fields still ignored and dropped on serialize → `config::dto::tests::legacy_multiplex_values_are_ignored_and_dropped_on_serialize`, `unknown_fields_and_legacy_multiplex_still_load`.
- Clean doc build → Task 2 Step 5 commands; CI `lint` doc step.
- Broken link is rejected → Task 2 Step 6 negative check.
- Deviation visible in API docs → Task 2 Step 4, checked in review (doc text).
- Gates fail on a member-only defect → unchanged CI `lint`/`unit` workspace-wide commands (Task 3 keeps `--workspace`).
- Doc gate fails on a member-only broken link → Task 2 Step 6 (host-only broken link fails the workspace doc command).

**ci-cd-pipeline (Pull-request test gate)**
- Code change → Task 3 Step 1 (the `lint` job runs fmt, clippy and doc on code paths); verified on the PR run.
- Broken rustdoc link → Task 2 Step 6 plus the Task 3 Step 1 CI step (the `ci-ok` dependency is unchanged).
- Installer script change only, Workflow change only, Action change only, Version bump, Mixed change, Docs-only change, A check fails, Superseded push → unchanged `changes` filters and the `ci-ok` job; Task 3 Step 3 confirms `ci.yml` keeps them (actionlint plus the `needs` check).
