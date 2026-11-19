# Windows user-tier stop limitation Implementation Plan

> Executed via `/opsx-apply`, which chooses the executor.
> Spec reference: this change's `specs/` and `design.md`.

**Goal:** Make docs and the Windows lifecycle test state, and check, that a Windows user-tier `service stop` ends the role without draining and leaves crash-style leftovers that neither read as live nor block the next start.

**Architecture:** No runtime change. `docs/services.md` gains the limitation and the system-tier pointer. `tests/service_user_windows.rs` keeps reporting the drain outcome, and asserts the stale-snapshot warning and a start with leftovers in place instead of deleting them first.

**Tech Stack:** Markdown; Rust 2021 integration test (Windows only, opt-in `IROH_TUNNEL_E2E_SERVICE=1`).

## Global Constraints

- No change under `src/` or `crates/`; no new stop command or control channel.
- The `/End` drain outcome stays reported (`EVIDENCE` line), never asserted: a future Windows that drains must not fail the test.
- Repository text (docs, comments, commit messages) in English.
- MSRV 1.91; `cargo clippy ... -D warnings` and `cargo fmt --check` clean.

## Review Focus

- A leftover snapshot from an earlier test in the shared per-user state dir: waits must key on this fixture's node id and on a pid different from the stopped one, never on "a status file exists".
- `/End` returns before the process exits: every check after stop must wait for the old pid to be gone first.
- The snapshot may be absent after stop (a drained exit): the stale assertion runs only when the file is still present.
- `serve status` must run in the user tier (through `fx.run`, restricted token on an elevated runner), or it reads the wrong tier.

---

## Contracts

None – documentation and test-only change; no type, signature, CLI argument, config key or file format is added or modified.

---

## Task 1: Document the Windows user-tier stop limitation — implements 1

**Covers:** 1.1, 1.2
**Difficulty:** Easy
**Blocked by:** none

**Files:**
- Modify: `docs/services.md` (section `## Windows` → `### User tier`, the bullet "`stop` ends the task; `restart` waits up to 15 seconds ...")

- [ ] **Step 1:** Replace that bullet with two bullets:
  - "`stop` ends the task: Task Scheduler terminates the role without the graceful drain, so open connections are cut and the status file can stay behind. `<role> status` then reports it as stale, and the next `start` is not blocked. For a graceful stop on Windows, use the [system tier](#system-tier)."
  - "`restart` waits up to 15 seconds for the old process to exit before starting a new one."
- [ ] **Step 2:** Check the anchor `#system-tier` matches the `### System tier` heading and no other text in the file still says the user-tier stop drains: `rg -n -i "drain" docs/services.md`.
- [ ] **Step 3:** Commit: `git add docs/services.md && git commit -m "docs(services): record that Windows user-tier stop does not drain (#185)"`

---

## Task 2: Align the Windows lifecycle test with the specified stop — implements 2

**Covers:** 2.1, 2.2, 2.3, 2.4
**Difficulty:** Normal
**Blocked by:** none

**Files:**
- Modify: `tests/service_user_windows.rs` (module doc comment lines 19-21; `user_background_task_lifecycle`, the block from `// stop: the process ends.` to the `assert_ne!(started_pid, pid, ...)` after start)

**Interfaces:**
- Consumes: existing helpers `Fixture::run`, `Fixture::ok`, `wait_for`, `pid_of`, `pid_alive`, `node_id_of`, `stderr`, `stdout` in the same file.

- [ ] **Step 1:** Module comment: replace "SM-04 records terminate-without-drain as a known limitation, so the measurement is evidence rather than a gate." with "the `system-service-management` requirement \"Windows user-tier stop terminates without draining\" records the hard termination (measured in #185), so the drain outcome is evidence rather than a gate."
- [ ] **Step 2:** After the existing `EVIDENCE schtasks /End` line and the `service status` "serve is not running" assertion, add (spec scenario "Leftover snapshot reported stale"):

```rust
// A leftover snapshot is a crash leftover: shown, but flagged stale.
if !drained {
    let shown = fx.run(&["serve", "status"])?;
    let warning = stderr(&shown);
    assert!(
        warning.contains("serve is not running") && warning.contains("stale"),
        "{warning}"
    );
    assert!(stdout(&shown).contains(&first_id), "{}", stdout(&shown));
}
```

  The table prints `node_id: <id>` (`render_serve_status`, `src/status_cmd.rs`).
- [ ] **Step 3:** Delete the hand removal (`// A hard /End leaves a stale snapshot: remove it ...` and `let _ = std::fs::remove_file(&status_file);`). Replace the post-start wait with one that requires a new process, proving the start with leftovers in place (scenarios "Next start is not blocked", "Windows logon task lifecycle"):

```rust
// start with the leftovers in place: a NEW process, same identity.
fx.ok(&["serve", "service", "start"])?;
wait_for("a new process's status after start", || {
    node_id_of(&status_file).as_deref() == Some(first_id.as_str())
        && pid_of(&status_file).is_some_and(|p| p != pid)
})?;
let started_pid = pid_of(&status_file).unwrap();
assert_eq!(node_id_of(&status_file).unwrap(), first_id);
```

  Keep the following restart/uninstall code unchanged; remove the now-redundant `assert_ne!(started_pid, pid, ...)` only if the wait above already implies it.
- [ ] **Step 4:** `cargo fmt --all -- --check` on any host (expected: no diff). The file is `#![cfg(windows)]`, so it only compiles on a Windows host: run `cargo clippy --all-targets --all-features --locked -- -D warnings` there, or rely on the `lint (windows)` job of `ci.yml` on the PR. Cross-compiling from macOS fails in `ring` without a Windows SDK; if no Windows compile ran, record it in `apply.md` as not verified.
- [ ] **Step 5:** Run the E2E where it can run: on a Windows host, `$env:IROH_TUNNEL_E2E_SERVICE=1; cargo test --all-features --locked --test service_user_windows -- --nocapture --test-threads=1`; otherwise via `gh workflow run e2e.yml --ref <branch>` (needs the user's go-ahead to push). Expected: `user_background_task_lifecycle` passes and prints the `EVIDENCE schtasks /End` line. If neither is possible, record that in `apply.md` as not verified E2E.
- [ ] **Step 6:** Commit: `git add tests/service_user_windows.rs && git commit -m "test(service): check the stale leftover and leftover-tolerant start after Windows user-tier stop (#185)"`

---

## Scenario → test

- Stop leaves a stale snapshot → `tests/service_user_windows.rs::user_background_task_lifecycle` (waits for the old pid to exit, asserts `service status` says `serve is not running`; drain outcome reported)
- Leftover snapshot reported stale → `user_background_task_lifecycle` (Task 2 Step 2, `serve status` stale warning when the snapshot remains)
- Next start is not blocked → `user_background_task_lifecycle` (Task 2 Step 3, start without removing leftovers, new pid)
- No restart after stop → not automated: verifying needs a wait beyond the 1-minute restart interval on every E2E run; evidence is the #185 spike (no restart within 80 s on x64 and ARM64)
- Documented alternative → manual review of `docs/services.md` (Task 1 Step 2)
- Full lifecycle → `tests/service_user.rs::user_service_lifecycle_uses_the_pinned_absolute_paths` (systemd and launchd, unchanged)
- Windows logon task lifecycle → `user_background_task_lifecycle` (same node id after each start, task gone after uninstall)
- No linger → `tests/service_user.rs::user_install_runs_no_loginctl` (unchanged)
