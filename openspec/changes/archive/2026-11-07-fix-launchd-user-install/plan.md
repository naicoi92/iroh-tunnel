# launchd user install fix — Implementation Plan

> Executed via `/opsx-apply`, which chooses the executor.
> Spec reference: this change's `specs/` and `design.md`.

**Goal:** Make `service install --user` succeed on macOS when launchd has the job label disabled, and refuse an install that would register a service in a scope where the role already runs from the other side.

**Architecture:** Two independent edits in the host crate's service module. The launchd backend gains `launchctl enable` as the first command of `load`, matching the position `systemctl enable` already holds in the systemd backend. A new `ensure_no_cross_scope_conflict` joins `plan_install`'s pre-checks, comparing the role's held lock tier against the tier of the scope the install will effectively register in and refusing only on a mismatch. Both are covered through the existing `Native` fake, which records the exact command sequence. Lock-tier directories are made injectable so the new tests need neither `HOME` nor a cargo feature; the one behaviour no test can hold — launchd's persistent disable list — is verified by hand and recorded.

**Tech Stack:** Rust 2021 (MSRV 1.91), `anyhow`/`CliError`, the crate's `Native` runner and its `fake::FakeRunner` double, `tempfile`, `cargo test`/`clippy`/`fmt`.

## Global Constraints

- Rust 2021, MSRV 1.91; `Cargo.toml` is authoritative for dependency versions. No new dependency.
- Backend steps run commands through the injected `Native` runner, never `std::process::Command` directly, so tests can assert the sequence.
- `install` writes no registration before its checks pass; a failure reports what was already applied via `AppliedEffect`.
- The role's exit classification is unchanged. Conflict stays exit 6 (`CliError::Conflict`), as `crates/host/src/error.rs:9` documents.
- **New tests must pass under `cargo test -p iroh-tunnel-host --lib`, which does not enable the `test-utils` feature.** `ENV_MACHINE_LOCK_DIR` and the test-only `machine_dir()` are `#[cfg(feature = "test-utils")]`, so no new test may depend on them.
- **No new test may call `std::env::set_var`.** It is process-global and cargo runs tests in parallel. The codebase's only environment isolation is `fixtures::in_isolated_home`, which re-execs the test as a child with its own `HOME`; it is a private `#[cfg(test)]` module under `service`, unreachable from `ownership`'s tests.
- **No new test may touch the developer's real `~/Library/LaunchAgents`, `$HOME/.local/state/iroh-tunnel`, or `/run`//`var/run`.** Tests that would, run under `in_isolated_home` or against an injected directory.
- Rust standards RS-01..RS-16 in `docs/agents/rust-coding-standards.md` apply; repository standards override generic advice.
- Behaviour on systemd, BusyBox and Windows hosts is unchanged except where this plan states otherwise.
- Code, comments and commit messages in English.

## Host requirements per task

The backend test modules are platform-gated, so no single host verifies everything.

| Task | Host |
|---|---|
| 1, 3 | macOS (the launchd tests compile only there) |
| 2, steps 1-5 (plist) | macOS |
| 2, steps 6-9 (systemd guards) | Linux |
| 4, 5, 6 | any Unix |
| 7 | each of macOS and Linux, for the full suite |
| 8 | macOS, with a real install |

## Review Focus

Input classes the specs imply that no happy path exercises. Each has its test pinned to the task that owns the code.

1. **A held lock with no readable owner record.** `read_owner` returns `None` when `<role>.owner.json` is missing or unreadable, so a tier that *is* held can carry no record. A conflict check that reads "no record" as "not running" would silently permit the broken install. Pinned in Task 4, step 4.
2. **A system install run by root, where `HOME` is root's own.** The per-user lock directory derives from a home directory, so the probe must use the *resolved account's* home, never the process's. Pinned in Task 5, step 5, whose negative half is the discriminating assertion.
3. **A registered but stopped service in the other scope.** It holds no lock, so the install proceeds and two registrations coexist. That is the agreed behaviour — the check is about running roles — and a test pins it as a decision. Pinned in Task 5, step 6.
4. **A stale, unlocked lock file in the other tier.** `lock_is_held` treats a free lock as not held; a file left by a crashed process must not refuse the install. Pinned in Task 4, step 5.
5. **A non-root role's shared machine lock, probed by root.** `probe_tiers` prefers a *write* lock on the machine tier, so an exclusive attempt collides with another account's shared hold, while the user tier's shared probe does not. Root running `install --user` could therefore be refused because of someone else's process. Pinned in Task 4, step 6 — recorded, not changed, and noted among the residual gaps in Task 5 step 14.

---

## Contracts

