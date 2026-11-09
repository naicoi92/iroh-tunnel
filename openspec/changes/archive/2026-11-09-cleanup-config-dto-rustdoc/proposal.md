## Why

The refactor review (archived `2026-11-05-refactor-codebase/apply.md`, issue #167) recorded two deviations. First, the `ServeConfig`/`AccessConfig` DTOs still carry `load`, `save` and `resolve_and_save_key`, against the design rule that DTOs have no filesystem methods. Second, `cargo doc` reports 20 broken or private intra-doc links. Nothing in CI catches either, so both will keep drifting. Fixing them now, while the boundary is fresh, keeps the host configuration model honest and the rustdoc usable.

## What Changes

**Config DTOs**
- From: `ServeConfig`/`AccessConfig` expose `pub fn load`, `save` and `resolve_and_save_key`, and the crate-private `RoleDoc` trait carries filesystem default methods (`load`, `from_toml`, `save`, `resolve_and_save_key`).
- To: the DTOs and `RoleDoc` hold schema, validation and node access only. Loading, parsing and key persistence are crate-private free functions in host `config`. The unused raw `save` is removed: production writes already go through the locked edit/mutation operations and key persistence.
- Reason: refactor design Decision 3 says DTOs have no filesystem methods.
- Impact: the `iroh-tunnel-host` Rust API loses three inherent methods on each DTO (`publish = false`, internal workspace crate). The only out-of-host callers are the root integration tests, which are adapted. No CLI, file-format, exit-code or log change.

**Rustdoc**
- From: `cargo +1.91 doc --workspace --no-deps --locked` emits 20 warnings, and CI never builds docs.
- To: zero warnings. `iroh-tunnel-host` and `iroh-tunnel-engine` deny `rustdoc::broken_intra_doc_links` and `rustdoc::private_intra_doc_links`, and the CI `lint` job runs `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked`.
- Reason: keep the links correct once they are fixed.
- Impact: `lint` takes slightly longer; a pull request with a broken link fails `lint`, and therefore `ci-ok`.

**`limit_shrink` documentation**
- The engine observation and host status-schema docs for `limit_shrink` state the known #152 deviation: when datagrams are never usable on a connection, every datagram is also counted there. This matches the existing `udp-datagram-forwarding` requirement "Unusable datagrams are dropped, not failed at setup". Documentation only.

**Non-goals**
- Implementing #152 (failing the session at setup).
- Changing the TOML format, lock or atomic-write ordering, key adoption, error text or exit classification.
- Other DTO, `toml_edit` or API cleanups.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `workspace-boundaries`: adds a requirement that configuration documents carry no filesystem behaviour, and extends "Validation gates cover every member" with a warning-free rustdoc gate.
- `ci-cd-pipeline`: "Pull-request test gate" — `lint` also builds the workspace rustdoc with warnings denied.

## Impact

- **Code:**
  - `crates/host/src/config/{dto.rs,store.rs,mod.rs}`, plus a new crate-private config file module.
  - `crates/host/src/{runtime/skeleton.rs,runtime/strategy.rs,service/install.rs,service/trust.rs,service/platform.rs}`.
  - Doc-link fixes across `crates/host/src/{config,paths,runtime,service,status,update}`.
  - `limit_shrink` docs in `crates/engine/src/observation/types.rs` and `crates/host/src/status/schema.rs`.
  - Lint attributes in `crates/{host,engine}/src/lib.rs`.
- **Tests:**
  - `tests/configured_multiplex.rs` and `tests/config_multiplex_compat.rs` stop using the removed methods.
  - Save and key-persistence assertions move to host unit tests, with a test-name map.
- **CI and docs:** `.github/workflows/ci.yml` (`lint`), `CONTRIBUTING.md` and `docs/agents/rust-coding-standards.md` (local gate list).
- **Dependencies:** none.
- **Compatibility:** no user-visible change on any platform. Windows-only `cfg` code is not documented on the Linux `lint` runner.
- **Security:** secret-free parse diagnostics (RS-10), owner-only writes and the system-service trust path are unchanged.
