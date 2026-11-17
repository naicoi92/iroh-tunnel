## 1. Read DAEMON from both script generations

Blocked by: none

Files: `crates/host/src/service/effective_binary.rs` (owner: this group). Platforms: all (pure parser, unit tests).

- [ ] 1.1 Add failing unit tests in `effective_binary.rs` for scenario "BusyBox script generations": `DAEMON='/a b/it'\''s/iroh-tunnel'` → that exact path; legacy `DAEMON="/a b/iroh-tunnel"` → that path; plain unquoted `DAEMON=/usr/bin/iroh-tunnel` → that path.
- [ ] 1.2 Add failing unit tests for scenario "BusyBox DAEMON with shell expansion": `DAEMON=$HOME/x`, ``DAEMON=`id` ``, `DAEMON='/a`, `DAEMON=/a b`, `DAEMON=`, and legacy `DAEMON="/a/$x"`, ``DAEMON="/a/`id`"``, `DAEMON="/a\b"` → `EffectiveBinary::Unknown`.
- [ ] 1.3 Implement the decoder in `parse_busybox_script` per design D5 (legacy double-quoted branch accepted only without `$`, backtick, `\` or inner `"`; restricted POSIX word otherwise); keep existing tests green.

## 2. Render every script value as a shell literal

Blocked by: 1

Files: `crates/host/src/service/backend/busybox_script.rs` (new, owner: this group), `crates/host/src/service/backend/busybox.rs`, `crates/host/src/service/backend/mod.rs` (owner: this group), reuses `crates/host/src/service/policy.rs::shell_quote` read-only. Platforms: renderer module compiled on Linux and in unix test builds, so execution tests run on macOS `/bin/sh` and Linux CI.

- [ ] 2.1 Create `backend/busybox_script.rs` (`#[cfg(any(target_os = "linux", all(test, unix)))]` in `backend/mod.rs`) and add a test harness in it that renders a script with a temporary `ScriptLayout` (PID file, log, fake proc root) and a fake daemon shell script that records its argv and `IROH_TUNNEL_STATE_DIR` to a file, then `exec sleep`; runs verbs with `/bin/sh` and null stdin; spawns stand-in processes as orphans (double fork) so a stopped process is reaped and not a zombie; and kills every PID it ever saw on drop.
- [ ] 2.2 Add failing execution tests for "Config path with shell metacharacters" (config path with `$(touch pwned)`, `` `touch pwned2` ``, `"`, `'`, `\` and a space; cwd is the temp dir; no `pwned*` file appears; recorded config argument equals the path) and "State and binary paths with shell metacharacters".
- [ ] 2.3 Add a failing unit test for "Non-UTF-8 path refused" (`OsStr::from_bytes(b"/c\xff")` config → `render` error is `CliError::Config` naming the path).
- [ ] 2.4 Introduce `InitScript` and `ScriptLayout` (contracts in plan.md), make `render` return `Result<String>`, quote every interpolated value with `shell_quote`, refuse non-UTF-8 paths; delete `format_script` / `format_script_invocation` and port their tests ("BusyBox paths and stopped status", "Script verbs", invocation forms, managed path with spaces, round-trip through `parse_busybox_script`) to the new API with single-quoted expectations.
- [ ] 2.5 Wire `install` to `InitScript::render(&ScriptLayout::for_role(role))`, propagating the error before `write_registration`.

## 3. Verify process identity before acting on the PID file

Blocked by: 2

Files: `crates/host/src/service/backend/busybox_script.rs` (owner: this group). Platforms: unix execution tests with a fake proc root (macOS and Linux); production uses `/proc` on Linux.

- [ ] 3.1 Add failing execution tests: "Start records the launched executable", "Stop of the running daemon" (both a daemon that exits on SIGTERM and one that ignores it and is force-killed), "PID reused during the stop grace window", "Stale PID file names an unrelated live process" (a real `sleep` child is not signalled and keeps running), "Start with a stale PID file", "Status with a stale PID file" (exit 3, file kept), "Dead or malformed PID file", "Unverifiable identity" (exit 1/1/4, including a live PID that `kill -0` cannot signal while its proc entry exists), "Running daemon replaced on disk" (` (deleted)` target).
- [ ] 3.2 Add failing execution tests for the legacy requirement: "Legacy PID file of the same binary", "Legacy PID file after a binary path change" (combined and standalone invocation forms), "Legacy PID file of an unrelated process", and "Re-install migrates an existing registration" (a fixture copy of the pre-change script starts the daemon and writes a real legacy PID file; the new script's `restart` replaces it).
- [ ] 3.3 Implement the PID file format and the three-way `identity` shell function per design D4 in the script template (POSIX sh, no `local`); make `start`, `stop` (including the re-check before SIGKILL) and `status` use it.
- [ ] 3.4 Run the full verify set (`cargo fmt`, clippy `-D warnings`, `cargo test --all-features --locked`) green.

## 4. Document the hardened BusyBox script

Blocked by: 3

Files: `docs/services.md` (owner: this group). Platforms: n/a.

- [ ] 4.1 Update the "BusyBox and SysV init" section: values are stored as shell literals, `start`/`stop`/`status` only act on the daemon the script launched (stale PID files are removed; an unverifiable PID fails with exit 1, `status` exit 4), existing registrations should be re-installed to pick up the hardened script, a downgrade needs `service stop` first, and a re-install that changes both the binary name and the config may need the old process stopped once by hand.
