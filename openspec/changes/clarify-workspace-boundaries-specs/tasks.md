## 1. Engine dependency rule test

Blocked by: none

- [ ] 1.1 Add `engine_dependencies_are_inherited_from_the_workspace` to `tests/workspace_layout.rs`. It parses `crates/engine/Cargo.toml` with `toml` and checks every entry of `dependencies`, `dev-dependencies` and `build-dependencies`, top level and under each `target.<cfg>`, for a table with `workspace = true`, as design D2 specifies. A failure names each offending table and entry (scenario "Engine manifest inherits every dependency").
- [ ] 1.2 Prove the negative case: a unit-level helper test feeds an in-memory manifest with `foo = "1"` and `bar = { path = "../bar" }`, including one under `[target.'cfg(unix)'.dependencies]`, and asserts that both are reported (scenario "Directly versioned engine dependency is rejected").
- [ ] 1.3 Run `cargo +1.91 test --locked --all-features --test workspace_layout` and `cargo +1.91 clippy --workspace --all-targets --all-features --locked -- -D warnings`.

## 2. Purpose for the 14 placeholder specs

Blocked by: none

- [ ] 2.1 Replace `TBD` with a one-to-three-sentence Purpose, derived only from that spec's own requirements (design D1), in `openspec/specs/<capability>/spec.md` for `cli-entrypoints`, `config-management-commands`, `connection-path-status` and `frontend-independent-management`.
- [ ] 2.2 Do the same for `install-release-packaging`, `iroh-endpoint-relay`, `role-ownership-shutdown`, `self-update` and `system-service-management`.
- [ ] 2.3 Do the same for `tcp-tunnel-forwarding`, `tunnel-configuration`, `udp-datagram-forwarding`, `windows-service-trust` and `workspace-boundaries`.
- [ ] 2.4 Run `openspec validate --specs` and confirm that no Purpose warning remains and every spec passes.

## 3. CONTRIBUTING migration-map link

Blocked by: none

- [ ] 3.1 In `CONTRIBUTING.md` § "Workspace layout and Rust API migration", remove the paragraph that links `openspec/changes/archive/2026-11-05-refactor-codebase/migration-map.md`, and rename the heading to "Workspace layout"; keep the layout bullets (brainstorm decision 4).
- [ ] 3.2 Check that no Markdown link to the old anchor `#workspace-layout-and-rust-api-migration` remains outside `openspec/changes/archive/`, and that no living doc links `migration-map.md` (commands in plan Task 3 Step 2).

## 4. Change-level verification

Blocked by: 1, 2, 3

- [ ] 4.1 Run `openspec validate clarify-workspace-boundaries-specs --strict` and `openspec validate --specs`.
- [ ] 4.2 Run the workspace gates on Rust 1.91: `cargo +1.91 fmt --all -- --check`, clippy as in 1.3, `cargo +1.91 test --workspace --all-features --locked`, and `RUSTDOCFLAGS="-D warnings" cargo +1.91 doc --workspace --no-deps --locked`.
- [ ] 4.3 E2E check for the restated exit-code scenarios through the real CLI, as a user runs it:
  - `iroh-tunnel serve config check --config <file with port = 0>` exits 2 and names the path and `invalid port`;
  - `iroh-tunnel serve config list` on malformed TOML exits 2.

  - `iroh-tunnel serve run --config <file with secret_key = "%%%">` fails with `invalid secret_key` and not with `failed to parse config`.

  Record the commands and their output.
- [ ] 4.4 Open a GitHub issue in `naicoi92/iroh-tunnel`, after user approval in chat, for the defect found in design D6: `serve run --config` exits 1 on a config error, and a validation error does not name the path.
