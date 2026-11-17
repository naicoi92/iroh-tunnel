## Why

The BusyBox/SysV init script that `service install` writes runs as root, yet it embeds binary, config and state paths verbatim inside double quotes, so a config path from a user-owned directory containing `$(...)` or backticks runs commands as root (#220). Its start/stop/status logic also trusts a bare PID file, so after a crash `stop` can SIGTERM/SIGKILL an unrelated process that reused the PID and `start` can refuse to launch the daemon.

## What Changes

**BusyBox script values**
- From: paths are interpolated verbatim as `DAEMON="<path>"`, `CONFIG="<path>"`, `export IROH_TUNNEL_STATE_DIR="<path>"`.
- To: every interpolated value is a POSIX single-quoted literal (shared host `shell_quote`); the daemon receives the literal path; a path that is not valid UTF-8 is refused at install with a config error (exit 2) instead of being silently altered.
- Reason: close root command injection without restricting legal paths.
- Impact: non-breaking; re-install rewrites existing scripts.

**BusyBox process identity**
- From: any live PID recorded in `/var/run/iroh-tunnel-<role>.pid` counts as the running daemon.
- To: the PID file records the PID and the launched executable; a PID counts only when `/proc/<pid>/exe` is that executable (or the same path marked ` (deleted)`). PID files from older scripts are accepted only when the process runs the current `DAEMON`, or an `iroh-tunnel*` executable whose command line carries this role's `run --config <config>`. A dead, malformed or foreign PID is a stale PID file: removed by `start`/`stop`, never signalled, reported as not running. A live PID whose executable cannot be read is unverifiable: nothing is signalled, `start`/`stop` fail (exit 1) and `status` reports unknown (exit 4).
- Reason: never signal an unrelated process; never block start on a stale PID.
- Impact: non-breaking; re-install over a running legacy registration still stops the old daemon, including when the binary path changes.

**Registered binary read-back**
- From: the BusyBox effective binary is read only from `DAEMON="<path>"`.
- To: it is read from both the single-quoted form and the legacy double-quoted form.

Non-goals: the systemd, launchd and Windows backends; merging the CLI's own `shell_quote` (`src/render.rs`) and the parameter-object refactor (#225); `start-stop-daemon`; ownership lock changes.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `system-service-management`: "BusyBox init script content and lifecycle" changes (PID file format, identity-checked stop); new requirements for literal script values, non-UTF-8 refusal, process identity, stale PID files, unverifiable identity and legacy PID files.
- `unix-service-binary-trust`: "CLI start and restart re-check the registered binary" — the BusyBox effective definition is read from both script generations.

## Impact

- Code: `crates/host/src/service/backend/busybox.rs` (renderer, script template, tests that execute the script), `crates/host/src/service/effective_binary.rs` (`parse_busybox_script`), reuse of `crates/host/src/service/policy.rs::shell_quote`.
- Docs: `docs/services.md` BusyBox section.
- Security: removes a local root command injection and a wrong-process signalling path on non-systemd Linux hosts. No new dependency, no public CLI or API change.
- Compatibility: scripts already on disk keep their old behaviour until re-installed; `service install` rewrites them and its `restart` stops the old daemon through the legacy check. Downgrading needs `service stop` first (the older script cannot read the new PID file).