| Name | new / modified | Shape | public |
|---|---|---|---|
| `ownership::LockKind` | modified | `enum LockKind { Exclusive, Shared }` becomes `pub(crate)`. It is private today, so a `pub(crate)` function naming it would trip `private_interfaces`, and `install.rs`'s tests could not write `LockKind::Exclusive` | no |
| `ownership::LockDirs` | new | `pub(crate) struct LockDirs { pub user: PathBuf, pub machine: Option<PathBuf> }`, plus `fn resolve(home: Option<&Path>) -> Result<LockDirs>` — both tiers' lock directories in one value. `machine` is `None` where the platform defines none. `user` resolves through `paths::state_dir_in(home)` when `home` is given, else `user_dir()?`. `state_dir_in` is `#[cfg(unix)]`, so on non-Unix `resolve` ignores `home` and always uses `user_dir()?`; its only non-Unix caller passes `None` anyway | no |
| `ownership::LockResolver` | new | `pub(crate) type LockResolver<'a> = &'a dyn Fn(Option<&Path>) -> Result<LockDirs>` — the seam the install path resolves lock directories through. Production passes `&LockDirs::resolve`; tests pass a closure returning temporary directories. A resolver rather than a resolved `LockDirs` because the directories depend on the account home, which is known only after `resolve_target_account`, and because resolving lazily at the check keeps `HOME`-missing failures off the paths that never probe (`ScopeChoice`, `InvokingUser`, permission refusals) | no |
| `ownership::tier_liveness_in` | new | `pub(crate) fn tier_liveness_in(role: Role, tier: Tier, dir: &Path) -> Result<Liveness>` — probes exactly `dir` for `role`'s lock and reports it as `tier`. Takes `tier` because it labels the returned `Liveness::Running { tier, .. }` and selects `lock_is_held`'s `prefer_write` (true for `Machine`, false for `User`), exactly as `probe_tiers` does | no |
| `ownership::tier_liveness` | new | `pub(crate) fn tier_liveness(role: Role, tier: Tier, dirs: &LockDirs) -> Result<Liveness>` — picks the tier's directory out of `dirs` and delegates to `tier_liveness_in`; `NotRunning` when that tier has no directory. Returns `Liveness`, not `Option<OwnerRecord>`, so "held" and "has a record" stay distinct — a held lock whose `<role>.owner.json` is unreadable is `Running { owner: None }` | no |
| `ownership::TestHold` | new, test-only | `#[cfg(test)] pub(crate) struct TestHold { _file: File }` (the leading underscore keeps `dead_code` quiet, as `RoleClaim::_locks` does) and `#[cfg(test)] pub(crate) fn hold_for_test(role: Role, dir: &Path, kind: LockKind) -> Result<TestHold>` — holds one tier's lock at an explicit directory, releasing when dropped. A concrete type, not `impl Drop`: `File` has no `Drop` impl of its own, and `impl Drop` in a return position trips `drop_bounds`. `dir` must exist; `create_lock_file` does not create parents | no |
| `service::platform::system_wide_only` | new | `pub(crate) fn system_wide_only() -> bool` — true only on a Linux host whose init is not systemd, where every registration is system-wide. One `#[cfg(target_os = "linux")]` body calling `linux_is_systemd`, and a `false` body elsewhere. Needed because `linux_is_systemd` is itself `#[cfg(target_os = "linux")]`, so `cfg!(target_os = "linux") && !linux_is_systemd()` fails to compile off Linux — `cfg!` is a value, not a conditional compilation guard. Lives beside `linux_is_systemd` so the backend selection has one home | no |
| `service::install::effective_scope_for` | new | `fn effective_scope_for(requested: ServiceScope, system_wide_only: bool) -> ServiceScope` — pure; `System` when `system_wide_only`, else `requested`. Its one caller passes `platform::system_wide_only()`, keeping the host probe out of the decision | no |
| `service::install::ensure_no_cross_scope_conflict_in` | new | `fn ensure_no_cross_scope_conflict_in(role: Role, registering: ServiceScope, dirs: &LockDirs) -> Result<()>` — `CliError::Conflict` when the tier the install does *not* register in holds the role. Takes both directories so a test can hold either tier and see the matching one permitted and the mismatched one refused | no |
| `service::install::ensure_no_cross_scope_conflict` | new | `fn ensure_no_cross_scope_conflict(role: Role, registering: ServiceScope, account_home: Option<&Path>, resolve: LockResolver) -> Result<()>` — calls `resolve(account_home)?` and delegates to the `_in` form. The wrapper is where the account-home-to-directory mapping meets the probe, so one test exercises it with the real `&LockDirs::resolve` | no |
| `service::install::install_via` | modified | gains a `lock_dirs: LockResolver` parameter, threaded into `plan_install` and `plan_system_install`. The public `install` passes `&LockDirs::resolve`; its signature and `InstallRequest` do not change | no |
| `service::fixtures::run_install_in` | new, test-only | `run_install_in(req, env, responses, lock_dirs: LockResolver)` — today's `run_install` body with the resolver passed through. `run_install` keeps its signature and delegates with a resolver returning two fresh tempdirs that outlive the call, so the seven existing callers (`install.rs:311, 324, 339, 361, 399, 431, 452`) never probe the real machine directory | no |
| `CliError::Conflict` | modified | `#[error("{0}")] Conflict(String)` — the `start refused: ` prefix moves to the call sites that mean a start | **yes, user-visible** |
| `ServiceNote::LaunchdBootstrapHint` | modified | `{ domain: String, plist: PathBuf }` — `target` removed. `ServiceNote` is re-exported at `crates/host/src/service/mod.rs:48`, so this is a published enum's shape | **yes** |

Two entries are public API changes: `CliError::Conflict`'s Display text and `ServiceNote::LaunchdBootstrapHint`'s fields. Under the project's stop rule for public API changes, **`/opsx-apply` must confirm both with the user before starting Task 3 and Task 6.** Neither is required by the specs — the conflict could keep the misleading `start refused:` prefix, and the note could keep an unused field — so both are available to drop if the user prefers. Nothing else in the plan depends on either, and Task 5 carries every specified behaviour on its own.

**`plan_install` gains an injected lock-directory resolver.** Without it the machine tier resolves through `machine_dir()`, which is the real `/run/iroh-tunnel` or `/var/run/iroh-tunnel` in a build without `test-utils`. Every install test would then probe the developer's machine, so a developer with the system service running — the very scenario this change is about — would see unrelated install tests fail, and results would differ between `--lib` and a workspace build that unifies the feature. `install_via` therefore takes a `LockResolver` and threads it into the private `plan_install`/`plan_system_install` pair. The public `install` passes `&LockDirs::resolve`; tests reach `install_via` only through `fixtures::run_install`/`run_install_in`, which pass a resolver returning temporary directories. The resolver is called only at the conflict check, with the account home where one is known, so no other path gains a resolution failure. A resolution error at the check surfaces as `ServiceFailure::before_effects`, like every other planning error. `InstallRequest` and the public `install` signature do not change.

No CLI argument, config key, file format or persisted schema changes. The plist's DOCTYPE string changes but no reader parses it.

---

## Task 1: launchd load enables the label — implements 1.1, 1.2, 1.3, 1.4, 1.5

**Files:**
- Modify: `crates/host/src/service/backend/launchd.rs` — `load` and the module doc comment
- Test: the same file's `mod tests`; `crates/host/src/service/install.rs` `mod tests`
- Host: macOS

**Interfaces:**
- Consumes: `Native`, `fake::{call, output, note_sink, Event, Events, FakeRunner}`, `OutputMode`, `run_launchctl`, `service_target`, `domain_target` — all present.
- Produces: nothing. No later task depends on this one.

- [ ] **Step 1: Write the failing test for the command order**

