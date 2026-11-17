## Context

`crates/host/src/service/backend/busybox.rs` renders `/etc/init.d/S96iroh-tunnel-<role>` with `format!` and interpolates `binary`, `config` and `state` via `to_string_lossy()` inside double quotes. The script runs as root at boot and from `service start|stop|restart`. `crates/host/src/service/effective_binary.rs::parse_busybox_script` reads `DAEMON="<path>"` back for the CLI binary-trust re-check (`unix-service-binary-trust`). `crates/host/src/service/policy.rs::shell_quote` already serializes POSIX shell literals (plain words unquoted, otherwise `'…'` with `'\''`).

Bounded context: platform service registration (host crate, `service` module). The init script is an adapter artifact; its invariants are "every value is a literal" and "only the daemon this script launched is ever signalled". No domain/engine code is involved.

Constraints: the script must run on a minimal BusyBox `sh` (ash) and, for tests on macOS, on `/bin/sh` = bash 3.2 in POSIX mode; no `local`, no `start-stop-daemon`, no `pidof`. Production code follows the no-panic policy (#216). #225 will later refactor the backend parameter lists; this change must not pre-empt it.

## Goals / Non-Goals

**Goals:**
- No interpolated value is ever expanded by the shell; the daemon gets the literal paths.
- `start`/`stop`/`status` act on a PID only after confirming it is the daemon this registration launched.
- Existing registrations migrate by re-install, including re-installs that change the binary path.
- Behaviour proven by executing the rendered script.

**Non-Goals:**
- systemd/launchd/Windows backends; the CLI's duplicate `shell_quote` in `src/render.rs` (#225); the `too_many_arguments` refactor (#225); using the ownership lock from the script.

## Decisions

### D1. Quote with the existing host `shell_quote`
Every interpolated value (`DAEMON`, `ROLE`, `CONFIG`, `IROH_TUNNEL_STATE_DIR`, `PIDFILE`, `LOG`, `PROC`) is rendered as `NAME=<shell_quote(value)>`. The pin comment is already percent-escaped by `Pin::record` and the trusted uid is numeric, so both stay as they are. Rejected: refusing metacharacters (denylist, rejects legal paths).

### D2. Non-UTF-8 paths are refused at render time
The script is written as `&str`; `to_string_lossy` would silently register a different path. `render` returns `Result` and fails with `CliError::Config("<what> path is not valid UTF-8: <display>")` (exit code 2, consistent with other install input errors) before anything is written. Assumption recorded: such paths do not work today either (lossy), so this turns a silent misconfiguration into a clear error. Rejected: byte-level rendering (needs a non-`str` registration writer for a case that has never worked).

### D3. Renderer seam: `InitScript` + `ScriptLayout`
Replace `format_script` / `format_script_invocation` with a value type:

- `InitScript<'a> { role, invocation, binary, config, state, pin }` — what the registration runs.
- `ScriptLayout { pidfile, logfile, proc_root }` — host locations; `ScriptLayout::for_role(role)` gives `/var/run/iroh-tunnel-<role>.pid`, `/var/log/iroh-tunnel-<role>.log`, `/proc`.
- `InitScript::render(&self, &ScriptLayout) -> Result<String>`.

`install` builds the `InitScript` from its existing arguments (its signature is left for #225). The layout exists so tests can execute the script against a temporary PID/log path and a fake proc root on macOS and Linux; production always uses `for_role`. No environment-variable override is added to the root script.

`InitScript`, `ScriptLayout` and the template live in a new module `backend/busybox_script.rs`, compiled for `cfg(any(target_os = "linux", all(test, unix)))`, while the effectful backend (`busybox.rs`) stays Linux-only. Rendering is pure and the script is plain POSIX sh, so its execution tests run on macOS too; compiling the whole backend on macOS was rejected because every effectful item would need a dead-code allowance.

### D4. PID file format and identity check (shell)
- `start` writes `printf '%s\n%s.' "$!" "$DAEMON" > "$PIDFILE"`: line 1 the PID, the rest the launched executable followed by a `.` sentinel so a path's trailing characters survive command substitution. (Considered: a separate `$PIDFILE.exe` file so older scripts could still read the PID; rejected because it doubles the stale-state surface, and downgrade is handled in Migration Plan.)
- Reading: `PID=$(head -n 1)`, `REC=$(tail -n +2)`. `REC` empty → legacy file; otherwise strip the sentinel.
- `identity` (a shell function) returns 0 = ours, 1 = stale, 2 = unverifiable:
  1. `PID` must be a positive decimal without leading zeros (`kill -0 0` targets the process group), else stale.
  2. If `kill -0 "$PID"` fails: unverifiable when `$PROC/$PID` exists (EPERM for a non-root caller on a root daemon), else stale (dead process).
  3. If `$PROC/$PID` does not exist → unverifiable. A zombie (state `Z` after the last `) ` of a readable `$PROC/$PID/stat`) → stale. If `readlink "$PROC/$PID/exe"` fails → unverifiable. A readable link that does not match → stale.
  4. New format: `EXE` equals `REC` or `"$REC (deleted)"` (the kernel suffix after the managed copy is replaced by rename on re-install).
  5. Legacy format: `EXE` equals `$DAEMON` or `"$DAEMON (deleted)"`; or the basename `NAME` of `EXE` (with ` (deleted)` removed) starts with `iroh-tunnel` and `/proc/$PID/cmdline`, read with NULs translated to `\001`, ends with `<SEP>run<SEP>--config<SEP>$CONFIG<SEP>` and the role is identified: either the word before `run` is `$ROLE` (combined form) or `NAME` is `iroh-tunnel-$ROLE` (standalone form). Both forms are accepted whatever the current invocation is, so a re-install that switches between combined and standalone still recognises the old daemon. Quoted variables inside `case` patterns are matched literally.
- `start`: ours → "already running", exit 0; unverifiable → message, exit 1, PID file kept, nothing launched; stale or no file → remove and launch.
- `stop`: ours → SIGTERM, wait up to 5 s while ours, then SIGKILL only if still ours (guards a PID reused during the grace window), remove the PID file; unverifiable → message, exit 1, PID file kept; stale → remove, exit 0.
- `status`: ours → running (exit 0); stale → "not running" (exit 3); unverifiable → "unknown" (exit 4, LSB "status unknown"). Never modifies the PID file (status needs no root).
- Unverifiable vs stale: a stale verdict deletes state and may lead to a second launch, so it requires positive evidence (dead PID, garbage, or a readable different exe). Missing `/proc` or `readlink` fails loud instead of silently orphaning the daemon. A non-root `status` on a root daemon gets EPERM from `kill -0` while `/proc/<pid>` exists, and so reports unknown (exit 4) rather than a wrong "not running".
- `$!` is the PID of the process that execs `$DAEMON` (simple command run in background), so `/proc/$!/exe` is the daemon once it has exec'd; the record uses `$DAEMON`, not a racy readlink. Registered paths are canonical (`unix-service-binary-trust`), so `exe` equals `DAEMON` for scripts written by this version.

Rejected: comparing only with the current `$DAEMON` (breaks binary path changes on re-install); ownership lock via `status --json` (public CLI change). Chosen with the user in brainstorm.

### D5. `parse_busybox_script` reads both generations
Find the line starting with `DAEMON=`. If the value starts with `"`, keep the legacy behaviour (strip the surrounding quotes, verbatim). Otherwise decode a restricted POSIX word: plain characters, `'…'` segments and `\'`; anything else (`$`, `` ` ``, unquoted `"`, whitespace, unterminated quote) → `Unknown`. Round-trip property: `decode(shell_quote(s)) == s` for the paths used in tests.

## Risks / Trade-offs

- [`/proc` missing or `readlink` unavailable] → unverifiable: nothing is signalled, `start`/`stop` fail visibly (exit 1) with the PID file kept; the operator can stop the process manually. No silent orphaning, no wrong-process kill.
- [A path ending in a newline] → the readlink comparison does not match → stale; the old daemon (if any) then refuses the new start through the ownership lock (visible error). Accepted.
- [Legacy daemon not recognised: a standalone executable not named exactly `iroh-tunnel-<role>`, or a re-install that also changes `--config`] → treated as stale; the new start is refused by the ownership lock (visible error) and the operator stops the old process once. Accepted and documented.
- [Race between `&` and the daemon's exec] → for a few milliseconds `/proc/$!/exe` is the shell, so a `status`/`stop` issued immediately after `start` sees "stale". Accepted: init verbs are not issued concurrently; the ownership lock still prevents a duplicate daemon.
- [Legacy fallback matches another iroh-tunnel process] → it must also carry this role and exactly this config's `run --config`; that is the same role instance by construction.
- [Tests use a fake proc root, not a real kernel] → logic covered on macOS and Linux CI; a real BusyBox run is a manual/E2E check (see plan).
- [Scripts already installed stay vulnerable until re-install] → documented in `docs/services.md` and the CHANGELOG entry produced by the release tooling (commit message).

## Migration Plan

`service install` rewrites the script and runs `restart`; the new `stop` recognises the old daemon through the legacy check and the new `start` writes the new PID format. Rollback: run `service stop` with the current version first, then re-install with the previous version. Without the stop, the previous script reads the two-line PID file via `$(cat …)` as a non-numeric PID, cannot stop the running daemon, and its start is refused by the ownership lock, leaving the daemon unmanaged until it is stopped manually.

## Open Questions

- None.
