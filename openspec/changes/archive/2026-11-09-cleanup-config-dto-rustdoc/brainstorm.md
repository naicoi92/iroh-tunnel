## Design Summary

Close the drift the refactor review recorded in the archived `2026-11-05-refactor-codebase/apply.md` (issue #167). No user-observable behaviour changes.

1. **Config DTOs become pure data.** `ServeConfig` and `AccessConfig` (`crates/host/src/config/dto.rs`) lose their `load`, `save` and `resolve_and_save_key` methods, and the `RoleDoc` trait loses its filesystem default methods. That behaviour moves into host `config` store free functions. This honours refactor design Decision 3 ("DTOs have no filesystem methods"). Locking, atomic owner-only writes, concurrent-key adoption, retired-field stripping and secret-free diagnostics are preserved unchanged.
2. **Rustdoc is clean and stays clean.** All 20 current `cargo +1.91 doc --workspace --no-deps --locked` warnings are fixed. They are all in `iroh-tunnel-host`: links to crate-private items, wrong paths, and two links to the private `RoleDoc`. The library crates (host, engine) deny `rustdoc::broken_intra_doc_links` and `rustdoc::private_intra_doc_links`. The CI `lint` job adds `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked`, so a broken link fails the pull request.
3. **`limit_shrink` docs state the known deviation.** The engine observation field and the host status-schema field say that, until #152 lands, a connection whose datagrams are never usable also counts every datagram as `limit_shrink`, not only a limit that shrank mid-session.

Living specs involved:
- `workspace-boundaries` — "Host owns persistence and platform policy" (still satisfied, made stricter in code), "Public internal APIs are justified and documented" (rustdoc links), "Validation gates cover every member" (**altered**: adds a doc gate), "Observable contracts are unchanged" and "No test is lost by relocation" (both constrain this change).
- `ci-cd-pipeline` — "Pull-request test gate" (**altered**: `lint` also runs the rustdoc check).

## Alternatives Considered

### Alternative A: Keep the DTO methods, fix docs only
- **Approach**: Fix the doc links, leave `load`/`save`/`resolve_and_save_key` on the DTOs, and record the deviation as accepted.
- **Pros**: Smallest diff; no test movement.
- **Cons**: The design contract stays violated, and the root tests keep using a filesystem API the boundary says should not exist.
- **Why not chosen**: The issue explicitly asks to remove the methods.

### Alternative B: Public store free functions
- **Approach**: Move the methods to `pub` free functions (`config::load_role_config`, `config::save_role_config`, …) so the root integration tests keep calling them.
- **Pros**: The root tests change only in call syntax.
- **Cons**: It widens the host public surface with a raw save that skips the edit/mutation policy, just for tests. That conflicts with "Public internal APIs are justified and documented", because the only callers would be tests.
- **Why not chosen**: The issue says the callers are internal to host; a test-only public API is the wrong seam.

### Alternative C: Docs-only enforcement without CI
- **Approach**: Add the deny attributes but no CI step.
- **Pros**: No CI spec change.
- **Cons**: `cargo doc` never runs in CI, so the attributes would not prevent regressions.
- **Why not chosen**: The user chose the CI gate.

## Agreed Approach

Crate-private store free functions (`pub(crate)`) plus deny attributes plus a CI `cargo doc` gate in `lint`. Host-internal callers (`runtime/skeleton.rs`, `service/install.rs`, `service/trust.rs`, `service/platform.rs`) switch to the free functions. Root integration tests stop using the removed methods:
- **Fixture reads and writes** (`tests/configured_multiplex.rs`): plain `toml` + `std::fs` in the test.
- **Behaviour assertions on save and key persistence** (`tests/config_multiplex_compat.rs`: `legacy_multiplex_values_are_ignored_and_removed_on_save`, `generated_identity_save_drops_legacy_fields_for_both_roles`): move to host unit tests next to the store, with a before/after test-name map so no test is lost.

The crate-private visibility and test relocation were proposed in the Inbox thread without objection; this gate confirms them. The CI gate is the user's explicit choice.

## Key Decisions

- **Visibility:** the new functions are `pub(crate)`, not `pub`. The issue's premise that callers are host-internal was wrong only for root tests, and those tests are adapted rather than kept on a widened API.
- **Doc gate:** the user chose to deny the intra-doc lints on the host and engine library crates and to add `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked` to CI `lint`, a deliberate change to the ci-cd-pipeline and workspace-boundaries specs.
- **No behaviour change:** config text, error messages, exit codes, lock and atomic-write ordering, and key adoption stay identical.
- **#152 itself is out of scope:** only its doc deviation is written down.

## Open Questions

- None blocking. Exact function names and module placement are left to design.md.
