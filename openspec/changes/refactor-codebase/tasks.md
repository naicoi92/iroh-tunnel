# Tasks — refactor-codebase

Traceability keys: `WB-n` = requirement n in `specs/workspace-boundaries/spec.md`; `FIM-n` = requirement n in `specs/frontend-independent-management/spec.md` (numbered in file order). Existing capability specs are cited by name. Every task lists its scenarios, file ownership (one writer per file per group), dependencies and platform coverage. Exact signatures, the acceptance-to-test matrix and commands belong in `plan.md`.

Platform legend: **all** = Linux, macOS and Windows via `cfg` compilation plus host-native tests; **linux-native**, **macos-native**, **windows-native** = needs that OS (CI workflow or local); **blocked-ok** = record as blocked when unavailable, never claim E2E.

## 1. Baseline and characterization (Migration step 1)

Owner: `openspec/changes/refactor-codebase/baseline/**` (new), new characterization tests under `tests/` only. No production source edits. Depends on: nothing.

- [x] 1.1 Record the baseline test inventory: `cargo +1.91 test --all-features --locked -- --list` and `-- --list --ignored` per target, saved under `baseline/`. Traces WB-20 (Test inventory comparison). Platform: all; record Windows/macOS-only targets from CI listing.
- [x] 1.2 Record the baseline external crate set from `Cargo.lock`, the normal and `--all-features` feature trees (`cargo tree -e features`), and the three binary target names/paths. Traces WB-3, WB-18 (Release graph excludes test-utils), WB-21.
- [x] 1.3 Inventory every `pub` item reachable from `src/lib.rs` and every `iroh_tunnel::*` use in `tests/`, and draft the starting old-path column of the Rust API migration map. Traces WB-23, WB-24.
- [x] 1.4 Inventory path assumptions in `.github/workflows/*.yml`, `Dockerfile*`, `packaging/`, `install.sh`, `install.ps1`, `build.rs`, docs and fixtures (`include_*`, literal `src/`/`tests/` paths, root `[package].version` reads). Traces WB-21, WB-22.
- [x] 1.5 Add process-level characterization tests pinning exit codes 2/3/5/6 and stderr guidance for: invalid config, unprivileged `service install --system`, Windows `--system`, ownership conflict. Traces WB-15 (all three scenarios), FIM-10 (Windows remains user-only), `cli-entrypoints`. Platform: all; Windows case windows-native.
- [x] 1.6 Add characterization tests pinning CLI output byte-for-byte for `config list`, `config show` (redacted), `config path`, `status --json` raw passthrough and the `service status` note/native-output ordering (fake backend where native is unavailable). Traces WB-16, FIM-19, FIM-22, `config-management-commands`, `connection-path-status`. Platform: all; ordering on systemd linux-native.
- [x] 1.7 Add acceptance characterization for unknown fields, legacy `multiplex`, hostname targets and NodeId-length-at-load validation stage. Traces WB-17 (Previously accepted configuration still loads), `tunnel-configuration`.
- [x] 1.8 Add status-publishing characterization: initial write before first admission, unchanged observation not rewritten, retry after failed write, file kept when the flush join times out. Traces WB-12 (both scenarios), WB-13 (all three scenarios), `connection-path-status`.

## 2. In-package boundaries: drain context, ownership and redaction (Migration step 2)

Owner: `src/shutdown.rs`, `src/udp.rs`, `src/serve.rs`, `src/access.rs`, `src/role_run.rs`, `src/config.rs` (Debug impls only), `src/main.rs`, `src/bin_*.rs`. Depends on: 1.

