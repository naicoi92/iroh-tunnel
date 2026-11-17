## Context

`crates/host/src/ownership.rs` implements role ownership with two lock tiers (user, machine) and an informational `<role>.owner.json` per tier. Liveness is the kernel lock; the record supplies pid and node id for messages and the cross-role identity check (`RoleClaim::publish_identity`). Today `write_owner` returns nothing (errors are logged at debug), `read_owner` collapses "absent", "unreadable" and "corrupt" into `None`, and `machine_dir()` collapses a Windows `program_data_dir()` error into `None`. `claim` resolves the machine directory only after it holds the user lock.

The behaviour and alternatives are settled in `brainstorm.md`; this document covers how.

## Goals / Non-Goals

**Goals:**
- Keep the invariant "no two running processes of different roles share a public identity" enforced even when a record is unreadable or a write fails.
- Make the Windows machine tier's absence mean only "this platform has none".
- Test both on every OS through seams, plus one process-level test.

**Non-Goals:**
- Identity-keyed locks, record format changes, exit code changes, status output redesign, unrelated refactors.

## Decisions

1. **Tri-state owner read.** Add a private `OwnerRead { Absent, Present(OwnerRecord), Unreadable { path, error } }` returned by `read_owner_record(dir, role)`. `NotFound` → `Absent`; any other read error or JSON error → `Unreadable`. The existing `read_owner(..) -> Option<OwnerRecord>` stays as the informational view (messages, `status`, `Drop`) built on top of it, so `Liveness` and the public status types are unchanged. Alternative: change `Liveness::Running.owner` to carry the tri-state — rejected, it changes a public type for a decision only `publish_identity` makes.

2. **Identity check reads records directly, not through `Liveness`.** `publish_identity` iterates the other role's tier directories; for each tier whose other-role lock is held **exclusively** (tested by a shared `try_lock` on a read-only open: `WouldBlock` means an exclusive holder; user-tier locks are always exclusive, machine-tier shared holders never write a machine record) it calls `read_owner_record` and decides: `Present` with equal node id → existing "same public identity" conflict; `Unreadable` (after a bounded retry of four attempts at 25 ms, re-checking `held_exclusively` each time, which absorbs a reader racing a Windows rename, a writer mid-publish and an elevated status probe's microsecond exclusive hold) → `CliError::Conflict` "start refused: the <other> role is running but its identity cannot be determined (<path>: <error>); …"; `Absent` or `Present` without node id → continue. A tier whose lock is not held exclusively is never read, so a crash leftover (including a crashed system service's machine record while user processes still hold the machine lock shared) never blocks a start. Helper: `fn held_exclusively(path: &Path) -> Result<bool>` (missing file → `false`).

3. **Mandatory publish.** `write_owner` returns `Result<()>` with context naming the record path. `claim` propagates it (the locks are released by drop on error); `publish_identity` propagates it before probing. The temp file (`<record>.<pid>.tmp`) is opened with `create(true).truncate(true)` on `base_options()` (no symlink follow), never `create_new`, so a temp left by a `kill -9` with a reused pid (common in containers) cannot block later starts; it is removed on failure (best-effort, logged). Records in the machine tier are written with mode `0o644` set on the open file (umask-independent, like the machine lock), records in the user tier with `0o600`, so a non-elevated process can always read the system holder's record (Unix; on Windows the record inherits the machine directory DACL, Users read) (otherwise decision 2 would refuse every user start while the system service runs under a restrictive umask).

4. **Directory resolution before locking, through one seam.** A private `TierDirs { user: PathBuf, machine: Option<MachineDir> }` with `TierDirs::resolve() -> Result<TierDirs>` (production: `user_dir()?` and `machine_dir()?`). `claim(role)` becomes `claim_in(role, TierDirs::resolve()?)` and `probe_tiers(role)` becomes `probe_tiers_in(role, &TierDirs::resolve()?)`; `RoleClaim` stores its `TierDirs` so `publish_identity` inspects the same directories it claimed in. Resolving first means a resolution error never leaves a half-taken claim. Tests call `claim_in`/`probe_tiers_in` with temp directories. Resolution itself has a seam: `TierDirs::resolve_with(machine: impl FnOnce() -> Result<Option<MachineDir>>)` and `LockDirs::resolve_with(home, machine)`; `resolve()` passes `machine_dir`. Tests pass `|| machine_dir_from_program_data(Err(..))` to prove claim, probe, status (`probe` → `TierDirs::resolve`) and install (`LockDirs::resolve`) receive the error.

5. **`machine_dir() -> Result<Option<MachineDir>>`.** The Windows production body delegates to a platform-neutral `machine_dir_from_program_data(Result<PathBuf>) -> Result<Option<MachineDir>>`, compiled on every target (replacing `windows_machine_dir`), which adds context "cannot resolve the machine lock directory (%ProgramData%)". This is the injected resolver the issue asks for: the `test-utils` build replaces `machine_dir` entirely, so the Windows-specific branch is otherwise untestable under `--all-features`. Unix and test-utils bodies return `Ok(..)`. Callers: `TierDirs::resolve`, `LockDirs::resolve` and `install.rs` use `?`; `machine_log_dir` maps `Err` to `None` (log location only; the service still fails closed when it claims).

6. **No-panic policy.** New production code uses no `unwrap`/`expect`/`panic!`/indexing; errors carry path context; tests may unwrap.

## Risks / Trade-offs

- [User-session process of role A, then system service of role B with the same key, is not detected (pre-existing; per-user records of other accounts are not visible)] → stated as a known limitation in the spec; follow-up issue.
- [A crash-left valid record with the same node id, read while an elevated status probe holds the free lock exclusively for microseconds, refuses once (pre-existing false-refusal class, never a missed conflict)] → accepted; a retry of the start succeeds.
- [`service status` maps a probe error to `Fact::Unavailable` rather than failing] → unchanged; the resolution error is still reported.
- [Windows `rename` over a record a prober has open] → std opens files with `FILE_SHARE_DELETE`, so `MoveFileExW(REPLACE_EXISTING)` succeeds; a failure now refuses the start, which is the intended fail-closed outcome.

- [A host whose user state directory is not writable now refuses to start] → intended fail-closed behaviour; the error names the path.
- [Windows host where `SHGetKnownFolderPath` fails now refuses start/status/install] → intended; the error carries the HRESULT context.
- [Windows code path not exercised by local macOS runs] → the resolver seam is platform-neutral and unit-tested; the Windows production branch is compiled only by the e2e/release workflows (no clippy `-D warnings` there).

## Migration Plan

None: no data or format migration. Rollback is a revert.

## Open Questions

None.