```rust
#[test]
fn load_enables_the_label_before_bootout_and_bootstrap() {
    let events = Events::default();
    let mut fake = FakeRunner::new(&events, vec![output(0, "", ""), output(0, "", ""), output(0, "", "")]);
    let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
    let mut native = Native::new(OutputMode::Inherit, &mut runner);
    load(&mut native, &mut |_| {}, "serve", ServiceScope::System, Path::new("/tmp/x.plist")).unwrap();
    assert_eq!(
        *events.borrow(),
        vec![
            call("launchctl", &["enable", "system/dev.iroh-tunnel.serve"], OutputMode::Inherit),
            call("launchctl", &["bootout", "system/dev.iroh-tunnel.serve"], OutputMode::Capture),
            call("launchctl", &["bootstrap", "system", "/tmp/x.plist"], OutputMode::Inherit),
        ]
    );
}
```

`enable` goes through `run_launchctl`, which uses `native.run`, so it records the ambient `OutputMode::Inherit`; `bootout` uses `native.output`, which is always `Capture`. If the implementation routes `enable` differently, fix the expectation to match the chosen call, not the other way round.

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p iroh-tunnel-host --lib load_enables_the_label_before_bootout_and_bootstrap`
Expected: FAIL — the recorded events start at `bootout`.

Do not add `--exact` with a partial path: zero tests would match and cargo would exit 0, making the red step vacuous.

- [ ] **Step 3: Write the failing test for an enable failure**

```rust
#[test]
fn load_fails_without_bootstrapping_when_enable_fails() {
    let events = Events::default();
    let mut fake = FakeRunner::new(&events, vec![output(1, "", "Operation not permitted")]);
    let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
    let mut native = Native::new(OutputMode::Inherit, &mut runner);
    let err = load(&mut native, &mut |_| {}, "serve", ServiceScope::User, Path::new("/tmp/x.plist")).unwrap_err();
    assert!(format!("{err:#}").contains("enable"), "{err:#}");
    assert_eq!(events.borrow().len(), 1, "no command after a failed enable");
}
```

This is Review Focus item 5's launchd half: an enable failure is reported, never swallowed like `bootout`.

- [ ] **Step 4: Write the failing test that a bootout failure is still ignored**

```rust
#[test]
fn load_ignores_a_bootout_failure_and_still_bootstraps() {
    let events = Events::default();
    let mut fake = FakeRunner::new(
        &events,
        vec![output(0, "", ""), output(3, "", "Boot-out failed: 3: No such process"), output(0, "", "")],
    );
    let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
    let mut native = Native::new(OutputMode::Inherit, &mut runner);
    load(&mut native, &mut |_| {}, "serve", ServiceScope::System, Path::new("/tmp/x.plist")).unwrap();
    assert_eq!(events.borrow().len(), 3, "bootstrap still issued");
}
```

- [ ] **Step 5: Run the three to verify they fail**

Run: `cargo test -p iroh-tunnel-host --lib launchd::tests`
Expected: the three new tests FAIL. The pre-existing ones still pass at this point — they break only once `load` changes, in step 6.

- [ ] **Step 6: Add the enable step to `load`**

Insert, as the first statement of `load` in `crates/host/src/service/backend/launchd.rs`, `run_launchctl(native, &["enable", &target])?`, before the existing best-effort `bootout`. Use `run_launchctl`, not `native.output(...)`, so the failure propagates.

- [ ] **Step 7: Fix every test whose response vector the new command shifts**

`enable` now consumes the first response, so each affected test needs one more leading `output(0, "", "")`.

- `launchd.rs`: `notes_reach_observer_before_native_command` asserts the exact event list for a failing load. Add the `enable` call at its head and a leading `output(0, "", "")` to its vector.
- `install.rs`: `partial_install_reports_written_registration` supplies `[output(0), output(5)]` on macOS, meaning "bootout ok, bootstrap fails". After the change, `enable` takes the 0, `bootout` takes the 5 and ignores it, and `bootstrap` gets the fake's default exit 0 — the install succeeds and `expect_err("the manager step must fail")` panics. Change the macOS vector to `[output(0), output(0), output(5)]`.
- Then search for the rest: `rg -n 'bootout|bootstrap|fake::output' crates tests` and check each site that builds a response list for a macOS `load` or `install`, including `tests/service_user.rs`.

- [ ] **Step 8: Run the crate's tests to verify they pass**

Run: `cargo test -p iroh-tunnel-host`
Expected: PASS.

- [ ] **Step 9: Update the doc comments**

In `launchd.rs`'s module doc comment, extend the paragraph about the modern vocabulary with why loading enables first: launchd keeps a persistent per-domain disable list, keyed by label, which survives `bootout`, uninstall and reboot, and makes `bootstrap` fail with `5: Input/output error`; `systemctl enable` holds the same position in the systemd backend. Add a line to `load`'s doc comment saying the enable is unconditional and idempotent, and that unlike `bootout` its failure is reported.

- [ ] **Step 10: Commit**

```bash
git add crates/host/src/service/backend/launchd.rs crates/host/src/service/install.rs
git commit -m "fix(launchd): enable the job label before bootstrap"
```

---

## Task 2: plist DOCTYPE and restart-containment guards — implements 1.6, 1.7, 1.8, 4.2

Two hosts are needed, so this task commits twice: steps 1 to 5 on macOS, steps 6 to 9 on Linux.

**Files:**
- Modify: `crates/host/src/service/backend/launchd.rs` — `format_plist_with`
- Modify: `crates/host/src/service/types.rs` — a comment on `RestartPolicy`
- Test: `launchd.rs` and `systemd.rs` test modules

**Interfaces:**
- Consumes: `format_plist`, `format_plist_with`, `RestartPolicy::DEFAULT`.
- Produces: nothing.

- [ ] **Step 1: Write the failing DOCTYPE test** (macOS)

```rust
#[test]
fn plist_declares_apples_property_list_doctype() {
    let body = format_plist("serve", Path::new("/b"), Path::new("/c"), Path::new("/s"));
    assert!(body.contains("PropertyList-1.0.dtd"), "{body}");
    assert!(!body.contains("PropertyManifest"), "{body}");
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p iroh-tunnel-host --lib plist_declares_apples_property_list_doctype`
Expected: FAIL — the body contains `PropertyManifest-1.0.dtd`.

- [ ] **Step 3: Correct the DOCTYPE string in `format_plist_with`**

Change `PropertyManifest-1.0.dtd` to `PropertyList-1.0.dtd` in the format string. Nothing else.

- [ ] **Step 4: Add the launchd restart-containment guard**

```rust
#[test]
fn plist_carries_no_exit_status_exemption() {
    // launchd's KeepAlive cannot exempt an exit status the way systemd's
    // RestartPreventExitStatus does; see design.md. This pins the gap so a
    // future edit cannot claim containment the platform does not provide.
    let body = format_plist("serve", Path::new("/b"), Path::new("/c"), Path::new("/s"));
    assert!(body.contains("<key>SuccessfulExit</key>\n\t\t<false/>"), "{body}");
    assert!(!body.contains("ExitStatus") && !body.contains("OtherJobEnabled"), "{body}");
}
```

It passes immediately — it is a regression guard, not a behaviour change. Its negative assertion is what `plist_default_policy_emits_keepalive_successful_exit_false` lacks.

- [ ] **Step 5: Run and commit the macOS half**

Run: `cargo test -p iroh-tunnel-host --lib launchd::tests`
Expected: PASS.

```bash
git add crates/host/src/service/backend/launchd.rs
git commit -m "fix(launchd): declare Apple's property-list doctype"
```

- [ ] **Step 6: Add the systemd counterpart guard** (Linux)

`format_unit_uses_default_restart_policy` asserts `RestartPreventExitStatus=6` among other unit text. Add a focused test, `unit_exempts_the_refused_start_status`, that renders the unit exactly as `format_unit_uses_default_restart_policy` (`systemd.rs:329-341`) does — same call, same arguments, `RestartPolicy::DEFAULT` — and asserts only `body.contains("RestartPreventExitStatus=6")`. It duplicates one assertion of that test on purpose, so the two platforms' containment guards sit side by side and neither can be deleted silently.

- [ ] **Step 7: Pin the re-install command order**

`notes_reach_observer_before_native_command` (`systemd.rs:296`) asserts only `log.len() == 4` and the final `Installed` note, so nothing pins that a re-install over a running unit restarts it. Extend it: the three `Event::Call` entries before the note carry `args` ending in `daemon-reload`, `enable iroh-tunnel-serve.service` and `restart iroh-tunnel-serve.service`, in that order. Compare the trailing arguments rather than the whole vector, so the assertion does not depend on the scope flag `run_systemctl` prepends. It passes immediately: it is the regression guard the delta's "Re-install over a running unit" scenario cites.

- [ ] **Step 8: Record the limitation beside the restart policy**

Add a comment to `RestartPolicy` in `crates/host/src/service/types.rs`: `on_failure` becomes `RestartPreventExitStatus=6` under systemd and a bare `KeepAlive{SuccessfulExit=false}` under launchd, which has no exit-status exemption, so a macOS job refused for "already running" is respawned until it is booted out.

- [ ] **Step 9: Run and commit the Linux half**

Run: `cargo test -p iroh-tunnel-host --lib systemd::tests`
Expected: PASS.

```bash
git add crates/host/src/service/backend/systemd.rs crates/host/src/service/types.rs
git commit -m "test(systemd): guard the refused-start restart exemption"
```

---

## Task 3: load-failure guidance names the disable list — implements 2.1, 2.2, 2.3

**Confirm with the user first:** step 4 removes a field from the published `ServiceNote` enum. If the user declines, do steps 1 to 3 only and keep `target`.

**Files:**
- Modify: `src/render.rs:31-42` (the arm) and `src/render.rs:358-366` (its test)
- Modify: `crates/host/src/service/native.rs:63-70` — the note's fields
- Modify: `crates/host/src/service/backend/launchd.rs` — the note's construction and `notes_reach_observer_before_native_command`
- Host: macOS, for the launchd test

**Interfaces:**
- Consumes: `ServiceNote::LaunchdBootstrapHint`.
- Produces: the modified `LaunchdBootstrapHint` contract.

- [ ] **Step 1: Write the failing render test**

`check(...)` at `src/render.rs:358` compares the full rendered string, so give the exact expected literal, trailing newline included. For `domain: "gui/501"`, `plist: "/p/x.plist"`:

```rust
"Hint: a disabled label is the usual cause - check `launchctl print-disabled gui/501`; \
 run `launchctl bootstrap gui/501 /p/x.plist` for launchd's own diagnostics.\n"
```

Use ASCII only in the message text, matching the other notes.

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --lib render::tests`
Expected: FAIL on the hint text.

- [ ] **Step 3: Rewrite the hint in `src/render.rs`**

Produce exactly the literal from step 1, with `domain` and `plist` interpolated. Drop `launchctl print {target}`, which reports only that the service is absent — the dead end this change exists to remove.

- [ ] **Step 4: Remove the now-unused `target` field**

Four sites: the variant in `crates/host/src/service/native.rs`; the destructuring pattern in `src/render.rs:31-35`; the `ServiceNote::LaunchdBootstrapHint { .. }` literal in `src/render.rs:358`; and the construction in `launchd.rs`'s `load` plus the `Event::Note(...)` expectation in `notes_reach_observer_before_native_command`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib render::tests && cargo test -p iroh-tunnel-host --lib launchd::tests`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/render.rs crates/host/src/service/native.rs crates/host/src/service/backend/launchd.rs
git commit -m "fix(launchd): point the bootstrap hint at the disable list"
```

---
## Task 4: tier-aware liveness probe — implements 3.1

**Files:**
- Modify: `crates/host/src/ownership.rs` — beside `probe_tiers`
- Test: the same file's `mod tests`
- Host: any Unix

**Interfaces:**
- Consumes: `probe_tiers`, `lock_is_held`, `read_owner`, `user_dir`, `machine_dir`, `lock_name`, `Tier`, `Liveness`, `OwnerRecord`, `create_lock_file`, `try_lock`, `Lock`, `LockKind`, `paths::state_dir_in`.
- Produces: `LockDirs`, `LockResolver`, `tier_liveness_in`, `tier_liveness`, `TestHold`/`hold_for_test`, and `LockKind` widened to `pub(crate)` — all as in Contracts. Task 5 consumes every one of them.

`probe` returns only the first `Liveness` from `probe_tiers`, and `ensure_not_running` refuses on either tier, so neither can ask about one tier. The directory resolution is a separate value because no new test may set an environment variable, depend on the `test-utils` feature, or read the real `HOME`.

- [ ] **Step 1: Widen `LockKind` and add the test hold**

Make `LockKind` `pub(crate)`. Add `TestHold { _file: File }` and `hold_for_test` as in Contracts — a concrete type over `File`, with an underscore-named field so `dead_code` does not fire on a field nothing reads, because `File` has no `Drop` impl of its own and `impl Drop` in a return position trips `drop_bounds`. `hold_for_test` creates the lock file with `create_lock_file` and holds it with `try_lock`, returning `TestHold` on `Lock::Held` and an error on `Lock::Busy`.

Verify it compiles before writing tests against it: `cargo clippy --all-targets -- -D warnings`.

- [ ] **Step 2: Write the failing tests for `LockDirs::resolve`**

Three cases, no filesystem writes and no environment mutation: with `Some(tempdir)`, `user` is `paths::state_dir_in(tempdir)` (gate this case on `cfg(unix)`, since `state_dir_in` is `#[cfg(unix)]`); with `None`, assert `LockDirs::resolve(None).ok().map(|d| d.user) == user_dir().ok()` — `user_dir` reads `HOME` and errs when it is unset, so compare the `Option`s rather than unwrapping; `machine` equals `machine_dir()`, `None` included. On non-Unix, `resolve` ignores `home` and always calls `user_dir()?`, as in Contracts.

- [ ] **Step 3: Write the failing tests for `tier_liveness_in`**

```rust
#[test]
fn tier_liveness_in_reports_a_held_lock() {
    let dir = tempfile::tempdir().unwrap();
    let _hold = hold_for_test(Role::Serve, dir.path(), LockKind::Exclusive).unwrap();
    let seen = tier_liveness_in(Role::Serve, Tier::Machine, dir.path()).unwrap();
    assert!(matches!(seen, Liveness::Running { tier: Tier::Machine, .. }), "{seen:?}");
}

#[test]
fn tier_liveness_in_is_not_running_for_an_empty_dir() {
    let dir = tempfile::tempdir().unwrap();
    let seen = tier_liveness_in(Role::Serve, Tier::User, dir.path()).unwrap();
    assert!(matches!(seen, Liveness::NotRunning), "{seen:?}");
}
```

The `tier` argument is not decoration: it labels the returned `Running { tier, .. }` and it selects `lock_is_held`'s `prefer_write`, which `probe_tiers` sets true for the machine tier and false for the user tier.

- [ ] **Step 4: Write the Review-Focus test for a held lock with no owner record**

```rust
#[test]
fn tier_liveness_in_is_running_even_without_an_owner_record() {
    // read_owner returns None when <role>.owner.json is missing. A held lock
    // must still read as Running, or a conflict check would permit the install.
    let dir = tempfile::tempdir().unwrap();
    let _hold = hold_for_test(Role::Serve, dir.path(), LockKind::Exclusive).unwrap();
    let Liveness::Running { owner, .. } = tier_liveness_in(Role::Serve, Tier::Machine, dir.path()).unwrap()
        else { panic!("a held lock must read as Running") };
    assert!(owner.is_none(), "no record was written");
}
```

This is why the contract returns `Liveness`: "held" and "has a record" are different questions.

- [ ] **Step 5: Write the Review-Focus test for a stale, unlocked file**

```rust
#[test]
fn tier_liveness_in_is_not_running_for_an_existing_but_unlocked_file() {
    let dir = tempfile::tempdir().unwrap();
    create_lock_file(&dir.path().join(lock_name(Role::Serve)), 0o600).unwrap(); // created, not locked
    let seen = tier_liveness_in(Role::Serve, Tier::Machine, dir.path()).unwrap();
    assert!(matches!(seen, Liveness::NotRunning), "{seen:?}");
}
```

- [ ] **Step 6: Write the Review-Focus test for a shared hold on each tier**

The machine tier probes with `prefer_write = true` and the user tier with `false`, so one shared holder is seen by the first and not by the second. Write both assertions and let the implementation's real behaviour set them — if the outcome differs from this expectation, the implementation is right and the test is wrong, because `prefer_write` comes from `probe_tiers` unchanged:

```rust
#[test]
fn a_shared_hold_is_seen_on_the_machine_tier_only() {
    // probe_tiers prefers a write lock on the machine tier, so an exclusive
    // probe collides with another account's shared hold. Pinned because it
    // means root running `install --user` can be refused by someone else's
    // user process. Not changed here; recorded.
    let dir = tempfile::tempdir().unwrap();
    let _hold = hold_for_test(Role::Serve, dir.path(), LockKind::Shared).unwrap();
    assert!(matches!(
        tier_liveness_in(Role::Serve, Tier::Machine, dir.path()).unwrap(),
        Liveness::Running { .. }
    ));
    assert!(matches!(
        tier_liveness_in(Role::Serve, Tier::User, dir.path()).unwrap(),
        Liveness::NotRunning
    ));
}
```

- [ ] **Step 7: Run them to verify they fail**

Run: `cargo test -p iroh-tunnel-host --lib ownership::tests`
Expected: FAIL — `LockDirs`, `tier_liveness_in` and `tier_liveness` are not defined.

- [ ] **Step 8: Implement the three items**

`LockDirs` and its `resolve`, `tier_liveness_in`, and `tier_liveness` as in Contracts. Factor `tier_liveness_in`'s body with `probe_tiers` rather than duplicating the lock logic, keeping `probe_tiers`' `prefer_write` choice per tier. Do not change `probe_tiers`' own behaviour; if step 6 shows the write preference makes a cross-account refusal possible, note it in a comment on `tier_liveness`.

- [ ] **Step 9: Run them to verify they pass**

Run: `cargo test -p iroh-tunnel-host --lib ownership::tests`
Expected: PASS, including the pre-existing lock tests.

- [ ] **Step 10: Keep the build warning-free until Task 5**

`tier_liveness`, `LockDirs::resolve` and the `LockResolver` alias have no non-test caller yet, so `clippy -D warnings` fails on `dead_code`. Add `#[cfg_attr(not(test), allow(dead_code))]` to each, with a comment naming Task 5, and remove it there.

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 11: Commit**

```bash
git add crates/host/src/ownership.rs
git commit -m "feat(ownership): add a tier-scoped liveness probe"
```

---

## Task 5: install refuses a cross-scope conflict — implements 3.2, 3.3, 3.4, 3.5, 3.6, 3.7, 3.9, 4.1

Self-contained: it depends on no later task. Task 6 only changes how the resulting error prints, so step 3's wording assertion is written as a substring check that holds either way.

**Files:**
- Modify: `crates/host/src/service/platform.rs` — `system_wide_only`
- Modify: `crates/host/src/service/install.rs` — `install_via`, `plan_install`, `plan_system_install`, new helpers
- Modify: `crates/host/src/service/fixtures.rs` — `run_install_in`, and `run_install` delegating to it
- Modify: `crates/host/src/service/install.rs` — `use crate::ownership::{self, LockDirs, LockResolver, Liveness, Tier};` for the production code, and `use crate::ownership::{hold_for_test, LockKind};` in `mod tests`; `install.rs` imports nothing from `ownership` today
- Test: `install.rs`'s `mod tests`, using `fixtures::{request, run_install, run_install_in, commands, in_isolated_home, write_valid_config, BATCH}`
- Host: any Unix; the install-level cases follow the existing early return on a non-systemd Linux host

**Interfaces:**
- Consumes: `ownership::{tier_liveness, LockDirs, LockResolver, Tier, Liveness, describe_owner, hold_for_test, LockKind}` from Task 4 (`describe_owner` is `pub fn` — reuse it); `verb::flag(scope)` for the `--system`/`--user` text; `platform::linux_is_systemd`; `ServiceScope`, `InstallRequest`, `ServiceFailure` (its `applied` field is `pub`), `AppliedEffect`, `CliError::Conflict`; `system::resolve_target_account`.
- Produces: `platform::system_wide_only`, `effective_scope_for`, `ensure_no_cross_scope_conflict_in`, `ensure_no_cross_scope_conflict`, the `install_via` resolver parameter and `fixtures::run_install_in`, as in Contracts. No later task consumes them.

Most coverage sits on the helper, because an install-level test needs a valid config, an isolated home and a platform-dependent backend, which would smother the behaviour under setup. The wrapper gets one test of its own, since the account-home-to-directory mapping lives only there.

- [ ] **Step 1: Add `platform::system_wide_only` with its tests**

Two bodies: `#[cfg(target_os = "linux")]` returning `!linux_is_systemd()`, and a `false` body elsewhere. A plain `cfg!(target_os = "linux") && !linux_is_systemd()` does not compile off Linux, because `cfg!` is a value and both operands must type-check. Put it beside `linux_is_systemd` so the backend selection keeps one home, and note in `backend_install`'s comment that the two now share the condition.

Test what the host allows: on Linux, that it is the negation of `linux_is_systemd()`; elsewhere, that it is false.

- [ ] **Step 2: Write the failing tests for `effective_scope_for`**

The pure function, three cases: `(User, false) -> User`, `(User, true) -> System`, `(System, true) -> System`. This is what makes tasks.md 3.2's BusyBox rule testable on any host.

- [ ] **Step 3: Write the failing tests for a refused mismatch**

```rust
#[test]
fn a_user_install_is_refused_while_the_machine_tier_holds_the_role() {
    let user = tempfile::tempdir().unwrap();
    let machine = tempfile::tempdir().unwrap();
    let _hold = ownership::hold_for_test(Role::Serve, machine.path(), LockKind::Exclusive).unwrap();
    let dirs = LockDirs { user: user.path().into(), machine: Some(machine.path().into()) };
    let err = ensure_no_cross_scope_conflict_in(Role::Serve, ServiceScope::User, &dirs)
        .expect_err("a machine-tier hold must refuse a user install");
    let text = format!("{err:#}");
    assert!(matches!(err.downcast_ref::<CliError>(), Some(CliError::Conflict(_))), "{err:?}");
    assert!(text.contains("service uninstall --system"), "{text}");
}
```

Add the mirror: a hold at `user` refuses `ServiceScope::System`, its message naming `service uninstall --user`. Holding two separate directories is what makes these tests mean something — the helper sees both tiers and must pick the right one.

- [ ] **Step 4: Write the failing tests for a permitted match**

A hold at the tier *matching* `registering` returns `Ok(())`, with the other directory empty. Two cases, one per scope. Not tautological: the helper has both directories, so a wrong tier choice fails here. This is the delta's "Re-install over the same scope's own running service" scenario, and the reason the check compares a tier rather than calling `ensure_not_running`.

- [ ] **Step 5: Write the Review-Focus test for the resolved account's home**

This one exercises the *wrapper*, because the account-home-to-directory mapping is the thing under test:

```rust
#[cfg(unix)]
#[test]
fn a_system_install_probes_the_resolved_accounts_home() {
    let account = tempfile::tempdir().unwrap();
    let state = crate::paths::state_dir_in(account.path());
    std::fs::create_dir_all(&state).unwrap(); // create_lock_file does not create parents
    let _hold = ownership::hold_for_test(Role::Serve, &state, LockKind::Exclusive).unwrap();
    ensure_no_cross_scope_conflict(Role::Serve, ServiceScope::System, Some(account.path()), &LockDirs::resolve)
        .expect_err("the resolved account's per-user tier must refuse a system install");
}
```

Then the negative half: the same call with `Some(other_empty_home)` returns `Ok(())` even while the first home's lock is held — the pin for Review Focus item 2, since `account::lookup` reads the passwd database and never `HOME`, so a wrapper that ignored its argument would look in the wrong place.

- [ ] **Step 6: Write the Review-Focus test for a registered-but-stopped service**

Produce the unheld lock file with `drop(ownership::hold_for_test(Role::Serve, machine.path(), LockKind::Exclusive).unwrap())` — lock files are never unlinked, and `create_lock_file`/`lock_name` are private to `ownership`, so `install.rs`'s tests cannot call them. With that file present in the mismatched tier's directory but unheld, `ensure_no_cross_scope_conflict_in` returns `Ok(())`. Pins that the check is about running roles, not registrations, so two coexisting registrations are a decision rather than an oversight.

- [ ] **Step 7: Run them to verify they fail**

Run: `cargo test -p iroh-tunnel-host --lib install::tests`
Expected: FAIL — the helpers are not defined.

- [ ] **Step 8: Implement the helpers**

`effective_scope_for` as in Contracts, its one caller passing `platform::system_wide_only()`.

`ensure_no_cross_scope_conflict_in` computes the non-registering tier — `Tier::Machine` for `ServiceScope::User`, `Tier::User` for `ServiceScope::System` — calls `ownership::tier_liveness(role, that_tier, dirs)`, and on `Liveness::Running { tier, owner }` returns `CliError::Conflict` naming `tier.label()`, `describe_owner(owner.as_ref())`, and `iroh-tunnel <role> service uninstall <flag>` with `verb::flag` of the *other* scope. `ensure_no_cross_scope_conflict` calls `resolve(account_home)?` on the injected resolver and delegates.

- [ ] **Step 9: Thread `LockDirs` through the install plan**

`install_via` gains a `lock_dirs: LockResolver` parameter and passes it to `plan_install`, which passes it to `plan_system_install` (both the Unix body and the non-Unix stub at `install.rs:194`, which ignores it). The public `install` passes `&LockDirs::resolve`. Nothing resolves eagerly: the resolver is called only inside `ensure_no_cross_scope_conflict`, with `None` on the user path and the account's home on the system path, so `ScopeChoice`, `InvokingUser` and permission refusals gain no new failure. Any error from it propagates through `plan_install` and becomes `ServiceFailure::before_effects` at `install_via:62`, like every planning error.

In `fixtures.rs`, move today's `run_install` body into `run_install_in(req, env, responses, lock_dirs: LockResolver)`, which passes the resolver to `install_via`. `run_install` keeps its signature: it creates two `tempfile::tempdir()`s held for the duration of the call and delegates with a closure returning `LockDirs { user: <first>, machine: Some(<second>) }` regardless of the home argument. Write the closure inline as `&|_: Option<&Path>| …` (or annotate the parameter): bound first with `let c = |_| …;`, rustc does not infer the higher-ranked signature and reports "not general enough". The seven existing callers stay unchanged and never probe `/run/iroh-tunnel` or `/var/run/iroh-tunnel` — without this, a developer with the system service running, the scenario this change is about, would see unrelated install tests fail.

- [ ] **Step 10: Call the check from both plan paths**

In `plan_install`, after `ensure_privilege` and before `check_installable_config`, with `effective_scope_for(scope, platform::system_wide_only())`, `None` as the account home, and the injected resolver. In `plan_system_install`, after `resolve_target_account` and before `trust::verify_dir`, with `ServiceScope::System`, `Some(&target.account.home)` and the injected resolver, so the per-user tier probed belongs to the invoking user.

- [ ] **Step 11: Add the install-level tests, including precedence**

Now that the helper is covered, pin the wiring through `run_install_in`, holding the machine tier in a tempdir and passing a resolver that returns it:

```rust
assert!(commands(&log).is_empty(), "the refusal precedes every command");
assert!(failure.applied.is_empty(), "nothing was applied");
// and: no registration file exists at the backend's path
```

covering tasks.md 3.5. Follow `partial_install_reports_written_registration` for the `in_isolated_home` and non-systemd early-return idioms.

Then tasks.md 3.4, which belongs here and not in Task 6 because it does not depend on the error's wording: an unprivileged `--system` install with the mismatched tier held reports `CliError::Permission`, not `CliError::Conflict` — the consequence of step 10's ordering. Gate it on `!crate::service::is_elevated()` as `start_is_refused_for_system_scope_without_privilege` does, and make the held tier the one that *would* refuse, so the assertion discriminates rather than passing for the same reason `unprivileged_system_install_is_refused_with_guidance` already does.

- [ ] **Step 12: Run the crate's tests to verify they pass**

Run: `cargo test -p iroh-tunnel-host`
Expected: PASS.

- [ ] **Step 13: Remove Task 4's `allow(dead_code)` attributes**

`tier_liveness`, `LockDirs::resolve` and `LockResolver` now have real callers.

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 14: Document the decision in `install.rs`**

Above `ensure_no_cross_scope_conflict`: why it compares a tier rather than calling `ensure_not_running` (a same-scope re-install is supported and specified); why the tier is the effective scope's (BusyBox has no per-user services); why the directories are injected (the real machine path would make the tests probe the developer's host); and the residual gaps — a foreground run in the same tier, a job loaded outside `service install`, a stopped service in the other scope, a shared machine-lock holder in another account, and a BusyBox user-scope request as root, where the non-registering tier is the calling process's own per-user directory rather than an invoking user's.

