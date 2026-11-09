## 1. Config file effects move from DTOs to store functions

Blocked by: none

Files owned: `crates/host/src/config/{dto.rs,store.rs,mod.rs}`, `crates/host/src/runtime/{skeleton.rs,strategy.rs}`, `crates/host/src/service/{install.rs,trust.rs,platform.rs}`, `tests/config_multiplex_compat.rs`, `tests/configured_multiplex.rs`, `tests/config_lifecycle.rs`, `tests/relay_auth.rs` (comment). Platforms: unix trust path (`trust.rs`), Windows path (`platform.rs::windows_load_config`) and the fallback branch, all compiled through the same store functions. Scenarios: "Public DTO API has no file methods", "No file methods remain on the documents internally", "Run still persists a generated key", "Run adopts a concurrently persisted key", "Load diagnostics are unchanged", "Legacy fields still ignored and dropped on serialize"; workspace-boundaries "No test is lost by relocation".

- [x] 1.1 Record the pre-change test inventory: `cargo test --workspace --all-features --locked -- --list --include-ignored` into a scratch file.
- [x] 1.2 TDD: add `store.rs` unit tests for `load_role_config` and `parse_role_config` (missing file → `failed to read config: <path>`; a syntax error on a secret line → `failed to parse config: <path> (line N): …` without the secret; valid text round-trips through `toml::to_string_pretty`). They fail to compile until 1.3.
- [x] 1.3 Add `pub(crate)` `load_role_config`, `parse_role_config` and `resolve_and_persist_key` to `config/store.rs`, moving the `RoleDoc` default bodies verbatim; re-export them `pub(crate)` from `config/mod.rs`.
- [x] 1.4 Move the key tests to `store.rs`: `access_resolve_and_save_key_generates_and_persists` → `resolve_and_persist_key_generates_and_persists`, `access_resolve_and_save_key_no_save_when_present` → `resolve_and_persist_key_no_save_when_present`. Add `resolve_and_persist_key_adopts_concurrently_persisted_key` (the file gains a key after the DTO was loaded keyless) and `resolve_and_persist_key_drops_legacy_fields_for_both_roles` (from root `generated_identity_save_drops_legacy_fields_for_both_roles`).
- [x] 1.5 Switch host callers to the store functions: `runtime/skeleton.rs`, `service/install.rs` (`check_installable_config`, `check_installable_text`), `service/trust.rs::load_config`, `service/platform.rs` (`load_config`, `windows_load_config`); update the `RoleStrategy` doc in `runtime/strategy.rs`.
- [x] 1.6 Remove the inherent `load`/`save`/`resolve_and_save_key` from `ServeConfig`/`AccessConfig` and the default `load`/`from_toml`/`save`/`resolve_and_save_key` from `RoleDoc` (keep `validate`, `node_mut`); rewrite its doc. Convert `dto.rs` tests: the `ServeConfig::load(f.path())` and `AccessConfig::load` calls go through `load_role_config`, `save_roundtrip_writes_file` → `serialized_config_roundtrips`, and root `legacy_multiplex_values_are_ignored_and_removed_on_save` → `legacy_multiplex_values_are_ignored_and_dropped_on_serialize`.
- [x] 1.7 Adapt the root tests. `tests/config_multiplex_compat.rs`: delete the two relocated tests, switch the kept tests' `AccessConfig::load` reads to `toml::from_str`, and update the module doc. `tests/configured_multiplex.rs`: read fixtures with `toml`/`toml_edit` and write them with `std::fs::write` instead of `AccessConfig::load/save`, keeping the same assertions.
- [x] 1.8 Record the post-change inventory and write the before/after test-name map (relocated and renamed tests) for `apply.md`; grep to confirm no `resolve_and_save_key` or DTO `::load(`/`.save(` remains in non-archive sources.
- [x] 1.9 Strengthen the real-binary E2E `tests/config_lifecycle.rs::run_persists_a_generated_key_owner_only_and_reuses_it` to assert that the persisted key's public key equals the printed NodeId. Run on 1.91: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, `cargo test --workspace --all-features --locked`, and default-feature and release builds without test features.

## 2. Rustdoc links, lints and limit_shrink docs

Blocked by: 1

Files owned: `crates/host/src/{lib.rs,config/require_present.rs,config/types.rs,paths/default_config_path.rs,paths/default_state_dir.rs,runtime/run_quick_access.rs,runtime/run_quick_serve.rs,service/restart.rs,service/start.rs,service/stop.rs,service/uninstall.rs,service/types.rs,status/schema.rs,update/types.rs,update/update.rs}`, `crates/engine/src/{lib.rs,observation/types.rs}`; `dto.rs` only if a doc link remains there after group 1. It is blocked by 1 because group 1 rewrites the `RoleDoc` links in `dto.rs`, and the doc build must be measured after it. Scenarios: "Clean doc build", "Broken link is rejected", "Deviation visible in API docs".

- [ ] 2.1 Add `#![deny(rustdoc::broken_intra_doc_links, rustdoc::private_intra_doc_links)]` to `crates/host/src/lib.rs` and `crates/engine/src/lib.rs`; confirm `cargo +1.91 doc --workspace --no-deps --locked` now fails on the existing links (red).
- [ ] 2.2 Fix every unresolved link with a crate path. `update_owner` does not exist: reword to the real owner-decision item. Reword links to crate-private `StatusWriter`/`RoleDoc` from public docs as code spans.
- [ ] 2.3 Extend the `limit_shrink` rustdoc in `crates/engine/src/observation/types.rs` and `crates/host/src/status/schema.rs` with the #152 never-usable clause.
- [ ] 2.4 Verify that `RUSTDOCFLAGS="-D warnings" cargo +1.91 doc --workspace --no-deps --locked` exits 0 with no warnings; confirm a deliberately broken scratch link fails, then revert it. Re-run clippy and fmt.

## 3. CI doc gate and contributor docs

Blocked by: 2

Files owned: `.github/workflows/ci.yml`, `CONTRIBUTING.md`, `docs/agents/rust-coding-standards.md`. It is blocked by 2 because the gate must not land before the tree is warning-free. Scenarios: ci-cd-pipeline "Code change", "Broken rustdoc link"; workspace-boundaries "Doc gate fails on a member-only broken link", "Gates fail on a member-only defect" (unchanged).

- [ ] 3.1 Add a step to `lint` in `.github/workflows/ci.yml` after clippy: `run: cargo doc --workspace --no-deps --locked` with `env: RUSTDOCFLAGS: -D warnings`; rename the job to `lint (fmt + clippy + doc)` and update the header comment.
- [ ] 3.2 Add the doc command to the local gate lists in `CONTRIBUTING.md` and `docs/agents/rust-coding-standards.md` (RS-16 wording).
- [ ] 3.3 Run `actionlint` on `ci.yml` (it is in the `scripts` job); check that `ci-ok` still lists `lint`.
