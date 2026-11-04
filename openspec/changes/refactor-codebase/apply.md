# Apply Receipt

> Generated at the end of the apply phase to mark code-implementation
> complete and provide verify with the state it needs.
> Overwritten on each apply iteration; iteration counter grows.

**Change**: `refactor-codebase`
**Iteration**: `1`
**Applied at**: `2026-11-05 03:30`
**Executor**: `subagent-driven-development`

---

## Workspace

- **Worktree path**: `/Users/naicoi/Projects/iroh/iroh-tunnel.refactor-codebase`
- **Worktree branch**: `refactor-codebase`
- **Feature branch**: `explore/refactor-codebase`
- **Base branch**: `main`

Parallel lanes ran in temporary sibling worktrees (`refactor-codebase-{t16,t17,svc,ci,t21,t22,t23,t24,t25}`); each lane was reviewed and cherry-picked onto `refactor-codebase`. Those lane branches hold no unique work and can be removed.

---

## Commits

- **Range**: `393e396..f0f0bc4` (393e396 = change artifacts committed on the feature branch; implementation starts at a2c6cdd)
- **Count**: `66`

---

## Tasks

- **Completed**: `47 of 47` checkboxes in tasks.md flipped to `- [x]`
- **Remaining**: `none`

### Verification at receipt time (Rust 1.91, macOS aarch64)

- `cargo fmt --all -- --check`: clean
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`: clean
- `cargo build --workspace --locked` (default features) and `cargo build --release --locked`: OK
- `cargo test --workspace --all-features --locked`: 39 targets, **547 passed, 0 failed, 9 ignored**
- Release normal-feature graph contains no `test-utils` (count 0)
- `--version`: `iroh-tunnel|iroh-tunnel-serve|iroh-tunnel-access 0.4.3-rc.3-dev`, the same format as before
- Test inventory: 419 → 553 listed; all 179 removed names mapped (`baseline/test-mapping.md`); external crates: only the two new workspace members added
- Native E2E on macOS (launchd): `caller_exit_leaves_user_service_running` and `captured_systemctl_status_is_returned` (launchctl variant) passed

### Not verified here (blocked)

- Linux and Windows `cfg` code was not compiled locally (the `ring`/`aws-lc-sys` C toolchains are missing). Linux had one zig cross-check during Task 20b. **Linux and Windows CI must be green before release.**
- Linux systemd and BusyBox lifecycle tests, the Windows Task Scheduler mirror, and `update_zip_windows` need CI or native hosts.
- Public-relay tests (`serve_access_tunnel` ignored set ×4, `release_smoke`) and signed `release_artifacts` ×4 have no safe local harness.
- `docker build` was not run (daemon not running).

---

## Contract deviations

- `RetargetConsent.previous`: `Pin` → `Option<Pin>`. With `Pin`, consent cannot be given for a legacy unpinned registration. **User-approved** (Inbox).
- `render_service_note` (root, internal): returns `(Stream, String)` so that stdout/stderr routing stays byte-identical (ruling R2).
- Host bridge re-exports (R3): `iroh_tunnel_host` re-exports `RunContext, ReadyReport, ReadyServices, ServeReady, AccessReady, Protocol, OnReady` from engine, so root has no normal engine dependency. `OnReady` was added in Task 20a.
- `ServiceNote`: added `RoleRunning`, `RoleNotRunning`, `TaskListing` to keep Windows `service status` text byte-identical (Task 13).
- `NativeOutput`: private `status_display` field, so error texts keep std's `ExitStatus` Display for signal-killed children (Task 13 fix).
- `paths::MigrationNotice` / `set_migration_notice` (new, public): write-once hook so host never prints the macOS legacy-config migration notice. Root installs the exact legacy text; without a hook host logs `tracing::info!` (Task 20a ruling).
- Host read accessors added in Task 20b, so root does not reach into internals: `config::resolve_write_target`, `service::{account_name, unknown_user}`, `StatusPayload::{pid, started_at}`, `Role::status_file_name`, `status::{describe_owner, short_peer_id, Liveness, OwnerRecord, Tier}`, crate-root `Role`. The api-audit classifies these as contract additions with no violations.
- `ServiceStatusReport.owner`: `Fact<OwnerRecord>`, where a not-running role is reported as `Unavailable("<role> is not running")`. **A user decision is pending** (Inbox), on whether to change it to `Fact<Option<OwnerRecord>>`.
- Root `[lib]` target kept (the plan preferred removal): the three binaries share it, and `tests/access_status_file.rs` uses the root status renderers.
- Engine `settings` is a flat `settings.rs` (the plan listed `settings/{mod,types}.rs`).

Recorded during finalize, from verify warnings W3–W6. These are contract drift with no behaviour change:

- `config::{ServeConfig, AccessConfig}` (W3): these DTOs keep `load`, `save` and `resolve_and_save_key`, which design Decision 3 says DTOs should not have. Only internal host callers use them (`runtime/skeleton.rs`, `service/install.rs`, `service/trust.rs`, `service/platform.rs`), and they are not part of the management surface root uses. A follow-up can move them into `config` store free functions.
- Engine `testing` module (W4): the module is always compiled. Its insecure items (`set_relay_only`, the env-driven `hang_point`) are `#[cfg(feature = "test-utils")]`, and a production no-op `hang_point` keeps callers cfg-free. No test bypass is reachable without `test-utils`.
- `service::Environment::detect` (W5): it lives in root as `cli_service::detect_environment`, because host must not probe stdin (Task 14/20a rulings). Host keeps only the `Environment { elevated, can_answer }` value.
- Root `render` (W6): `render_serve_status` and `render_access_status` stay in `src/status_cmd.rs` (the Task 16 parallel-lane file constraint). The output is unchanged.

### Observable differences (accepted, not contract changes)

- Access startup: stderr log lines (relay resolution, "nothing to expose") are now emitted before the stdout `NodeId:` line. These are separate streams; stdout bytes are unchanged.
- A panic inside a config/service/status command now exits 1 (a JoinError from `spawn_blocking`) instead of 101. This affects bug paths only; the exit classes 0–6, 130 and 1 are unchanged.
- The "wrote status file" log moved from `info` to `debug`, because it now fires on every flush.
- `RoleDoc::from_toml` errors no longer quote the offending source line. This restores the existing spec "Diagnostics never echo secret lines" on the run/load path.
- With `--insecure --to <invalid>`, the insecure warning now precedes the version error (failure-edge stderr order).

### Follow-ups (not blocking merge)

- Move quick-mode config construction (`quick_protocol`, `read_key_file`, node-id parse) from root `app.rs` into host.
- About 22 `cargo doc` intra-doc link warnings (broken or private links) remain.
- Status tests assert the program name only, not `status: Some(..)` (FIM-18 wording).
- The Windows `service status` probes liveness twice.
- The minor-only items listed in the SDD ledger (test hygiene, README row wording, Windows-update filter on `src/app.rs`).

---

## Next step

Run `/opsx-verify`. Linux and Windows CI must pass before finalize.
