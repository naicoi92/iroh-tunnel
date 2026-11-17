# BusyBox Script Hardening Implementation Plan

> Executed via `/opsx-apply`, which chooses the executor.
> Spec reference: this change's `specs/` and `design.md`.

**Goal:** Make the root-run BusyBox init script immune to path injection and to PID reuse (#220).

**Architecture:** The renderer becomes a small value type (`InitScript`) rendered against a `ScriptLayout` (PID file, log, proc root); every interpolated value goes through the host's existing `shell_quote`. The script records the launched executable in the PID file and a three-way `identity` shell function (ours / stale / unverifiable) checks `/proc/<pid>/exe` (plus a narrow legacy fallback) before any signal. `parse_busybox_script` decodes both script generations.

**Tech Stack:** Rust 2021 (MSRV 1.91), `anyhow`, `tempfile` (dev), POSIX `sh` (BusyBox ash in production; `/bin/sh` = bash 3.2 on macOS and dash/bash on Linux CI for tests).

**Module layout:** the pure renderer and template move to a new `crates/host/src/service/backend/busybox_script.rs`, declared in `backend/mod.rs` as `#[cfg(any(target_os = "linux", all(test, unix)))] pub(super) mod busybox_script;` so its unit and execution tests compile and run on macOS too (design D3). `busybox.rs` (effects, Linux-only) imports it. Add no dead-code attribute up front (tests use every item on macOS, so an `expect` would be unfulfilled); if clippy on macOS still reports an item, use `#[cfg_attr(not(target_os = "linux"), allow(dead_code, reason = "..."))]` as `effective_binary.rs` does.

## Global Constraints

- No-panic policy (#216) for production code: no `unwrap`/`panic!`/`todo!`/`unimplemented!`; `expect`/`unreachable!`/indexing/slicing only with a narrow `#[expect(clippy::<lint>, reason = "invariant: ...")]`; I/O and external input → `Result` with context; no `print!`/`println!`. Tests may unwrap.
- Script must be POSIX sh: no `local`, no `[[`, no arrays, no `pidof`/`start-stop-daemon`; only `cat`, `head`, `tail`, `tr`, `readlink`, `kill`, `sleep`, `rm`, `printf`, `echo`.
- No new dependency, no public CLI/API change, do not touch `src/render.rs` or backend signatures beyond `busybox.rs` internals (#225).
- Verify: `cargo fmt`; `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`; `RUBY=/opt/homebrew/Library/Homebrew/vendor/portable-ruby/current/bin/ruby cargo test --all-features --locked`.

## Review Focus

- A path whose last characters are spaces or newlines → it must still be passed literally to the daemon (quoting), and identity must fail closed rather than match a wrong process.
- PID file with garbage (empty, non-numeric, negative, `1 2`) → treated as stale, nothing signalled (pinned in Task 3 by `stale_garbage_pid_file_is_removed_without_signal`).
- Process ignores SIGTERM and the PID stops matching during the 5 s grace window → SIGKILL must not be sent (pinned by `pid_reused_during_grace_is_not_killed`, which swaps the fake `exe` link while `stop` waits).
- `status` run by a non-root user → must not delete the PID file and must not claim "not running" for an unreadable root process (pinned by `status_with_stale_pid_reports_not_running` and `unverifiable_identity_fails_loud`).
- Legacy fallback with the standalone binary name `iroh-tunnel-serve` → its argv has no role word; both old forms (combined argv role word, standalone executable name) are accepted regardless of the current script's invocation (pinned by `legacy_pid_after_binary_change_is_stopped`, each current × each old form).
- PID `0` or with leading zeros → stale, never `kill -0 0` (process group) (pinned by adding `"0"`, `"00"` to `stale_garbage_pid_file_is_removed_without_signal`).

---

## Contracts

| Name | new / modified | Shape | public |
|---|---|---|---|
| `InitScript<'a>` (`backend/busybox_script.rs`, `pub(super)`) | new | `struct InitScript<'a> { role: &'a str, invocation: ServiceInvocation, binary: &'a Path, config: &'a Path, state: &'a Path, pin: Option<&'a Pin> }` with `fn render(&self, layout: &ScriptLayout) -> anyhow::Result<String>`; error `CliError::Config("<what> path is not valid UTF-8: <path.display()>")` for binary/config/state | no |
| `ScriptLayout` (`backend/busybox_script.rs`, `pub(super)`) | new | `struct ScriptLayout { pidfile: PathBuf, logfile: PathBuf, proc_root: PathBuf }` with `fn for_role(role: &str) -> Self` = `/var/run/iroh-tunnel-<role>.pid`, `/var/log/iroh-tunnel-<role>.log`, `/proc` | no |
| `format_script`, `format_script_invocation` | modified (removed) | replaced by `InitScript::render` | no |
| BusyBox init script variables | modified | lines `DAEMON=<q>`, `ROLE=<q>`, `CONFIG=<q>`, `export IROH_TUNNEL_STATE_DIR=<q>`, pin lines unchanged, `PIDFILE=<q>`, `LOG=<q>`, `PROC=<q>`, where `<q>` = `policy::shell_quote(value)` | no (generated file) |
| BusyBox PID file format | modified | line 1: decimal PID; rest: launched `$DAEMON` path followed by a single `.` sentinel (`printf '%s\n%s.' "$!" "$DAEMON"`). Legacy: PID only | no (runtime file) |
| BusyBox init script exit statuses | modified | `start`/`stop`: 0 ok, 1 unverifiable identity (PID file kept); `status`: 0 running, 3 not running, 4 unknown | no (generated file; `run_script` already surfaces non-zero) |
| `parse_busybox_script(Option<&str>) -> EffectiveBinary` | modified | signature unchanged; accepts `DAEMON="<path>"` (legacy, only when the path has no `$`, backtick, `\` or `"`) or a restricted POSIX word (plain chars `[A-Za-z0-9_@%+=:,./-]`, `'…'`, `\'`); anything else → `Unknown` | no |

---

## Task 1: Decode DAEMON from both script generations — implements 1

**Covers:** 1.1, 1.2, 1.3
**Difficulty:** Easy
**Blocked by:** none

**Files:**
- Modify: `crates/host/src/service/effective_binary.rs` (`parse_busybox_script` and its tests)

**Interfaces:**
- Produces: `parse_busybox_script` (contract above), used by Task 2 round-trip tests.

- [ ] **Step 1: Write failing tests** in `effective_binary.rs` `mod tests`:
  - `busybox_single_quoted_daemon_yields_exact_path`: `DAEMON='/a b/it'\''s/iroh-tunnel'` → `bin("/a b/it's/iroh-tunnel")`.
  - `busybox_plain_daemon_yields_path`: `DAEMON=/usr/bin/iroh-tunnel` → `bin("/usr/bin/iroh-tunnel")`.
  - `busybox_legacy_double_quoted_daemon_still_parses`: `DAEMON="/a b/iroh-tunnel"` → `bin("/a b/iroh-tunnel")` (existing `busybox_daemon_line_yields_binary` may already cover; keep it).
  - `busybox_daemon_with_expansion_is_unknown`: each of `DAEMON=$HOME/x`, ``DAEMON=`id` ``, `DAEMON='/a`, `DAEMON=/a b`, `DAEMON=`, `DAEMON="/a/$x"`, ``DAEMON="/a/`id`"``, `DAEMON="/a\b"`, `DAEMON="/a"b"` → `matches!(…, EffectiveBinary::Unknown(_))`.
- [ ] **Step 2:** `cargo test -p iroh-tunnel-host --all-features --locked effective_binary` → new tests FAIL.
- [ ] **Step 3: Implement** `fn decode_shell_word(word: &str) -> Option<String>` (private) iterating `chars()`: plain set → push; `'` → read until next `'` (missing → `None`); `\` followed by `'` → push `'`; anything else → `None`; empty result → `None`. `parse_busybox_script`: find the line with prefix `DAEMON=`; if the rest starts with `"`, strip `"…"` and reject (→ `Unknown`) when the inner text is empty or contains `$`, `` ` ``, `\` or `"`; else `decode_shell_word`. No indexing/slicing.
- [ ] **Step 4:** re-run → PASS; whole host crate tests green.
- [ ] **Step 5: Commit** `fix(service): read BusyBox DAEMON as a shell literal`.

---

## Task 2: Render every script value as a shell literal — implements 2

**Covers:** 2.1, 2.2, 2.3, 2.4, 2.5
**Difficulty:** Normal
**Blocked by:** Task 1

**Files:**
- Create: `crates/host/src/service/backend/busybox_script.rs` (renderer, template, all render/exec tests)
- Modify: `crates/host/src/service/backend/mod.rs` (module declaration), `crates/host/src/service/backend/busybox.rs` (`install` uses the renderer; move render tests out)
- Read only: `crates/host/src/service/policy.rs` (`shell_quote`)

**Interfaces:**
- Consumes: `parse_busybox_script` (Task 1), `super::super::policy::shell_quote(&str) -> String`.
- Produces: `InitScript`, `ScriptLayout`, test harness `ScriptHarness` (below) for Task 3.

- [ ] **Step 1: Test harness** (`#[cfg(test)] mod exec` inside `busybox_script.rs` tests):
  `struct ScriptHarness { dir: tempfile::TempDir, script: PathBuf, layout: ScriptLayout, record: PathBuf, seen: RefCell<Vec<u32>> }` with
  - `fn new(binary_name: &str, config_name: &str, state_name: &str, invocation: ServiceInvocation, daemon: DaemonKind) -> Self` where `enum DaemonKind { Exits, IgnoresTerm }` — creates `dir/<binary_name>` fake daemon (`#!/bin/sh`, `IgnoresTerm` first runs `trap '' TERM`; writes each argv on its own line, then `STATE=$IROH_TUNNEL_STATE_DIR` to `record`, then `exec sleep 30`), mode 0755; config/state as paths under `dir`; layout under `dir` (`run.pid`, `run.log`, `proc/`); renders with `InitScript::render` and writes `script` mode 0755.
  - `fn run(&self, verb: &str) -> std::process::Output` — `Command::new("/bin/sh").arg(&self.script).arg(verb).current_dir(&self.dir).stdin(Stdio::null())`; records any PID found in the PID file afterwards into `seen`.
  - `fn fake_proc(&self, pid: u32, exe: &str, cmdline: &[&str])` — creates `proc/<pid>/exe` symlink to `exe` (target need not exist) and `proc/<pid>/cmdline` with NUL-separated args + trailing NUL.
  - `fn pidfile_pid(&self) -> u32`; `fn spawn_unrelated(&self) -> u32` spawns an **orphan** `sleep 30` via `sh -c 'sleep 30 >/dev/null 2>&1 & echo $!'` (never a direct child, so a killed stand-in is reaped by init/launchd and not left as a zombie that `kill -0` still sees) and remembers it in `seen`; `fn alive(pid) -> bool` via `kill -0`; `fn wait_gone(pid, timeout)` polls `alive`.
  - `fn set_exe(&self, pid, target)` re-points `proc/<pid>/exe` atomically (symlink to a temp name, then `rename`).
  - `Drop`: `kill -9` every PID in `seen` (not only the current PID file, which `stop`/`start` rewrite).
  The record file must be awaited (poll up to 5 s) before asserting or before sending `stop`, since the daemon starts in background and installs its trap before `exec`. Daemons started by the script are orphans as well; when running these tests in a Linux container use an init (`docker run --init`).
- [ ] **Step 2: Failing tests**
  - `metachar_config_path_reaches_daemon_literally`: config name `` c $(touch pwned) `touch pwned2` "q" 'a' \b.toml ``; `run("start")`; wait for record; assert record has `--config` followed by the exact full config path, and `!dir.join("pwned").exists() && !dir.join("pwned2").exists()`.
  - `metachar_binary_and_state_paths_are_literal`: binary name `` it $HOME `x`-tunnel ``, state `` s $HOME `x` ``; assert the daemon ran (record exists) and `STATE=` line equals the literal state path.
  - `non_utf8_config_is_refused` (unix): `OsStr::from_bytes(b"/c\xff")` → `render` is `Err`, downcasts to `CliError::Config` and the message contains `not valid UTF-8`.
- [ ] **Step 3:** run → FAIL (types missing / injection happens).
- [ ] **Step 4: Implement** `InitScript`/`ScriptLayout` per Contracts in `busybox_script.rs`; declare the module in `backend/mod.rs` per Module layout. `fn utf8<'p>(what: &str, path: &'p Path) -> Result<&'p str>` using `to_str().ok_or_else(|| CliError::Config(…).into())`. Variable block uses `shell_quote` for `DAEMON`, `ROLE`, `CONFIG`, `IROH_TUNNEL_STATE_DIR`, `PIDFILE`, `LOG`, `PROC` (add `PROC` now, used in Task 3). Keep the existing start/stop/status bodies unchanged in this task. Update the module doc comment ("Paths are embedded verbatim…" → literal quoting).
- [ ] **Step 5: Move and port old render tests** from `busybox.rs` to `busybox_script.rs`, using `InitScript { … }.render(&ScriptLayout::for_role(role))?`: `format_script_embeds_paths_and_all_verbs` (expect `DAEMON=/usr/local/bin/iroh-tunnel`, `PIDFILE=/var/run/iroh-tunnel-serve.pid`, `LOG=/var/log/iroh-tunnel-serve.log`, `export IROH_TUNNEL_STATE_DIR=/var/lib/iroh-tunnel`, plus shebang, invocation and case arms), `invocation_form_survives_managed_paths_with_spaces` (expect `DAEMON='/managed path/copy'`), `managed_path_renders_as_one_argument_in_each_backend` and `effective_binary_round_trips_the_generated_script` (round-trip through `parse_busybox_script`, also for a path with `'` and `$`). Delete `format_script` and `format_script_invocation`.
- [ ] **Step 6:** `install` renders with `ScriptLayout::for_role(role)` and `?` before `write_registration`.
- [ ] **Step 7:** run host tests → PASS; clippy on host crate clean.
- [ ] **Step 8: Commit** `fix(service): quote every BusyBox init script value as a shell literal`.

---

## Task 3: Verify process identity before acting on the PID file — implements 3

**Covers:** 3.1, 3.2, 3.3, 3.4
**Difficulty:** Hard
**Blocked by:** Task 2

**Files:**
- Modify: `crates/host/src/service/backend/busybox_script.rs` (script template, exec tests)

**Interfaces:**
- Consumes: `InitScript`, `ScriptLayout`, `ScriptHarness` (Task 2).

- [ ] **Step 1: Failing exec tests** (all `#[cfg(unix)]`, using `ScriptHarness`; "ours" is simulated with `fake_proc(pid, <daemon path>, …)` since a fake proc root replaces `/proc`):
  - `start_records_pid_and_daemon`: after `start`, PID file line 1 = live pid, rest = `<daemon path>.`.
  - `stop_terminates_the_recorded_daemon`: `DaemonKind::Exits`; `start`; `fake_proc(pid, daemon)`; `stop` → process gone within 2 s, PID file removed.
  - `stubborn_daemon_is_killed_after_grace`: `DaemonKind::IgnoresTerm`; `start`; wait for record; `fake_proc(pid, daemon)`; `stop` returns after ~5 s; process gone, PID file removed.
  - `status_without_pid_file_is_not_running`: no PID file → `status` exit 3.
  - `stale_pid_of_unrelated_process_is_not_signalled`: unrelated `sleep` pid; write new-format PID file `"<pid>\n<daemon>."`; `fake_proc(pid, "/bin/sleep")`; `stop` → still alive, PID file removed.
  - `start_with_stale_pid_launches_daemon`: same stale setup; `start` → stdout lacks "already running", record appears, PID file names the new pid.
  - `status_with_stale_pid_reports_not_running`: stale setup; `status` exit code 3, PID file still present.
  - `stale_garbage_pid_file_is_removed_without_signal`: PID files `""`, `"abc"`, `"-1"`, `"1 2"`, `"0"`, `"00"` and a dead PID (spawn+reap a `true`) → `status` exit 3 (file kept); `start` launches the daemon and rewrites the file.
  - `replaced_daemon_with_deleted_suffix_is_ours`: `start`; `fake_proc(pid, "<daemon> (deleted)")`; second `start` says "already running"; `stop` terminates it.
  - `unverifiable_identity_fails_loud`: unrelated live pid in new-format file, no fake proc entry → `stop` exit 1, `start` exit 1 (nothing launched), `status` exit 4; process alive and PID file unchanged after each.
  - `unsignalable_pid_with_proc_entry_is_unverifiable`: a dead PID (spawn and reap `true`) with a fake proc entry present simulates EPERM from `kill -0` → `status` exit 4, `stop` exit 1, PID file unchanged.
  - `pid_reused_during_grace_is_not_killed`: `DaemonKind::IgnoresTerm`; `start`; wait for record; `fake_proc(pid, daemon)`; spawn `stop` as a child process; after ~1 s `set_exe(pid, "/bin/sleep")`; when `stop` returns the process is still alive (no SIGKILL) and the PID file is removed.
- [ ] **Step 2: Failing legacy tests**:
  - `legacy_pid_of_current_daemon_is_restarted`: unrelated `sleep` as stand-in, legacy file `"<pid>\n"`, `fake_proc(pid, daemon)`; `restart` → stand-in gone, new two-line PID file.
  - `legacy_pid_after_binary_change_is_stopped`: for each current invocation form × each old form: stand-in with `fake_proc(pid, exe, argv)` where old combined is exe `/old/iroh-tunnel`, argv `["/old/iroh-tunnel", "serve", "run", "--config", <config>]` and old standalone is exe `/old/iroh-tunnel-serve`, argv `["/old/iroh-tunnel-serve", "run", "--config", <config>]`; `restart` → stand-in gone.
  - `legacy_pid_of_unrelated_process_is_kept`: legacy file with `fake_proc` cases `("/usr/bin/sleep", ["sleep","30"])`, `("/x/iroh-tunnel", [.., "serve", "run","--config","/other.toml"])` and `("/x/iroh-tunnel", [.., "access", "run","--config",<config>])` (other role) → `stop` leaves it alive; `start` launches the daemon.
  - `reinstall_over_legacy_registration_replaces_old_daemon`: a test fixture `const LEGACY_SCRIPT` holds the pre-change template (copy of today's `format_script` output for the harness paths, with `PIDFILE`/`LOG` pointing at the harness layout); run its `start` (real legacy PID file via `echo $! >`), wait for the record, `fake_proc(pid, daemon)`, then run the new script's `restart` → old daemon gone, new two-line PID file names a new live daemon (covers "Re-install migrates an existing registration" together with `install` always calling `restart`).
- [ ] **Step 3:** run → FAIL.
- [ ] **Step 4: Implement the template** (design D4):

```sh
SEP=$(printf '\001')

read_pidfile() {
    PID=$(head -n 1 "$PIDFILE" 2>/dev/null)
    REC=$(tail -n +2 "$PIDFILE" 2>/dev/null)
    REC=${REC%.}
}

exe_matches() { # $1 exe, $2 expected
    [ -n "$2" ] && { [ "$1" = "$2" ] || [ "$1" = "$2 (deleted)" ]; }
}

legacy_match() { # $1 exe
    exe_matches "$1" "$DAEMON" && return 0
    NAME=${1%" (deleted)"}; NAME=${NAME##*/}
    case "$NAME" in iroh-tunnel*) ;; *) return 1 ;; esac
    CMD=$(tr '\000' '\001' 2>/dev/null < "$PROC/$PID/cmdline") || return 1
    TAIL="${SEP}run${SEP}--config$SEP$CONFIG$SEP"
    case "$CMD" in
        *"$SEP$ROLE$TAIL") return 0 ;;                                    # combined
        *"$TAIL") [ "$NAME" = "iroh-tunnel-$ROLE" ] && return 0 ;;         # standalone
    esac
    return 1
}

# 0 = ours, 1 = stale, 2 = unverifiable
identity() {
    [ -f "$PIDFILE" ] || return 1
    read_pidfile
    case "$PID" in ''|0*|*[!0-9]*) return 1 ;; esac
    if ! kill -0 "$PID" 2>/dev/null; then
        [ -e "$PROC/$PID" ] && return 2   # alive but not ours to signal (EPERM)
        return 1                           # dead
    fi
    [ -e "$PROC/$PID" ] || return 2
    EXE=$(readlink "$PROC/$PID/exe" 2>/dev/null) || return 2
    if [ -n "$REC" ]; then exe_matches "$EXE" "$REC" && return 0; return 1; fi
    legacy_match "$EXE" && return 0
    return 1
}
```

  Note on the combined pattern: `*"$SEP$ROLE$TAIL"` also matches an argv whose program basename is the role (impossible for `iroh-tunnel*` names other than `iroh-tunnel-<role>`, which the standalone branch accepts anyway). `start`: `identity; case $? in 0) echo already running; return 0;; 2) echo "cannot verify pid $PID"; return 1;; esac; rm -f "$PIDFILE"; launch; printf '%s\n%s.' "$!" "$DAEMON" > "$PIDFILE"`. `stop`: `identity; case $? in 2) …; return 1;; 0) TERM; i=0; while [ $i -lt 5 ] && identity; do sleep 1; …; done; identity && KILL;; esac; rm -f "$PIDFILE"`. `status`: `identity; case $? in 0) running, return 0;; 2) unknown, return 4;; *) not running, return 3;; esac`. The `case "$1"` dispatcher's `exit $?` keeps these codes; `restart` is `stop && start` so an unverifiable stop does not launch a second daemon. Use only POSIX constructs; `$SEP` is not a newline, so it survives command substitution. Update the ported `format_script_embeds_paths_and_all_verbs` case-arm assertion from `restart) stop; start` to `restart) stop && start`.

- [ ] **Step 5:** run Task 3 tests, then all host tests → PASS.
- [ ] **Step 6:** Full verify set from Global Constraints → all green.
- [ ] **Step 7: Commit** `fix(service): BusyBox script acts only on the daemon it launched`.

---

## Task 4: Document the hardened BusyBox script — implements 4

**Covers:** 4.1
**Difficulty:** Easy
**Blocked by:** Task 3

**Files:**
- Modify: `docs/services.md` ("BusyBox and SysV init")

- [ ] **Step 1:** Add: values in the script are stored as shell literals; `start`, `stop` and `status` only act on the process the script launched (checked through `/proc/<pid>/exe`), a stale PID file is removed and never signalled, an unverifiable PID makes `start`/`stop` fail (exit 1) and `status` report unknown (exit 4); re-run `service install` to upgrade scripts written by older versions; run `service stop` before downgrading.
- [ ] **Step 2: Commit** `docs(services): describe BusyBox script hardening`.

---

## Scenario → test

- BusyBox paths and stopped status → `format_script_embeds_paths_and_all_verbs` (ported) + `status_without_pid_file_is_not_running` (exit 3)
- Script verbs → `format_script_embeds_paths_and_all_verbs` (ported)
- Start records the launched executable → `start_records_pid_and_daemon`
- Stop of the running daemon → `stop_terminates_the_recorded_daemon`, `stubborn_daemon_is_killed_after_grace`
- PID reused during the stop grace window → `pid_reused_during_grace_is_not_killed`
- Config path with shell metacharacters → `metachar_config_path_reaches_daemon_literally`
- State and binary paths with shell metacharacters → `metachar_binary_and_state_paths_are_literal`
- Non-UTF-8 path refused → `non_utf8_config_is_refused`
- Stale PID file names an unrelated live process → `stale_pid_of_unrelated_process_is_not_signalled`
- Start with a stale PID file → `start_with_stale_pid_launches_daemon`
- Status with a stale PID file → `status_with_stale_pid_reports_not_running`
- Dead or malformed PID file → `stale_garbage_pid_file_is_removed_without_signal`
- Unverifiable identity → `unverifiable_identity_fails_loud`, `missing_exe_link_is_unverifiable`, `unreadable_pid_file_is_unverifiable`, `legacy_pid_with_unreadable_cmdline_is_unverifiable`
- Status of a root daemon by a non-root user → `unsignalable_pid_with_proc_entry_is_unverifiable` (simulates EPERM: `kill -0` fails while the proc entry exists)
- Running daemon replaced on disk → `replaced_daemon_with_deleted_suffix_is_ours`
- Legacy PID file of the same binary → `legacy_pid_of_current_daemon_is_restarted`
- Legacy PID file after a binary path change → `legacy_pid_after_binary_change_is_stopped`
- Legacy PID file of an unrelated process → `legacy_pid_of_unrelated_process_is_kept`
- Re-install migrates an existing registration → `reinstall_over_legacy_registration_replaces_old_daemon` + existing `install` path (`restart` after write)
- Restart after an upgrade of a managed install → existing tests (unchanged)
- Effective definition per backend → existing `effective_binary` tests + `effective_binary_round_trips_the_generated_script`
- Loaded definition differs from the file → existing tests (unchanged)
- Trusted registered binary → existing tests (unchanged)
- BusyBox script generations → `busybox_single_quoted_daemon_yields_exact_path`, `busybox_legacy_double_quoted_daemon_still_parses`
- BusyBox DAEMON with shell expansion → `busybox_daemon_with_expansion_is_unknown` (exit code 3 path: existing "cannot be read from the registration" handling of `Unknown`)