- [ ] **Step 15: Commit**

```bash
git add crates/host/src/service/install.rs crates/host/src/service/platform.rs crates/host/src/service/fixtures.rs
git commit -m "fix(service): refuse an install that conflicts with the other scope"
```

---

## Task 6: the conflict error reads correctly for an install — implements 3.8

**Confirm with the user first:** this changes a user-visible error string on every conflict path. If declined, skip the task entirely — every specified behaviour is already delivered by Task 5, and the install conflict simply keeps the misleading `start refused:` prefix.

**Files:**
- Modify: `crates/host/src/error.rs:34-37` — the `Conflict` variant's Display
- Modify: `crates/host/src/ownership.rs` — `already_running` and the other start-path `conflict(...)` callers
- Test: `crates/host/src/error.rs:56-59`, plus every test asserting `start refused:`
- Host: any

**Interfaces:**
- Consumes: `CliError::Conflict`, `ownership::conflict`.
- Produces: the modified `CliError::Conflict` contract. The exit code stays 6.

- [ ] **Step 1: Write the failing test for the prefix move**

Replace the assertion at `crates/host/src/error.rs:56-59` with `assert_eq!(CliError::Conflict("x".into()).to_string(), "x");`.

- [ ] **Step 2: Write the failing test that a start still says so**

