# Ownership Fail-Closed Implementation Plan

> Executed via `/opsx-apply`, which chooses the executor.
> Spec reference: this change's `specs/` and `design.md`.

**Goal:** Make the cross-role identity check and the Windows machine tier fail closed when an owner record or `%ProgramData%` cannot be read or resolved (issue #221).

**Architecture:** All decisions stay in `crates/host/src/ownership.rs`. Tier directories are resolved once, up front, into a private `TierDirs` that `claim`, `probe` and `publish_identity` share; owner records gain a tri-state read and a fallible write. Callers in `service/install.rs` propagate the resolution error; `service/log_file.rs` stays best-effort.

**Tech Stack:** Rust 2021 (MSRV 1.91), anyhow, serde_json, std file locks, tempfile in tests.

## Global Constraints

- No-panic policy (#216): production code has no `unwrap`/`panic!`/`todo!`/`unimplemented!`; `expect`/`unreachable!`/indexing/slicing only with `#[expect(clippy::<lint>, reason = "invariant: ...")]` on the narrowest item; OS/IO errors → `Result` + context; no `print!`/`println!` in production. Tests may unwrap.
- `CliError` classification preserved (conflict = exit 6, via `start_refused`).
- No new dependency, no change to lock layout, record format or public CLI.
- Gates: `cargo fmt`; `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`; `cargo test --all-features --locked` with `RUBY=/opt/homebrew/Library/Homebrew/vendor/portable-ruby/current/bin/ruby`.

## Review Focus

- Concurrent same-key starts: both publish before inspecting, so at least one refuses — pinned by `publish_identity_refuses_the_same_key_in_the_other_role` (unit) and the existing process test.
- Other role stopping (record removed, lock still held) → must not refuse — covered by `publish_identity_proceeds_while_the_other_role_has_no_record`.
- System service of role A starting while user processes hold role B's machine lock shared (no or stale machine record) → must not refuse — `publish_identity_ignores_a_machine_record_under_a_shared_hold`.
- Garbage record left by a crash with the lock free → must not block — `publish_identity_ignores_a_corrupt_record_of_a_stopped_role`.
- Restrictive umask hiding the system holder's machine record from users → `machine_owner_record_is_world_readable_under_umask_077`.

---

## Contracts

| Name | new / modified | Shape | public |
|---|---|---|---|
| `ownership::machine_dir` | modified | `pub(crate) fn machine_dir() -> anyhow::Result<Option<MachineDir>>`; `Ok(None)` only where the platform has no machine tier | no |
| `ownership::machine_dir_from_program_data` | new (replaces `windows_machine_dir`) | `fn machine_dir_from_program_data(program_data: anyhow::Result<PathBuf>) -> anyhow::Result<Option<MachineDir>>`; `Ok(p)` → `Some(MachineDir { path: p.join("iroh-tunnel"), verify: true })`; `Err(e)` → `e.context("cannot resolve the machine lock directory (%ProgramData%)")` | no |
| `ownership::TierDirs` | new | `struct TierDirs { user: PathBuf, machine: Option<MachineDir> }`; `fn resolve() -> Result<TierDirs>` = `resolve_with(machine_dir)`; `fn resolve_with(machine: impl FnOnce() -> Result<Option<MachineDir>>) -> Result<TierDirs>` resolves the machine dir first, then `user_dir()?` | no |
| `LockDirs::resolve_with` | new | `fn resolve_with(home: Option<&Path>, machine: impl FnOnce() -> Result<Option<MachineDir>>) -> Result<LockDirs>`; `resolve(home)` = `resolve_with(home, machine_dir)` | no |
| `held_exclusively` | new | `fn held_exclusively(path: &Path) -> Result<bool>`: open read-only (missing → `false`), `try_lock_shared`: `WouldBlock` → `true` | no |
| `ownership::claim` / `claim_in` | modified / new | `claim(role) = claim_in(role, TierDirs::resolve()?)`; `fn claim_in(role: Role, dirs: TierDirs) -> Result<RoleClaim>` | no |
| `ownership::probe_tiers_in` | new | `fn probe_tiers_in(role: Role, dirs: &TierDirs) -> Result<Vec<Liveness>>`; `probe_tiers(role)` resolves then delegates | no |
| `RoleClaim` | modified | fields `role: Role, dirs: TierDirs, holds_machine: bool, _locks: Vec<File>` (replaces `user_dir`, `machine_dir`) | no |
| `RoleClaim::publish_identity` | modified behaviour | same signature `fn publish_identity(&self, node_id: &str) -> Result<()>`; now errors on write failure and on an unreadable record of a running other role | no |
| `OwnerRead` | new | `enum OwnerRead { Absent, Present(OwnerRecord), Unreadable { path: PathBuf, error: anyhow::Error } }`; `fn read_owner_record(dir: &Path, role: Role) -> OwnerRead` | no |
| `write_owner` | modified | `fn write_owner(dir: &Path, role: Role, node_id: Option<&str>, mode: u32) -> Result<()>`; error context `cannot publish owner record <path>` | no |
| `LockDirs::resolve` | modified behaviour | signature unchanged; propagates `machine_dir()` errors | no |
| Identity-unknown refusal message | new | `start refused: the <other> role is running but its identity cannot be determined (<record path>: <cause>); <stop hint>` via `CliError::Conflict` | yes (stderr text) |

---

## Task 1: Fail-closed machine directory resolution — implements 1

**Covers:** 1.1, 1.2, 1.3, 1.4, 1.5
**Difficulty:** Normal
**Blocked by:** none

**Files:**
- Modify: `crates/host/src/ownership.rs` (machine_dir variants ~126-170, `claim` ~430-475, `RoleClaim`, `probe`/`probe_tiers` ~620-650, `LockDirs::resolve`, tests)
- Modify: `crates/host/src/service/install.rs` (~512)
- Modify: `crates/host/src/service/log_file.rs` (~128-135)

**Interfaces:** Produces `TierDirs`, `claim_in`, `probe_tiers_in`, `machine_dir_from_program_data`, new `RoleClaim` fields (see Contracts).

- [ ] **Step 1: Failing tests** in `ownership.rs` tests:
  - `machine_dir_from_program_data_joins_iroh_tunnel_and_verifies`: `machine_dir_from_program_data(Ok("C:\\ProgramData".into()))` → `Some(MachineDir{ path ends_with "iroh-tunnel", verify: true })` (replaces `windows_machine_dir_is_programdata_child`).
  - `machine_dir_from_program_data_propagates_a_resolution_failure`: `Err(anyhow!("hr 0x80004005"))` → `Err`, `format!("{e:#}")` contains `%ProgramData%` and `0x80004005`.
  - `tier_dirs_resolution_fails_when_program_data_cannot_be_resolved`: `TierDirs::resolve_with(|| machine_dir_from_program_data(Err(anyhow!("hr"))))` → `Err` containing `%ProgramData%`. That `claim`/`probe` take or check no lock on this error is structural (`claim(role) = claim_in(role, TierDirs::resolve()?)`, `probe_tiers(role) = probe_tiers_in(role, &TierDirs::resolve()?)`) and is checked in code review, not by a file assertion.
  - `lock_dirs_resolution_fails_when_program_data_cannot_be_resolved`: `LockDirs::resolve_with(None, || machine_dir_from_program_data(Err(..)))` → `Err` containing `%ProgramData%`.
- [ ] **Step 2:** `cargo test -p iroh-tunnel-host --all-features --locked ownership::` → compile failure / FAIL.
- [ ] **Step 3:** Implement per Contracts: `machine_dir()` → `Result<Option<MachineDir>>` on the three cfg variants (test-utils: `user_dir()` error propagates; non-Windows production: `Ok(None)` for other OSes); `machine_dir_from_program_data` compiled on all targets (`#[cfg_attr(..., allow(dead_code))]` as before, with reason); `TierDirs::resolve`/`resolve_with` (machine resolver called before `user_dir`); `LockDirs::resolve_with`; `claim` resolves before taking any lock and delegates to `claim_in`; `probe_tiers_in`; `LockDirs::resolve` uses `machine_dir()?`; `install.rs`: `if let Some(machine) = crate::ownership::machine_dir()? {`; `log_file.rs`: `crate::ownership::machine_dir().ok().flatten()?` with a doc line that log selection is best-effort and ownership fails closed on claim.
- [ ] **Step 4:** Same test command → PASS; existing `lock_dirs_resolve_without_home_matches_the_ambient_dirs` adjusted to `machine_dir().ok().flatten().map(|m| m.path)`; "No machine lock file" behaviour (missing lock → `MachineClaim::Absent`) unchanged and still green.
- [ ] **Step 5:** Commit `fix(ownership): fail closed when the machine lock directory cannot be resolved`.

---

## Task 2: Mandatory, fail-closed owner records and identity check — implements 2

**Covers:** 2.1, 2.2, 2.3, 2.4, 2.5
**Difficulty:** Hard
**Blocked by:** Task 1

**Files:**
- Modify: `crates/host/src/ownership.rs` (`read_owner`, `write_owner`, `claim_in`, `publish_identity`, `Drop`, module docs, tests)
- Modify: `tests/role_ownership.rs` (new process test)
- Modify: `crates/host/src/service/mod.rs` (`#[cfg(test)] pub(crate) mod fixtures;`)

**Interfaces:** Consumes `TierDirs`, `claim_in`, `RoleClaim { dirs, holds_machine }` from Task 1. Produces `OwnerRead`, `read_owner_record`, fallible `write_owner` (Contracts).

- [ ] **Step 1: Failing unit tests** (all with temp `TierDirs { user, machine: Some(MachineDir{ path, verify: false }) }` and `claim_in`):
  - `write_owner_failure_names_the_record_path`: `write_owner(&missing_dir, Role::Serve, None, 0o600)` → `Err`, `{e:#}` contains `serve.owner.json`.
  - `machine_owner_record_is_world_readable_under_umask_077` (unix; runs in a child via `crate::service::fixtures::in_isolated_home` with `NAME = "ownership::tests::machine_owner_record_is_world_readable_under_umask_077"`; make `service/mod.rs` declare `pub(crate) mod fixtures;` (still `#[cfg(test)]`)): after `umask(0o077)` (with a `// SAFETY:` comment), `write_owner(dir, Role::Serve, None, 0o644)` → file mode & 0o004 != 0.
  - `read_owner_record_distinguishes_absent_present_and_corrupt`: no file → `Absent`; valid → `Present`; `"{not json"` → `Unreadable { path }` with `path` ending `serve.owner.json`.
  - `publish_identity_refuses_the_same_key_in_the_other_role`: serve `claim_in` + `publish_identity("K")`; access `claim_in` (same dirs) + `publish_identity("K")` → `CliError::Conflict` containing `same public identity`.
  - `publish_identity_refuses_when_the_running_other_role_record_is_corrupt`: serve claimed, then overwrite user `serve.owner.json` with `"{not json"`; access `publish_identity("A")` → `CliError::Conflict` whose message contains `identity cannot be determined` and `serve.owner.json`. Repeat with the corrupt record on the machine tier.
  - `publish_identity_refuses_when_the_running_other_role_record_is_unreadable` (unix): replace the record with a directory named `serve.owner.json` (read error other than NotFound) → Conflict naming the path.
  - `publish_identity_proceeds_while_the_other_role_has_no_record`: serve claimed (record has `node_id: None`) → access publish `Ok`; delete serve's record → access publish `Ok`; also a machine-tier lock held shared via `hold_for_test` without any record → `Ok`.
  - `publish_identity_ignores_a_corrupt_record_of_a_stopped_role`: no serve lock held, corrupt `serve.owner.json` present in both tiers → access publish `Ok`.
  - `publish_identity_ignores_a_machine_record_under_a_shared_hold`: `hold_for_test(Role::Serve, machine, LockKind::Shared)` + corrupt machine `serve.owner.json` → access publish `Ok`.
  - `held_exclusively_distinguishes_free_shared_and_exclusive`: missing file → `false`; shared hold → `false`; exclusive hold → `true`.
  - `a_leftover_temp_record_does_not_block_publishing`: pre-create `<user>/serve.owner.json.<pid>.tmp` → `claim_in` succeeds and the record names this pid.
  - `publish_identity_retries_before_refusing_on_an_unreadable_record` is not added as a timing test; the retry is a bounded loop asserted by the corrupt-record test still refusing.
  - `claim_fails_when_the_owner_record_cannot_be_written`: make `<user>/serve.owner.json` a non-empty directory so the rename fails → `claim_in` `Err` naming `serve.owner.json`, and the user lock is released afterwards (a second `claim_in` after removing the directory succeeds).
- [ ] **Step 2:** Run `cargo test -p iroh-tunnel-host --all-features --locked ownership::` → FAIL.
- [ ] **Step 3:** Implement per design decisions 1–3: `read_owner_record` (`NotFound` → `Absent`; other IO / serde error → `Unreadable` with context); `read_owner` = `Present` → `Some`, else `None`; `write_owner` opens the temp file with `base_options().write(true).create(true).truncate(true)` + mode (Unix `OpenOptionsExt::mode` then `set_permissions(mode)` on the open file to defeat umask), writes, renames, removes the temp on error, returns context `cannot publish owner record <path>`; `claim_in` and `publish_identity` use `?` (user tier `0o600`, machine tier `0o644`); `publish_identity` loops over `[(Tier::User, &dirs.user), (Tier::Machine, machine path)]`, skips tiers whose other-role lock is not held exclusively (`held_exclusively`), and matches `read_owner_record`; on `Unreadable` it retries up to 4 attempts at 25 ms (re-checking `held_exclusively`; a lock that became free → skip) before refusing. Update module docs: the record is mandatory to publish and the identity check fails closed.
- [ ] **Step 4:** Unit tests PASS.
- [ ] **Step 5: Process test** `access_is_refused_when_the_running_serve_record_is_corrupt` in `tests/role_ownership.rs`: start serve (`start_serve`), overwrite `world.home()/.local/state/iroh-tunnel/serve.owner.json` with `"{not json"`, spawn access with a different key → `assert_refused(wait_refused(..)?, "identity cannot be determined")` and stderr contains `serve.owner.json`. Run `cargo test --all-features --locked --test role_ownership` → PASS (incl. existing same-key test).
- [ ] **Step 6:** Commit `fix(ownership): refuse to start when the other role's identity cannot be determined`.

---

## Task 3: Verification — implements 3

**Covers:** 3.1, 3.2
**Difficulty:** Easy
**Blocked by:** Task 2

**Files:** none expected beyond fixes found by the gates (`crates/host/src/ownership.rs`).

- [ ] **Step 1:** `cargo fmt --check` → clean.
- [ ] **Step 2:** `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` → no warnings.
- [ ] **Step 3:** `RUBY=/opt/homebrew/Library/Homebrew/vendor/portable-ruby/current/bin/ruby cargo test --all-features --locked` → all pass.
- [ ] **Step 4:** `git diff <base>.. -- crates src | grep -nE '^\+.*(unwrap\(|panic!|todo!|unimplemented!|\b(print|println)!)'` and manually discard hits inside `#[cfg(test)]` modules → none remain.
- [ ] **Step 5:** Commit any fixes.

---

## Scenario → test

- Access started with the serve key → `tests/role_ownership.rs::roles_with_distinct_keys_coexist_but_a_shared_key_is_rejected` (existing) + `publish_identity_refuses_the_same_key_in_the_other_role`
- Running other role with a corrupt owner record → `tests/role_ownership.rs::access_is_refused_when_the_running_serve_record_is_corrupt` + `publish_identity_refuses_when_the_running_other_role_record_is_corrupt`
- Running other role with an unreadable owner record → `publish_identity_refuses_when_the_running_other_role_record_is_unreadable`
- Running other role that has not published yet → `publish_identity_proceeds_while_the_other_role_has_no_record`
- Corrupt record of a role that is not running → `publish_identity_ignores_a_corrupt_record_of_a_stopped_role`
- Corrupt machine record while the machine lock is held only shared → `publish_identity_ignores_a_machine_record_under_a_shared_hold` + `held_exclusively_distinguishes_free_shared_and_exclusive`
- Owner record round trip → `owner_record_round_trips_without_secret_material` (existing, updated signature)
- Owner record cannot be written → `write_owner_failure_names_the_record_path` + `claim_fails_when_the_owner_record_cannot_be_written`
- Machine record readable under a restrictive umask → `machine_owner_record_is_world_readable_under_umask_077`
- Machine lock location → `machine_dir_from_program_data_joins_iroh_tunnel_and_verifies` (Windows) + existing platform constants
- System lock held, user start → existing `tests/role_ownership.rs` system-tier test (unchanged)
- User running, system start → existing `machine_lock_exclusive_then_shared_is_refused`
- Windows user task while the system service runs → existing Windows e2e coverage (unchanged)
- Windows ProgramData cannot be resolved → `machine_dir_from_program_data_propagates_a_resolution_failure` + `tier_dirs_resolution_fails_when_program_data_cannot_be_resolved`
- Install lock directories when ProgramData cannot be resolved → `lock_dirs_resolution_fails_when_program_data_cannot_be_resolved`
