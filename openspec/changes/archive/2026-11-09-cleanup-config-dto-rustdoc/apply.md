# Apply Receipt

> Generated at the end of the apply phase to mark code-implementation
> complete and provide verify with the state it needs.
> Overwritten on each apply iteration; iteration counter grows.

**Change**: `cleanup-config-dto-rustdoc`
**Iteration**: `1`
**Applied at**: `2026-11-09T12:01Z`
**Executor**: `subagent-driven-development` (implementers: `general-purpose` for Task 1, `easy-worker` for Tasks 2–3; task and final reviewer: `code-reviewer`)

---

## Workspace

- **Worktree path**: `/Users/naicoi/Projects/iroh/iroh-tunnel.cleanup-config-dto-rustdoc`
- **Worktree branch**: `cleanup-config-dto-rustdoc`
- **Feature branch**: `issue-167` (created from `main` before step 0; the scaffold commit `4d0d62b` is on it)
- **Base branch**: `main`

---

## Commits

- **Range**: `4d0d62b..b5b12b9` (this receipt commit follows b5b12b9)
- **Count**: `7` before the receipt commit
  - `dd1b69b` refactor(host): move config DTO file effects into store functions (#167)
  - `d1661b9` test(host): keep key material out of store test assertions (#167)
  - `0bc2886` docs(openspec): tick task group 1
  - `68a1f86` docs(host,engine): fix intra-doc links and deny broken links (#167)
  - `8161d9a` docs(openspec): tick task group 2
  - `15ea76e` ci: deny rustdoc warnings in the lint job (#167)
  - `b5b12b9` docs(openspec): tick task group 3

---

## Tasks

- **Completed**: `16 of 16` checkboxes in tasks.md flipped to `- [x]`
- **Remaining**: none

---

## Contract deviations

- none

---

## Lanes

| Task | Lane branch | Lane range | Review | Merge commit |
|---|---|---|---|---|
| Task 1 | `sequential` | `4d0d62b..d1661b9` | Approved after fix round 1 (RS-10: secret-bearing `assert_eq!` → boolean asserts) | n/a (sequential) |
| Task 2 | `sequential` | `0bc2886..68a1f86` | Approved (clean) | n/a (sequential) |
| Task 3 | `sequential` | `8161d9a..15ea76e` | Approved (clean) | n/a (sequential) |

The final whole-branch review over `4d0d62b..b5b12b9` (code-reviewer) said ready to merge: Yes, with no Critical or Important findings.

---

## Evidence summary

- **Gates** (Rust 1.91, macOS, fresh runs by the final reviewer):
  - `fmt` and `clippy -D warnings` (workspace, all targets and features, locked): pass.
  - `cargo test --workspace --all-features --locked`: **774 passed, 0 failed, 4 ignored**.
  - `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked`, default and `--all-features`: pass, 0 warnings.
  - Default-feature debug and release builds: pass.
  - `actionlint .github/workflows/ci.yml`: pass.
- **E2E (real binary):** `tests/config_lifecycle.rs::run_persists_a_generated_key_owner_only_and_reuses_it` (now asserts persisted key → printed NodeId) passes. The `config_multiplex_compat` and `configured_multiplex` suites (local relay) pass.
- **Test inventory** (`--list --include-ignored`, 774 → 778; every difference is mapped):

  | Before | After |
  |---|---|
  | `config::dto::tests::access_resolve_and_save_key_generates_and_persists` | `config::store::tests::resolve_and_persist_key_generates_and_persists` |
  | `config::dto::tests::access_resolve_and_save_key_no_save_when_present` | `config::store::tests::resolve_and_persist_key_no_save_when_present` |
  | `config::dto::tests::save_roundtrip_writes_file` | `config::dto::tests::serialized_config_roundtrips` |
  | root `config_multiplex_compat::legacy_multiplex_values_are_ignored_and_removed_on_save` | `config::dto::tests::legacy_multiplex_values_are_ignored_and_dropped_on_serialize` |
  | root `config_multiplex_compat::generated_identity_save_drops_legacy_fields_for_both_roles` | `config::store::tests::resolve_and_persist_key_drops_legacy_fields_for_both_roles` |
  | — (new) | `config::store::tests::{load_role_config_reads_and_validates, load_role_config_missing_file_names_the_path, parse_role_config_reports_path_and_line_without_secret, resolve_and_persist_key_adopts_concurrently_persisted_key}` |

- **Limitations:**
  - The Windows code (`service/platform.rs::windows_load_config`) was not compiled locally: there is no mingw cross-linker. The final reviewer inspected it for `cfg` usage, bounds and imports. CI `unit (windows)` must confirm it.
  - The Linux `lint` doc gate is confirmed only by the identical command run locally on macOS.
- **Deferred minor** (from the final review, not blocking): `store.rs` `resolve_and_persist_key_no_save_when_present` does not assert that the file stays unwritten. This weakness was inherited from the old test.

---

## Next step

Run `/opsx-verify`.