- [x] 2.1 Write failing tests, then replace process-global `DRAIN_DEADLINE` with a per-invocation drain context shared by the role task tree and the CLI watchdog; keep budgets, repeated-signal policy and forced exit codes. Traces WB-8 (both scenarios), `role-ownership-shutdown`. Platform: all.
- [x] 2.2 Ensure management-only code paths create no drain context and install no signal handler; test with a config/status operation in a process without a role. Traces WB-9.
- [x] 2.3 Move UDP Quarantine sink tasks under a role-lifetime owner that retains, reaps, observes and joins/aborts them within the role stop deadline, with guarded Flow cap slot release on every exit. TDD against disconnect, role stop and sink failure. Traces WB-10, WB-11 (all three scenarios), `udp-datagram-forwarding`, docs/udp-integration-contract.md.
- [x] 2.4 Make the status flush task supervisor-owned and report writer quiescence on every completion path; remove the owned status file only after quiescence and endpoint close. Traces WB-12, WB-13 (re-run 1.8).
- [ ] 2.5 Implement redacted `Debug` for every secret-bearing type (config DTOs, requests, edit attempts, documents, outcomes) and add Debug/error-chain leakage tests. Traces WB-14 (both scenarios), FIM-5. Storage format unchanged.

## 3. In-package boundaries: runtime output and status callback (Migration step 2)

Owner: `src/serve.rs`, `src/access.rs`, `src/conn_path.rs`, `src/status.rs`, `src/role_run.rs`, `src/app.rs` (startup rendering only). Depends on: 2.

- [ ] 3.1 Replace `println!`/`RoleStrategy::print_services` with a typed ready description delivered to an output hook before forwarding admission; CLI renders the existing text. Traces WB-5 (Startup readiness is reported as a value), `cli-entrypoints`.
- [ ] 3.2 Make runtime code emit native observation values (service order, stream/UDP counters, peer paths, limits) and inject the async save callback; move JSON DTO mapping to the status adapter. Equality matches rendered-value change detection without timestamps. Traces WB-5 (Status observations carry no file schema), WB-13.
- [ ] 3.3 Verify through real local-relay Serve/Access runs that status files, logs and stdout are byte-compatible (tests from 1.6, 1.8 and existing `serve_status_conn_path`, `access_status_file`). Traces WB-16.

## 4. In-package boundaries: management use cases (Migration step 2)

Owner: `src/config.rs`, `src/config_store.rs`, `src/config_cmd.rs`, `src/status_cmd.rs`, `src/app.rs`, `src/service/**`, `src/update.rs`, `src/update_owner.rs`, `src/cli.rs`. Depends on: 1; may run in parallel with 2–3 only if `src/app.rs` and `src/role_run.rs` edits are sequenced (single writer).

- [ ] 4.1 Extract config operations (resolve/path, add, remove, list, show, keygen, random key) taking plain requests and returning outcomes; CLI only maps args and renders. TDD per operation. Traces FIM-1, FIM-3 (both), FIM-4 (both), FIM-5, `config-management-commands`.
- [ ] 4.2 Add keygen rotation-consent outcome rechecked under the writer lock; CLI keeps prompt/`--yes`/non-TTY refusal. Traces FIM-6 (all four scenarios).
- [ ] 4.3 Split edit into prepare/commit with original-bytes comparison under the lock, `.rejected` sidecar, scratch cleanup on drop; `$EDITOR` invocation stays in CLI outside the lock. Traces FIM-7 (both), FIM-8.
- [ ] 4.4 Extract service operations (install/uninstall/start/stop/restart/status) with typed role/scope/paths, structured needs (scope, invoking user, retarget with old+proposed pin), fresh rechecks before mutation and private raw backend dispatch. Traces FIM-9 (both), FIM-10, FIM-11 (both), FIM-12 (both), `system-service-management`. Platform: all; fake-backend unit tests plus linux-native, macos-native, windows-native lifecycle.
- [ ] 4.5 Require an explicit tunnel executable path for install; keep CLI `PATH`/own-exe resolution and standalone-binary refusal at the CLI. Traces FIM-13 (both scenarios).
- [ ] 4.6 Introduce the child-process adapter: captured mode for non-terminal callers, inherited mode plus synchronous note observer for CLI; preserve backend non-zero rules. Traces FIM-18, FIM-19. Platform: linux-native for systemd ordering; others via fakes.
- [ ] 4.7 Return structured service status (registration facts, kernel-backed liveness, owner metadata, diagnostics, explicit unavailable fields) without parsing localized prose. Traces FIM-16, FIM-17.
- [ ] 4.8 Preserve composite install/uninstall semantics, start ownership refusal before manager call, partial-effects reporting with no rollback, no stop-on-drop. Traces FIM-14, FIM-15 (both), FIM-24 (both).
- [ ] 4.9 Extract role-status read returning raw body, typed snapshot, liveness/owner data and freshness condition, with no side effects; CLI keeps tables, warnings and raw `--json`. Traces FIM-20 (both), FIM-21, FIM-22.
- [ ] 4.10 Replace `UpdateArgs` with explicit update inputs (target executable, version, overwrite, insecure) and outcomes; preserve verification, delegation, atomic replacement and user-agent; no service restart. Traces FIM-13, FIM-25, FIM-26, `self-update`. Platform: all; Windows zip replacement windows-native.
- [ ] 4.11 Ensure config/service operations are synchronous, run under `spawn_blocking` from the CLI, and create no runtime. Traces FIM-23, FIM-24.

