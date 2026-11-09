## Context

Host `config` is one bounded context: the role configuration document. Today it mixes three concerns:
- the serde DTOs (`dto.rs`: `ServeConfig`, `AccessConfig`, `NodeConfig`, services, `UdpSettings`);
- the crate-private `RoleDoc` trait, whose default methods read files, parse text, write files and persist a generated key;
- the locked, atomic store (`store.rs`: `ConfigLock`, `write_atomic`, `persist_generated_key`, `describe_location`).

Callers of the DTO filesystem methods:

| Caller | Uses |
|---|---|
| `runtime/skeleton.rs::run_skeleton` | `cfg.resolve_and_save_key(&path)` (in `spawn_blocking`) |
| `service/trust.rs::load_config` (unix), `service/platform.rs::{load_config, windows_load_config}` | `C::load`, `C::from_toml` |
| `service/install.rs::{check_installable_config, check_installable_text}` | `ServeConfig::load`, `AccessConfig::load`, `from_toml` |
| `runtime/strategy.rs` | `RoleStrategy::Config: RoleDoc` bound (no file I/O itself) |
| `dto.rs` unit tests | `load`, `save`, `resolve_and_save_key` |
| root `tests/configured_multiplex.rs` | `AccessConfig::load` (fixture reads), `save` (fixture rewrite) |
| root `tests/config_multiplex_compat.rs` | `load`, `save`, `resolve_and_save_key` (behaviour assertions) |

No production code calls `save`. Every production write goes through the edit/mutation operations (`mutate.rs`, `set_service.rs`, `remove_service.rs`, `edit.rs`, `keygen.rs`) or `store::persist_generated_key`.

Rustdoc: all 20 warnings are in `iroh-tunnel-host`:
- 2 private links to `RoleDoc` (`dto.rs`);
- 18 unresolved links, either to items not in scope (`CliError::Config/Conflict`, `keygen`, `run_foreground`, `run_quick_serve`, `install`, `check_owner`, `update_owner`, `scope::decide_install_scope`) or to the `pub(crate)` `StatusWriter` from a public item (`status/schema.rs`).

CI (`.github/workflows/ci.yml`) never runs `cargo doc`.

## Goals / Non-Goals

**Goals:**
- DTOs and `RoleDoc` carry schema, validation and node access only. File effects live in crate-private store functions.
- Zero rustdoc warnings on Rust 1.91, enforced by crate lint attributes and by CI `lint`.
- The `limit_shrink` docs name the #152 deviation.
- Identical observable behaviour, and no lost test.

**Non-Goals:**
- #152 itself.
- Changing `persist_generated_key`, the lock or atomic write, or the trust paths.
- Moving `validate` off the DTOs: validation is format logic, not a filesystem effect, and Decision 3 keeps it in host `config`.
- A public replacement API for the removed methods.

## Decisions

### D1. Crate-private free functions in `config/store.rs`

Add to `store.rs`, next to `persist_generated_key` and `describe_location`, which they already depend on:
- `load_role_config::<D>(path)`
- `parse_role_config::<D>(text, path)`
- `resolve_and_persist_key::<D>(&mut D, path)`

Their bodies are the current `RoleDoc` defaults moved verbatim, including the "failed to read config" and "failed to parse config … (line N): msg" text and the `tracing::warn!` line. `config/mod.rs` re-exports them `pub(crate)`.

*Why `store.rs` and not a new module:* the `config/mod.rs` convention puts private infrastructure in `store`, `mutate` and `fixtures`, and every exported operation in its own file. These functions are infrastructure, not exported operations, so `store` is their home, as #167 names it. A new `role_file.rs` would split one persistence seam across two files.

*Why `pub(crate)`:* a public raw load/save would hand frontends a write path that bypasses the edit policy (validation-before-write under the lock, retired-field stripping, privilege and trust). That would violate "Public internal APIs are justified and documented", since the only non-host callers are tests.

*Rejected:* inherent `pub(crate)` methods on the DTOs. They would still be filesystem methods on DTOs, and Decision 3 forbids that independent of visibility.

### D2. `RoleDoc` shrinks to `validate` and `node_mut`

`RoleDoc` keeps `Serialize + DeserializeOwned` bounds and the two required methods. That is what `persist_generated_key`, the trust loaders and `RoleStrategy::Config` actually need. Its docs drop the load/save narrative. The inherent `ServeConfig::validate`/`AccessConfig::validate` stay.

### D3. Raw `save` is deleted, not moved