In `ownership.rs`'s tests, assert `already_running(Role::Serve, "this user", None).to_string().starts_with("start refused:")`.

- [ ] **Step 3: Run both to verify they fail**

Run: `cargo test -p iroh-tunnel-host --lib -- error::tests ownership::tests`
Expected: FAIL.

- [ ] **Step 4: Move the prefix**

Change `Conflict`'s `#[error(...)]` to `"{0}"`. Then give the prefix to the start-path callers of `ownership::conflict`: `already_running`, and the identity-conflict message `publish_identity` produces ("same public identity"). Decide each `conflict(...)` call site explicitly — a start gains the prefix, an install does not — rather than prefixing the helper itself.

- [ ] **Step 5: Find every other caller and every assertion**

Run: `rg -n 'CliError::Conflict|start refused' crates src tests`. Fix each test asserting the old text. Add `assert!(!text.contains("start refused"))` to Task 5 step 3's install-conflict test, which now holds.

- [ ] **Step 6: Run the workspace tests to verify they pass**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/host/src/error.rs crates/host/src/ownership.rs crates/host/src/service/install.rs
git commit -m "refactor(error): move the start-refused prefix to its call sites"
```

---

## Task 7: documentation and full verification — implements 4.3, 5.1, 5.2, 5.5

- [ ] **Step 1: Audit the prose for stale text**

Run: `rg -n 'service install|launchctl|LaunchAgent|start refused' docs README.md`. Update what this change makes wrong. If nothing needs changing, say so in the commit body rather than inventing an edit.

- [ ] **Step 2: Run the formatter and linter**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 3: Run the whole suite on both hosts**

Run: `cargo test --workspace`, on macOS and on Linux.
Expected: PASS, including `tests/service_user.rs`, `tests/service_system.rs` and `tests/role_ownership.rs`.

- [ ] **Step 4: Confirm the specs validate**

Run: `openspec validate fix-launchd-user-install`
Expected: `Change 'fix-launchd-user-install' is valid`.

- [ ] **Step 5: Map every scenario to its test**

Walk the table below, confirm each named test exists and passes, and list any scenario covered only by Task 8.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "docs: update service docs for the launchd install fix"
```