## 5. Extract `iroh-tunnel-engine` (Migration step 3)

Owner: `Cargo.toml` (workspace section), `Cargo.lock`, `crates/engine/**` (new), moved engine modules from `src/`, engine-owned tests. Depends on: 2, 3.

- [ ] 5.1 Convert root `Cargo.toml` to package+workspace manifest (`resolver = "2"`, all three default members, inherited version/edition/MSRV/license, shared dependency declarations without test features) and create `crates/engine` with `publish = false`. Traces WB-1 (both), WB-21.
- [ ] 5.2 Move `endpoint.rs`, `proto.rs`, `pipe.rs`, `udp.rs`, `conn_path.rs` observation, Serve/Access role loops and run-local drain mechanics into engine modules (`runtime`, `tcp`, `udp`, `observation`); keep UDP decision types together. Traces WB-4, WB-5.
- [ ] 5.3 Forward `test-utils` engine-locally to `iroh/test-utils`; move in-process network tests that exercise engine (`serve_access_tunnel`, `udp_tunnel`, `configured_multiplex`, `relay_auth`, `access_path_logging`) to engine or keep at root via explicit feature forwarding. Traces WB-18 (both), WB-20.
- [ ] 5.4 Add a dependency-direction check (cargo metadata assertion test or documented command) proving engine depends on neither host nor root, and a source scan for `process::exit`, signal handlers and `println!`/`eprintln!` in non-test engine code. Also add a check of the one-exported-operation-per-file convention (design.md Decision 1). Traces WB-2 (Engine depends on neither host nor root), WB-5 (No exit, signal or print calls).

## 6. Extract `iroh-tunnel-host` (Migration step 4)

Owner: `crates/host/**` (new), moved host modules from `src/`, host-owned tests, `src/error.rs` → host. Depends on: 4, 5.

- [ ] 6.1 Create `crates/host` (`publish = false`) and move config/store/identity, paths, ownership, trust, foreground orchestration, status schema/store/read, service backends and updater into modules `config`, `service`, `status`, `runtime`, `update`. Traces WB-6, WB-7, FIM-1.
- [ ] 6.2 Move `CliError` into host, preserving downcast through `anyhow` context; root keeps exit mapping. Re-run 1.5 characterization. Traces WB-15.
- [ ] 6.3 Make raw backend dispatch, OS command construction, trust handles, lock internals and ownership-bypass entrypoints private or `test-utils`-gated; add rustdoc (ownership, cancellation, errors) to every widened `pub` item. Traces WB-23, FIM-2, FIM-9.
- [ ] 6.4 Forward `test-utils` root → host → engine; remove the root self-dev-dependency; move host-owned tests (`config_lifecycle*`, `config_multiplex_compat`, `update*`, `role_ownership` where in-process) or keep real-binary ones at root. Traces WB-18, WB-20.
- [ ] 6.5 Add checks proving host has no `clap` dependency, no root dependency, no `process::exit` or stdin reads, opens no listener/socket in management calls, and follows the one-exported-operation-per-file convention (design.md Decision 1). Traces WB-2 (Host does not depend on the root package), WB-7 (both), FIM-2.