It has no production caller. Its only behavioural claim (an app-owned save drops legacy `multiplex`) is a property of the DTO schema. Production writers already prove that claim through their own tests (`config_commands_drop_legacy_multiplex_without_changing_other_settings`, `keygen_drops_multiplex_from_inline_services`, `config_edit_saves_valid_editor_output_and_drops_multiplex`, and the persist-key test). Keeping an unused locked writer would be dead code with a security-relevant surface.

### D4. Test relocation

- **Behaviour assertions in `tests/config_multiplex_compat.rs`:**
  - `legacy_multiplex_values_are_ignored_and_removed_on_save` becomes the host unit test `legacy_multiplex_values_are_ignored_and_dropped_on_serialize` in `dto.rs`. It parses each legacy variant, asserts equality with the base, serializes with `toml::to_string_pretty`, and asserts no `multiplex` key and an equal re-parse.
  - `generated_identity_save_drops_legacy_fields_for_both_roles` becomes `resolve_and_persist_key_drops_legacy_fields_for_both_roles` in `store.rs` tests. Same assertions, through `resolve_and_persist_key`.
- **Fixtures in `tests/configured_multiplex.rs`:** read with `toml::from_str::<toml::Value>` (or the existing `toml_edit::DocumentMut`) and rewrite with `std::fs::write`. `toml` and `toml_edit` are already dependencies of the root package. The assertions keep their meaning: the identity key, the service count and name, and the port edit.
- **`dto.rs` unit tests:** they switch to the store functions. `save_roundtrip_writes_file` becomes `serialized_config_roundtrips` (serialize, then `parse_role_config`). The two `access_resolve_and_save_key_*` tests move to `store.rs` as `resolve_and_persist_key_*` with the same names otherwise.
- **Inventory:** a pre/post `cargo test --workspace --all-features -- --list --include-ignored` diff is recorded in `apply.md` with this mapping.

### D5. Rustdoc fixes and lints

- **Out-of-scope links:** fix with explicit paths (`crate::error::CliError::Config`, `crate::config::keygen`, `super::run_foreground`, `crate::service::install`, `crate::update::check_owner`, `crate::service::decide_install_scope`).
- **`update_owner`:** it does not exist. Reword to the field or function that decides delegation, found during apply; do not invent an item.
- **`StatusWriter` and `RoleDoc` from public docs:** reword in code spans without links, because those items are crate-private by design (`private_intra_doc_links`).
- **Lint attributes:** add `#![deny(rustdoc::broken_intra_doc_links, rustdoc::private_intra_doc_links)]` to `crates/host/src/lib.rs` and `crates/engine/src/lib.rs`, next to the existing `#![deny(missing_docs)]`. The root binary crate has no warnings and is not a library API; the CI flag covers it.
- **CI:** add `- run: cargo doc --workspace --no-deps --locked` with `env: RUSTDOCFLAGS: -D warnings` to `lint`, after clippy. `lint` already installs 1.91 via `setup-rust`. The `code` path filter already covers `crates/**` and `src/**`.
- **Docs:** update the local gate lists in `CONTRIBUTING.md` and `docs/agents/rust-coding-standards.md` (RS-16 "No new lint gate is implied" → name the doc gate).

### D6. `limit_shrink` docs

- **Engine** (`observation/types.rs`): append "Until #152, a connection whose datagrams are never usable also counts every datagram here; see `docs/udp-integration-contract.md`."
- **Host** (`status/schema.rs`, "became unusable mid-session"): add the same clause.
- No behaviour or schema change.

## Risks / Trade-offs

- **Error text drift when moving code:** golden strings in host tests and root CLI tests (`failed to read config`, `failed to parse config`) catch it. Move the bodies verbatim.
- **Windows-only items are not documented on Linux `lint`:** `cfg(windows)` docs (`trust_windows`, `service/backend/windows.rs`) are not built there, so a broken link in them would pass CI. Accepted; documenting per OS would need a Windows doc job, which is out of scope. The crate deny attribute still fails a local Windows `cargo doc`.
- **Future rustdoc lint changes on a toolchain bump could fail `lint`:** that is the intended signal, and the toolchain is pinned to 1.91.
- **Dropping the root-level "public boundary" tests reduces integration coverage of DTO load/save:** the removed API is no longer a public boundary. Real-binary tests (`config set/rm/edit`, `keygen`, `run` persisting the key) still exercise persistence end to end.

## Migration Plan

Single PR; no data or format migration. Rollback is a revert.