---

## Task 8: manual verification on macOS — implements 5.3, 5.4

No test can hold launchd's persistent disable list, and none can observe launchd respawning a job. These two are verified by hand and recorded in this change's `verify.md`, at `openspec/changes/fix-launchd-user-install/verify.md`.

- [ ] **Step 1: Verify the disabled-label path**

On a macOS host with no conflicting system service:

```bash
launchctl disable gui/$(id -u)/dev.iroh-tunnel.serve
launchctl print-disabled gui/$(id -u) | grep iroh-tunnel   # expect: disabled
iroh-tunnel serve service install --user                   # expect: success
launchctl print gui/$(id -u)/dev.iroh-tunnel.serve | head  # expect: the job, loaded
```

Record every command and its output.

- [ ] **Step 2: Verify the conflict refusal**

With a system service of the role running:

```bash
iroh-tunnel serve service install --user ; echo "exit=$?"   # expect: exit 6
ls ~/Library/LaunchAgents/dev.iroh-tunnel.serve.plist       # expect: No such file
```

Confirm stderr names the machine-wide tier and `service uninstall --system`. Record it.

- [ ] **Step 3: Record any step the environment prevented**

If no host is available, or a running system service cannot be arranged, say so explicitly with what was done instead. Do not record a verification that was not performed.