## 7. Root CLI composition and distribution integration (Migration step 5)

Owner: `src/**` remaining root files, `build.rs`, `.github/workflows/**`, `Dockerfile*`, `docker-compose.yml`, `packaging/**`, `install.sh`, `install.ps1`, `docs/**`, `README.md`, `CONTRIBUTING.md`, `tests/` root real-binary tests and fixtures. Depends on: 6.

- [ ] 7.1 Reduce root `src/` to clap translation, prompts/editor, rendering, tracing, Tokio construction, signal/watchdog and exit mapping, with no duplicated host policy. Traces WB-6 (CLI mutations go through host operations).
- [ ] 7.2 Keep three binary targets, aliases and `APP_VERSION`; update the build workflow to read the inherited workspace version. Traces WB-21 (both scenarios). Platform: CI build workflow.
- [ ] 7.3 Update CI path filters, cache-warm/snapshot inputs, Docker build contexts (all member manifests/sources), packaging and release scripts; keep installer URLs and artifact/checksum/signature names. Traces WB-22 (both). Platform: Docker build local; release artifacts blocked-ok.
- [ ] 7.4 Update fixture `include_*` paths and docs/rustdoc links for moved files; keep root-relative installer and release fixtures. Traces WB-20, WB-22.
- [ ] 7.5 Write the Rust API and repository-path migration map (`openspec/changes/refactor-codebase/migration-map.md` and a summary in `CONTRIBUTING.md`), covering every entry from 1.3. Traces WB-24.

## 8. Verification (Migration step 6)

Owner: `openspec/changes/refactor-codebase/baseline/**` comparisons; no source edits except fixes routed back to the owning group. Depends on: 7.

- [ ] 8.1 Run workspace gates on Rust 1.91: `fmt --all --check`, `clippy --workspace --all-targets --all-features --locked -D warnings`, `test --workspace --all-features --locked`, default-feature build and `cargo build --release --locked` without test features. Traces WB-19, WB-21.
- [ ] 8.2 Compare test inventory, external crate set and normal/test feature graphs with the 1.1–1.2 baseline; document every split/merge/removal. Traces WB-3, WB-18, WB-20.
- [ ] 8.3 Re-run all characterization tests (1.5–1.8) and every requirement-linked test of the eleven existing capabilities through real binaries and local relays. Traces WB-16, WB-17, `cli-entrypoints`, `tunnel-configuration`, `connection-path-status`, `tcp-tunnel-forwarding`, `udp-datagram-forwarding`, `iroh-endpoint-relay`, `role-ownership-shutdown`, `config-management-commands`.
- [ ] 8.4 Exercise native service lifecycles from both CLI and a test-only non-terminal host caller (install, start, caller exit, status, stop, uninstall): systemd user/system (linux-native), BusyBox root (linux-native, blocked-ok), launchd (macos-native), Task Scheduler (windows-native via `windows-service-e2e.yml`). Traces FIM-14 (Caller exits after start), FIM-15, FIM-16, FIM-18, `system-service-management`.
- [ ] 8.5 Exercise updater flows including Windows zip replacement (`windows-update-e2e.yml`) and package-manager delegation; record signed-release and public-relay checks as blocked-ok unless their safe harness is available. Traces FIM-25, FIM-26, `self-update`, `install-release-packaging`.
- [ ] 8.6 Confirm the migration map is complete against the final public API of both internal packages and that no test-only bypass is exported without `test-utils`. Traces WB-23 (Former test bypass is not a production API), WB-24.
