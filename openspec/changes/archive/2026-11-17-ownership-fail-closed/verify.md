# Verification Report

**Change**: `ownership-fail-closed`
**Verified at**: `2026-11-17 03:08 UTC`
**Iteration**: `1`
**Verifier**: parent agent (inline evidence collection in the worktree `/Users/naicoi/Projects/iroh/iroh-tunnel.ownership-fail-closed`; no verifier child dispatched)

---

## 1. Structural Validation (`openspec validate <change>`)

- [x] The command passes for this change

**Result**:

```text
$ openspec validate ownership-fail-closed --strict
Change 'ownership-fail-closed' is valid   (exit 0)
```

---

## 2. Task Completion (`tasks.md`)

- [x] Every box is ticked, or each open box has a recorded reason

`openspec instructions apply` progress: `{"total":12,"complete":12,"remaining":0}`, state `all_done`; `grep -c "\- \[ \]" tasks.md` → `0`.

| Open task | Reason | Blocks archive? |
|---|---|---|
| — | — | — |

---

## 3. Requirement Evidence

- [x] ADDED / MODIFIED: implementation evidence (file / line)
- [x] REMOVED: none in this change
- [x] RENAMED: none in this change

| Requirement | Operation | Status | Evidence (file:line) or reason |
|---|---|---|---|
| Same public identity rejected across roles | MODIFIED | Pass | `crates/host/src/ownership.rs:785-810` (`publish_identity`: publish first, then inspect user + machine tiers; same-key conflict text kept at :797); `:821-850` (`running_owner`: only while `held_exclusively`, `Absent`/no node id → proceed, `Unreadable` retried 4×25 ms then `start_refused` "identity cannot be determined (<path>: …)"); `:454-463` (`held_exclusively`: shared try-lock on read-only open, missing file → false); `:418-437` (`read_owner_record` tri-state) |
| Owner record is informational and public only | MODIFIED | Pass | `:478-503` (`write_owner` returns `Result`, context "cannot publish owner record <path>", temp removed on failure); `:509-527` (`write_record_file`: create+truncate, never `create_new`; Unix mode set at open and via `set_permissions` on the open file); `:466-469` (user 0o600, machine 0o644); `:763-771` (`record_dirs`: machine tier only when held exclusively); `:645-650` (`claim_in` propagates publish errors; locks released by drop); liveness still from the lock (`probe_tiers_in` :880-889, unchanged decision) |
| Machine-tier lock between system service and user processes | MODIFIED | Pass | `:171-176` (`machine_dir_from_program_data`, context "cannot resolve the machine lock directory (%ProgramData%)"); `:186-190` (Windows production `machine_dir` delegates); `:144-158`, `:200-212` (other variants return `Result<Option<_>>`, `None` only where no tier); `:229-244` (`TierDirs::resolve(_with)`, machine first); `:600-602` (`claim` resolves before any lock); `:871-873` (`probe_tiers` resolves first); `:906-932` (`LockDirs::resolve(_with)`); `crates/host/src/service/install.rs:512` (`machine_dir()?`); `crates/host/src/service/log_file.rs:134` (best-effort, by design); missing machine lock file still `MachineClaim::Absent` (`:728-730`) |

---

## 4. Scenario Coverage

- [x] Every scenario maps to a test
- [x] The project test command was run fresh in this step

**Test command**: `RUBY=/opt/homebrew/Library/Homebrew/vendor/portable-ruby/current/bin/ruby cargo test --all-features --locked` (cwd: worktree, HEAD `63fc358`)
**Result**: final run exit 0 — 39 test result lines, 964 passed, 0 failed, 4 ignored. Also `cargo fmt --check` OK and `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` "No issues found".

Run history in this step (all fresh, same HEAD):
1. Run 1: exit 101 — 1 failure, `tests/udp_tunnel.rs:866` `a_flooding_flow_does_not_stop_another_flow` (assertion on a reply under a 5000-datagram flood). The test uses `iroh_tunnel_host::testing::run_unowned` (no ownership path) and no file of that test or of `crates/engine` changed in this range; rerun alone 5/5 passed.
2. Run 2: aborted by the 1900 s tool timeout while `udp_tunnel` was still running (`quarantine_holds_the_slot_and_then_releases_it`, which waits up to 60 s by design); 29 suites completed with 245 passed, 0 failed. `cargo test --test udp_tunnel` alone: 14 passed, 0 failed.
3. Run 3 (recorded result): exit 0, 964 passed, 0 failed.