---

## Scenario → test

Every `#### Scenario:` in `specs/system-service-management/spec.md`.

**Start and conflicting install refused when the role already runs**
- Role already running → existing `service::verb::tests::start_is_refused_before_manager_when_role_runs`, and `tests/role_ownership.rs`'s `service start` refusal
- User install while a system service of the role runs → Task 5 step 3, plus the install-level case in step 11
- System install while a user service of the role runs → Task 5 step 3's mirror case and step 5's wrapper test
- Re-install over the same scope's own running service → Task 5 step 4
- Privilege is reported before the conflict → Task 5 step 11's second half
- User-scope re-install on a backend with no per-user services → Task 5 step 2's `effective_scope_for(User, true) -> System`, with step 4's same-scope case supplying the permitted outcome

**launchd registration and domains**
- launchd registration shape → existing `plist_is_valid_xml_and_has_label`, `plist_pins_config_argument_and_state_dir_environment`, `plist_path_user_is_under_launchagents`
- Plist document type → Task 2 step 1 `plist_declares_apples_property_list_doctype`
- Load command order → Task 1 step 1 `load_enables_the_label_before_bootout_and_bootstrap`
- Re-install over a loaded job → Task 1 step 1, the same assertion: the sequence is the behaviour
- Enabling the label fails → Task 1 step 3 `load_fails_without_bootstrapping_when_enable_fails`
- Booting out a job that is not loaded → Task 1 step 4 `load_ignores_a_bootout_failure_and_still_bootstraps`

