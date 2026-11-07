# Apply Receipt

> Generated at the end of the apply phase to mark code-implementation
> complete and provide verify with the state it needs.
> Overwritten on each apply iteration; iteration counter grows.

**Change**: `fix-launchd-user-install`
**Iteration**: `1`
**Applied at**: `2026-11-07 11:47`
**Executor**: `subagent-driven-development` (pi-subagents: `superspec.implementer` per task, `superspec.task-reviewer` per task and for the final whole-branch review)

---

## Workspace

- **Worktree path**: `/Users/naicoi/Projects/iroh/iroh-tunnel.fix-launchd-user-install-impl`
- **Worktree branch**: `fix-launchd-user-install`
- **Feature branch**: `fix/launchd-user-install`
- **Base branch**: `main`

---

## Commits

- **Range**: `7f7d022..97a1b19` (implementation and tick commits; this receipt is committed on top)
- **Count**: `17`

Plan tasks map to commits as follows:
- Task 1 (launchd enable): `7ddf46d`
- Task 2 (DOCTYPE, restart guards): `1d7d219`, `40a3850`
- Task 4 (tier-scoped probe): `1aaf1cb`
- Task 5 (cross-scope install conflict): `7ea83bb`
- Task 3 (bootstrap hint): `8369123`
- Task 6 (Conflict prefix): `9a8bcdd`
- Task 7 (docs): `5c70886`, `fd77ee8`
- Final-review fix (stop and remove guidance in the refusal): `97a1b19`

Verification at HEAD 97a1b19:
- macOS (Rust 1.99): `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features --locked -- -D warnings` and `cargo test --workspace --all-features --locked` all pass.
- Linux (Docker `rust:1.91-bookworm`, MSRV): clippy is clean and 600 tests pass. Three failures are in code this change does not touch: two `update` tests fail with an ETXTBSY flake and pass on re-run, and `trust::an_owner_that_is_neither_root_nor_trusted_is_refused` fails because the container runs as root.
- `openspec validate fix-launchd-user-install` reports the change is valid.

---

## Tasks

- **Completed**: `26 of 28` checkboxes in tasks.md flipped to `- [x]`
- **Remaining**: `5.3`, `5.4`. Both are manual macOS verifications against real launchd state, and they are left for `/opsx-verify`. Plan Task 8 was not run during apply, because it changes launchd state outside the worktree and needs the operator's consent. The host currently runs a real root `iroh-tunnel serve` system service. That service suits 5.4 (the conflict refusal), but it must be stopped, or another role used, before 5.3 (the disabled-label install on a host with no conflicting system service).

---

## Contract deviations

- `ServiceNote::LaunchdBootstrapHint` (public): the `target` field was removed, leaving `{ domain, plist }`, as the plan proposed. The user approved this through the Plannotator Inbox (msg_01M6T1NR8DBMNFXN8PCQM6J5F4).
- `CliError::Conflict` (public, user-visible): Display is now `"{0}"`. The `start refused: ` prefix moved to the start call sites through `ownership::start_refused`, which replaces the private `conflict` helper. The user approved this through the Plannotator Inbox (msg_01M6T1NR8DBMNFXN8PCQM6J5F4).
- `ownership::stop_hint_scoped` (internal, new): `pub(crate) fn stop_hint_scoped(role, scope_flag) -> String`. It was added during the final-review fix so the install refusal names how to stop the conflicting holder (`service stop --system|--user`) as well as how to remove it, which the spec requires. The start-path `stop_hint` text is unchanged.
- `ownership::LockDirs::resolve`, `LockResolver`, `tier_liveness` (internal, temporary): between Task 4 and Task 5 these carried plain `#[allow(dead_code)]` instead of `cfg_attr(not(test), …)`. Task 5 removed the attributes, so nothing remains at HEAD.

---

## Next step

Run `/opsx-verify`. It includes the manual macOS checks 5.3 and 5.4, recorded in `verify.md`.
