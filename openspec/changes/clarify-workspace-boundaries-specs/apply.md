# Apply Receipt

> Generated at the end of the apply phase to mark code-implementation
> complete and provide verify with the state it needs.
> Overwritten on each apply iteration; iteration counter grows.

**Change**: `clarify-workspace-boundaries-specs`
**Iteration**: `1`
**Applied at**: `2026-11-10T00:35Z`
**Executor**: `subagent-driven-development` (implementer: `general-purpose`; task, final and re-reviewer: `code-reviewer`; Task 4 verification run by the controller)

---

## Workspace

- **Worktree path**: `/Users/naicoi/Projects/iroh/iroh-tunnel.clarify-workspace-boundaries-specs`
- **Worktree branch**: `clarify-workspace-boundaries-specs`
- **Feature branch**: `spec/clarify-workspace-boundaries-specs`. It was created from `main` with `wt` before step 0, and the scaffold commit `f9807db` is on it. The artifacts were moved there so they are not committed on `main`.
- **Base branch**: `main`

---

## Commits

- **Range**: `f9807db..4354df6`. This receipt commit follows `4354df6`.
- **Count**: `12` before the receipt commit
  - `7b32532` test(workspace): require engine dependencies to inherit from the workspace
  - `b6694ea` test(workspace): cite the engine dependency requirement on the helper
  - `fbd9d83` docs(openspec): tick task group 1
  - `1b4a57f` docs(openspec): write Purpose for cli-entrypoints, config-management-commands, connection-path-status, frontend-independent-management
  - `ba54cd2` docs(openspec): write Purpose for install-release-packaging, iroh-endpoint-relay, role-ownership-shutdown, self-update, system-service-management
  - `8d4b990` docs(openspec): write Purpose for tcp-tunnel-forwarding, tunnel-configuration, udp-datagram-forwarding, windows-service-trust, workspace-boundaries
  - `4460c5a` docs(openspec): tighten self-update and shutdown Purpose wording
  - `0cc57a8` docs(openspec): tick task group 2
  - `5ae9f36` docs(contributing): drop the archived migration-map link
  - `b428f2c` docs(openspec): tick task group 3
  - `eebe70f` docs(openspec): use the role status command in the raw JSON scenario
  - `4354df6` docs(openspec): tick task group 4 verification

---

## Tasks

- **Completed**: `12 of 13` checkboxes in tasks.md flipped to `- [x]`
- **Remaining**: `4.4`. Opening the D6 follow-up issue needs the user's approval in chat, because it is a side effect outside the worktree. The draft is below and in `.superpowers/sdd/plan/issue-draft.md`.

---

## Contract deviations

- none

---

## Lanes

| Task | Lane branch | Lane range | Review | Merge commit |
|---|---|---|---|---|
| Task 1 | `sequential` | `f9807db..b6694ea` | Approved after fix round 1 (plan-mandated requirement citation on the helper doc comment) | n/a (sequential) |
| Task 2 | `sequential` | `fbd9d83..4460c5a` | Approved after fix round 1 (`self-update` Purpose overclaimed verification and managed-install refusal; shutdown wording) | n/a (sequential) |
| Task 3 | `sequential` | `0cc57a8..5ae9f36` | Approved (clean) | n/a (sequential) |
| Task 4 | `sequential` (controller) | no code commits; tick `4354df6` | Verification evidence below | n/a (sequential) |

**Final whole-branch review.** The review over `f9807db..b428f2c` (code-reviewer) said "With fixes", with one Important finding: the delta scenario "Raw JSON passthrough" used the nonexistent command `iroh-tunnel status --json`. The fix wave `b428f2c..eebe70f` changed it to `iroh-tunnel serve status --json` and kept the scenario heading. The scoped re-review marked the finding ADDRESSED: ready to merge, Yes.

**Rulings made by the controller:**
- Run T1–T4 sequentially in one worktree, because parallel lanes cost more setup than they save. Cost if wrong: minutes.
- The controller ran T4 itself, and 4.4 is not filed without approval. Cost if wrong: none.
- The Task 1 fix re-review was done by the controller, because the fix diff only changed a doc comment. Cost if wrong: a wording nit.

---

## Evidence summary

- **OpenSpec**:
  - `openspec validate clarify-workspace-boundaries-specs --strict`: valid.
  - `openspec validate --specs`: `Totals: 16 passed, 0 failed (16 items)`, with 0 Purpose placeholder warnings (baseline 14).
- **Gates** (Rust 1.91, macOS, controller run at `b428f2c`):
  - `fmt --check`: pass.
  - `clippy --workspace --all-targets --all-features --locked -D warnings`: pass.
  - `RUSTDOCFLAGS="-D warnings" doc --workspace --no-deps --locked`: pass.
  - `test --workspace --all-features --locked`: **778 passed, 0 failed, 4 ignored**.
  - The fix commit `eebe70f` only touches a delta spec. `cli_golden::status_json_forwards_file_bytes` was re-run and passes.
- **E2E (real debug binary)**:
  - `serve config check --config <port = 0>`: `error: config error: invalid config at <path>: invalid port: must be 1-65535`, exit 2.
  - `serve config list --config <malformed TOML>`: `error: config error: failed to parse config <path> (line 1): …`, exit 2.
  - `serve run --config <secret_key = "%%%">`: `error: invalid secret_key: not valid base64 …`. This is not a parse error, so the key is checked at resolution.
  - D6 reproduction: `serve run --config` with `port = 0` gives `error: invalid port: must be 1-65535`, exit 1, with no path. With malformed TOML it exits 1.

---

## Follow-up issue draft (task 4.4)

Title: `serve run --config` exits 1 on configuration errors and omits the path on validation errors.

`iroh-tunnel <role> run --config <file>` exits 1 instead of 2 for a malformed TOML file or a schema violation such as `port = 0`. The validation message also does not name the file. This contradicts `cli-entrypoints` "Exit status mapping" and `tunnel-configuration` "Role configuration documents are loaded from TOML and validated". `serve config check` and `serve config list` handle the same files correctly, with exit 2 and the path. The full reproduction is in `.superpowers/sdd/plan/issue-draft.md`.

---

## Next step

Run `/opsx-verify clarify-workspace-boundaries-specs`. Before that, decide whether task 4.4 opens the follow-up issue.