**launchd verbs and idempotent stop**
- Stop an already stopped job → existing `stop_captures_even_when_inheriting_and_reports_already_stopped`
- Bootstrap fails → Task 3 step 1, the `render::tests` hint assertion

**Restart policy is shared across platforms**
- Policy disabled → existing `systemd::tests::format_unit_with_disabled_policy_omits_restart` (the scenario is about `Restart=no`, a systemd string; the launchd counterpart `plist_disabled_policy_omits_keepalive_block` covers its own backend)
- systemd exempts the refused-start status → Task 2 step 6 `unit_exempts_the_refused_start_status`
- launchd carries no exit-status exemption → Task 2 step 4 `plist_carries_no_exit_status_exemption`

**systemd unit content and lifecycle**
- Unit registration shape → existing `systemd::tests::unit_path_system_scope_is_under_etc`, `unit_name_is_stable`, `unit_pins_state_dir_and_quotes_paths_with_spaces`
- Unit text → existing `systemd::tests::format_unit_uses_default_restart_policy`
- Re-install over a running unit → `systemd::tests::notes_reach_observer_before_native_command`, extended in Task 2 step 7 to assert `daemon-reload`, `enable` and `restart` by name, together with Task 5 step 4's same-scope permission.

Covered only by manual verification (Task 8): the end-to-end disabled-label install, and the conflict refusal's real exit code and filesystem effect.
