# Apply Receipt

> Generated at the end of the apply phase to mark code-implementation
> complete and provide verify with the state it needs.
> Overwritten on each apply iteration; iteration counter grows.

**Change**: `windows-system-service`
**Iteration**: `1`
**Applied at**: `2026-11-08 04:05`
**Executor**: `subagent-driven-development` (implementer: `superspec.implementer`; per-task review: `superspec.task-reviewer`, and from Task 13C on `code-reviewer`; final whole-branch review: `code-reviewer`). Late Task 13 work (13C fix round 1, 13D, all review fix rounds after 13C) was implemented by the controller after an implementer run aborted and the owner asked to finish faster (see Rulings).

---

## Workspace

- **Worktree path**: `/Users/naicoi/Projects/iroh/iroh-tunnel.windows-system-service`
- **Worktree branch**: `windows-system-service`
- **Feature branch**: `feat/windows-system-service`
- **Base branch**: `main`

---

## Commits

- **Range**: `fa85e51..HEAD` (the last code commit is `506d0c4`; this receipt is committed on top)
- **Count**: `77` before this receipt

---

## Tasks

- **Completed**: `31 of 31` checkboxes in tasks.md flipped to `- [x]`
- **Remaining**: none

Verification at `506d0c4` (fresh):

**Local (macOS)**
- `cargo test --workspace --all-features --locked`: 735 passed.
- `cargo fmt --check`: clean.
- `cargo clippy -D warnings`, native and `cargo xwin` `x86_64-pc-windows-msvc`, each with and without `--all-features`: clean.
- `actionlint`, `bun run check-docs`, `openspec validate windows-system-service --strict`: clean.

**Windows (CI)**
- e2e run https://github.com/naicoi92/iroh-tunnel/actions/runs/42412502192 is all green.
- Step "Service lifecycle (Windows Service)":
  - `restricted_token_windows`: 2
  - `installer_acl_windows`: 5
  - `service_system_windows`: 23, in 182 s, against the real SCM on the elevated runner
- Step "Host unit tests (elevated Windows)": 348.
- The Linux, macOS and container cells are green.

**Reviews**
- Every task was reviewed.
- The task 13 fix rounds:
  - B: 4 rounds
  - C: 2 rounds
  - D: 1 round
- The final whole-branch review returned NEEDS_FIXES, with 2 Important findings, both fixed in `506d0c4` and proved by the new E2E `first_start_creates_the_machine_dir_and_logs`:
  - I-1: `run` read the config with a plain open before the trust check.
  - I-2: the first service start on a fresh host wrote no log.

### Product defects the real-SCM E2E found, and fixed

- `is_elevated` treated a token whose Administrators group is deny-only as elevated. It now also requires `CheckTokenMembership`.
- The pin in the service Description was not in the comment-line form `Pin::find` parses, so every reinstall saw an unknown owner. It is now written as `# <record>`.
- The binary refusal named only the offending ancestor. It now names the service binary and keeps exit code 3.
- I-1 and I-2 above.

### Rulings (controller)

- The 13C fix round 1, 13D, and the later fix rounds were implemented by the controller.
  - Reason: an implementer run aborted ("This operation was aborted"), and the owner asked to finish faster in premium mode.
  - Each was still reviewed by a fresh reviewer seat and proved on Windows CI.
  - Cost if wrong: one extra fix round.
- From 13C on, reviews used `code-reviewer`, because the `superspec.task-reviewer` invocation shape was unavailable in this harness.
  - Cost if wrong: none; the reviews were full spec and quality reviews.
- `invalid_config_never_runs` asserts any service-specific failure, not exit 2.
  - TOML syntax errors are a plain error (exit 1) on every platform; this predates the branch.
  - The spec scenario fixes no exit code.
  - Cost if wrong: one test line.
- The host lib stays in the e2e Windows job as its own step: 5 host unit tests are gated on `IROH_TUNNEL_E2E_SERVICE`.

### Deferred minors (can wait)

- **Non-elevated runs and `%ProgramData%\iroh-tunnel`.** A non-elevated run verifies an existing directory there. A local user who pre-creates it with a weak ACL makes other users' runs exit 3 until an admin deletes it. This follows the spec ("never repaired") but is a local denial of service, and it is worth an owner note.
- **`CappedLog` reopen and rotate** do not re-verify the machine directory.
- **The normal `run_to_code` path** drops the runtime instead of using `shutdown_background`.
- **`install.ps1` test knob.** `IROH_TUNNEL_TEST_ASSUME_NOT_ELEVATED` is honoured in the production script. It can only disable protection, under the admin's own environment.
- **TOML syntax errors** exit 1, not 2. This is on all platforms and predates the branch.
- **Test-only gaps.**
  - `acl_state` is a fingerprint, not full SDDL.
  - Some diagnostic details are not asserted: Everyone and rights in the config refusal, the junction path, the owner of the machine dir.
  - Machine dir case (a) is not created by a non-administrator.
  - A failure of `net user /delete` in cleanup is silent.
  - Restricted-token helper:
    - possible handle-inheritance race under parallel tests;
    - the `CreateProcessWithTokenW` fallback is unverified;
    - `env_clear` is ignored;
    - `LAUNCH_API` is a global.
- **Installer.**
  - A relative or trailing-separator `-BinDir` fails closed on the elevated path; the fix would be `GetFullPath`.
  - The PS 5.1 `CreateDirectory(path, sec)` path is not exercised in CI.
- **Pre-existing Linux flake.** `tests/update.rs` ETXTBSY.
- **Not covered by the final review seat**, which ran out of time: `trust_windows/model.rs`, `ace.rs`, `windows_scm_impl.rs` and `update/replace.rs`. Each passed its own task review. A focused `security-auditor` pass on `model.rs` and `ace.rs` is suggested before release.

---

## Contract deviations

Internal, recorded in plan.md Contracts:
- `components(&str)`
- `machine_dir() -> Option<MachineDir>`; `claim_machine(..., verify)`
- `map_create_error`
- the Clock seam, `ScmStatus` and `stop_and_delete`
- `WinTarget` and `WinRequest`; the retarget logic moved to `pin.rs` (`retarget_decision_for`)
- `system_service_name`, `ScmMachine`
- `run_to_code` takes `log_errors`
- `ownership::ensure_machine_dir` (new, final review I-1/I-2)
- `config::require_present` (new, final review I-1/I-2)

Public:
- Owner decision, Task 11b: the `update` locked hint names both `serve` and `access service stop --system`.
- Owner decision, Task 12: `install.ps1` protects a new `-BinDir` only when elevated; the `install-release-packaging` spec was amended.
- `is_elevated` on Windows also requires `CheckTokenMembership(Administrators)` (Task 13A defect fix).
- The service `Description` holds `# ` + the pin record (Task 13B defect fix).

---

## Next step

Run `/opsx-verify`.