| Scenario | Test |
|---|---|
| Access started with the serve key | `tests/role_ownership.rs:241` `roles_with_distinct_keys_coexist_but_a_shared_key_is_rejected`; `ownership.rs:1510` `publish_identity_refuses_the_same_key_in_the_other_role` |
| Running other role with a corrupt owner record | `tests/role_ownership.rs:276` `access_is_refused_when_the_running_serve_record_is_corrupt`; `ownership.rs:1521` `publish_identity_refuses_when_the_running_other_role_record_is_corrupt` (user and machine tier) |
| Running other role with an unreadable owner record | `ownership.rs:1543` `publish_identity_refuses_when_the_running_other_role_record_is_unreadable` |
| Running other role that has not published yet | `ownership.rs:1558` `publish_identity_proceeds_while_the_other_role_has_no_record` |
| Corrupt record of a role that is not running | `ownership.rs:1579` `publish_identity_ignores_a_corrupt_record_of_a_stopped_role` |
| Corrupt machine record while the machine lock is held only shared | `ownership.rs:1591` `publish_identity_ignores_a_machine_record_under_a_shared_hold`; `:1603` `held_exclusively_distinguishes_free_shared_and_exclusive` |
| Owner record round trip | `ownership.rs:1127` `owner_record_round_trips_without_secret_material` |
| Owner record cannot be written | `ownership.rs:1444` `write_owner_failure_names_the_record_path`; `:1630` `claim_fails_when_the_owner_record_cannot_be_written`; `:1615` `a_leftover_temp_record_does_not_block_publishing` |
| Machine record readable under a restrictive umask | `ownership.rs:1475` `machine_owner_record_mode_ignores_the_creation_mode` (fork-free stand-in, see WARNING 2); `:1482` `user_owner_record_is_private_even_over_a_wider_temp` |
| Machine lock location | `ownership.rs:1138` `machine_dir_from_program_data_joins_iroh_tunnel_and_verifies` (Windows path); Linux/macOS paths are constants in `machine_dir` (`:200-212`), unchanged |
| System lock held, user start | `tests/role_ownership.rs:505` `system_tier_lock_refuses_user_start_and_blocks_a_system_start` |
| User running, system start | `ownership.rs:1177` `machine_lock_exclusive_then_shared_is_refused`; `tests/role_ownership.rs:505` |
| Windows user task while the system service runs | Not verified here: Windows-only e2e (`tests/service_system_windows.rs`), not runnable on macOS; unchanged by this change |
| Windows ProgramData cannot be resolved | `ownership.rs:1151` `machine_dir_from_program_data_propagates_a_resolution_failure`; `:1160` `tier_dirs_resolution_fails_when_program_data_cannot_be_resolved` |
| Install lock directories when ProgramData cannot be resolved | `ownership.rs:1168` `lock_dirs_resolution_fails_when_program_data_cannot_be_resolved` |

---

## 5. Design and Contract Coherence (warning only)

- [x] Decisions in design.md match the code (decisions 1–6; see section 3 evidence)
- [x] Contracts in plan.md, with the Contract deviations recorded in apply.md, match the code

| Contract | Result | Compared with |
|---|---|---|
| `ownership::machine_dir` (modified) | Matched | `ownership.rs:144`, `:186`, `:200` |
| `machine_dir_from_program_data` (new) | Matched | `ownership.rs:171-176` |
| `ownership::TierDirs` (new) | Matched | `ownership.rs:217-244` |
| `claim` / `claim_in` | Matched | `ownership.rs:600-651` |
| `probe_tiers_in` (new) | Matched | `ownership.rs:871-889` |
| `RoleClaim` fields | Matched | `ownership.rs:583-590` |
| `RoleClaim::publish_identity` behaviour | Matched | `ownership.rs:785-810` |
| `OwnerRead` / `read_owner_record` | Matched | `ownership.rs:407-437` |
| `write_owner(dir, role, node_id, mode) -> Result<()>` | Matched | `ownership.rs:478-503` |
| `LockDirs::resolve` / `resolve_with` | Matched | `ownership.rs:906-932` |
| `held_exclusively` (new) | Matched | `ownership.rs:454-463` |
| Identity-unknown refusal message (public stderr text) | Matched | `ownership.rs:840-845`: "the <role> role is running but its identity cannot be determined (<path>: <cause>); <stop hint>" via `start_refused` |
| apply.md deviation: umask child test replaced by fork-free tests | Matched | `ownership.rs:1475`, `:1482`; `service/mod.rs` keeps `mod fixtures` private (unchanged) |

**Drift warnings** (non-blocking):

- None.

---

## 6. Clean Tree and Commit Range

- [x] Working tree is clean (`git status --porcelain` → empty before this receipt)

**Commit range**: `2d408796ae4fff89c197bcb7fbe04b27c81b09af..63fc358` (implementation `2d40879..b81db52` plus the apply receipt `63fc358`)

---

## Overall Decision

- [ ] PASS: proceed with `/opsx-continue` to finalize
- [x] PASS_WITH_WARNINGS: proceed, noting: one scenario is not verified on this host (Windows-only e2e), Windows-only code was not compiled here, the umask scenario is pinned by a stand-in test, and one unrelated UDP test failed once under full-suite load.
- [ ] FAIL: fix the cause, then re-apply (iteration N+1)

### Issues

**CRITICAL**: none.

**WARNING**
1. Windows-only branches (`ownership.rs:186-190`, `service/install.rs:512`) and the scenario "Windows user task while the system service runs" were not compiled or run on this macOS host. Recommendation: rely on the PR's Windows `ci-ok` job (and `e2e.yml` on main) before merging; treat a Windows compile failure as a fix iteration.
2. "Machine record readable under a restrictive umask" is covered by `machine_owner_record_mode_ignores_the_creation_mode`, which proves the final mode is independent of the creation mode but does not set `umask(077)`. Recommendation: add a real-umask test in a dedicated integration test binary (its own process, no concurrent lock tests) in a follow-up.
3. `tests/udp_tunnel.rs:866` `a_flooding_flow_does_not_stop_another_flow` failed once under full-suite load (passed 5/5 alone and in a later full run); unrelated to this change (no ownership path, file untouched). Recommendation: open a flaky-test issue for the UDP flood assertion.

**SUGGESTION**
1. Update the `Evidence:` lines of the three modified requirements (`src/ownership.rs:436-470` etc.) to `crates/host/src/ownership.rs` with current lines when syncing/archiving.
2. Follow-ups parked in apply.md: `held_exclusively` I/O errors are not classified as Conflict; no test for a lock freed during the retry; test-utils `machine_dir` error path untested; `in_isolated_home` child tests may inherit lock fds; known limitation user-then-system same key across accounts.

**Next step**:

Run `/opsx-continue` with the worktree as working directory to create the finalize artifact, then `/opsx-archive`.
