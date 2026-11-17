## MODIFIED Requirements

### Requirement: BusyBox init script content and lifecycle
The system SHALL install and restart a mode-0755 BusyBox role init script with pinned config/state, start/stop/restart/status support, and a SIGTERM then 5-second SIGKILL stop policy that applies only to a process confirmed as the daemon by "BusyBox init script verifies process identity" or "BusyBox init script recognises legacy PID files". The script's PID file SHALL hold the daemon's PID on its first line and, on the second, the launched `DAEMON` path followed by a single `.` sentinel so trailing characters of the path survive.
Evidence: crates/host/src/service/backend/busybox_script.rs (render and script execution tests), crates/host/src/service/backend/busybox.rs, tests/service_system.rs

#### Scenario: BusyBox paths and stopped status
- **WHEN** a BusyBox role script is generated
- **THEN** it uses `/etc/init.d/S96iroh-tunnel-<role>`, `/var/run/iroh-tunnel-<role>.pid`, `/var/log/iroh-tunnel-<role>.log`, exports `IROH_TUNNEL_STATE_DIR`, runs `<binary> <role> run --config <config>` in background and returns status 3 when stopped

#### Scenario: Script verbs
- **WHEN** the script is rendered
- **THEN** it begins with `#!/bin/sh`, embeds DAEMON, CONFIG, PIDFILE and LOG, and wires `start`, `stop`, `restart` (stop then start) and `status`

#### Scenario: Start records the launched executable
- **WHEN** `start` launches the daemon
- **THEN** the PID file's first line is the daemon's PID and its remaining content is the `DAEMON` path followed by `.`

#### Scenario: Stop of the running daemon
- **WHEN** the PID file names the daemon this script launched and `stop` runs
- **THEN** the daemon receives SIGTERM, is force-killed only if it is still that daemon after 5 seconds, and the PID file is removed

#### Scenario: PID reused during the stop grace window
- **WHEN** the daemon ignores SIGTERM and, before the 5-second check, its PID no longer runs the recorded executable
- **THEN** no SIGKILL is sent to that PID

## ADDED Requirements

### Requirement: BusyBox init script values are shell literals
The system SHALL render every value interpolated into the BusyBox init script (binary, role, config, state directory, PID file, log and proc paths) as a POSIX shell literal, so that no character is interpreted by the shell and the daemon receives the exact configured paths. The pin record (already percent-escaped) and the numeric trusted uid SHALL be written as before.

#### Scenario: Config path with shell metacharacters
- **WHEN** the config path contains `$(touch <marker>)`, a backtick command, `"`, `'`, `\` and a space, and the rendered script's `start` runs
- **THEN** no marker file is created and the daemon receives the config path byte-for-byte

#### Scenario: State and binary paths with shell metacharacters
- **WHEN** the binary and state paths contain `$name`, a backtick and a space
- **THEN** the daemon is the binary at the literal path and sees `IROH_TUNNEL_STATE_DIR` equal to the literal state path

### Requirement: BusyBox install refuses non-UTF-8 paths
The system SHALL refuse to install a BusyBox script whose binary, config or state path is not valid UTF-8, with a config error (exit code 2) naming the path, before writing anything.

#### Scenario: Non-UTF-8 path refused
- **WHEN** a BusyBox install is rendered with a config path that is not valid UTF-8
- **THEN** install fails with exit code 2 and an error naming the path, and no script is written

### Requirement: BusyBox init script verifies process identity
The BusyBox init script SHALL treat the PID in its PID file as the running daemon only when that process is alive and its executable (`/proc/<pid>/exe`) is the executable recorded in the PID file, with or without the kernel's ` (deleted)` suffix. `status` SHALL NOT modify the PID file.

#### Scenario: Running daemon replaced on disk
- **WHEN** the daemon's executable shows as `<recorded path> (deleted)` because a re-install replaced the managed copy
- **THEN** `start` reports it already running and `stop` stops it

### Requirement: BusyBox init script discards stale PID files
A PID file whose PID is not a positive decimal number without leading zeros, is not a live process, or whose process runs another executable SHALL be stale: `start` and `stop` SHALL remove it without signalling the process, `start` SHALL launch the daemon, and `status` SHALL report not running with exit status 3.

#### Scenario: Stale PID file names an unrelated live process
- **WHEN** the PID file names a live process whose executable is not the recorded daemon, and `stop` runs
- **THEN** the process receives no signal, keeps running, and the PID file is removed

#### Scenario: Start with a stale PID file
- **WHEN** the PID file names a live unrelated process and `start` runs
- **THEN** the daemon is launched and the PID file is rewritten with the daemon's PID and executable

#### Scenario: Status with a stale PID file
- **WHEN** the PID file names a live unrelated process and `status` runs
- **THEN** it reports not running, exits with status 3 and leaves the PID file in place

#### Scenario: Dead or malformed PID file
- **WHEN** the PID file names a PID with no live process, or holds no decimal PID, and `start` runs
- **THEN** the PID file is replaced and the daemon is launched; `status` on such a file reports not running with exit status 3

### Requirement: BusyBox init script refuses an unverifiable identity
When the PID may be live but its executable cannot be confirmed (no `<proc>/<pid>` entry, `readlink` unavailable, or the caller may not signal a process whose `<proc>/<pid>` entry exists), the BusyBox init script SHALL treat the identity as unverifiable: `start` and `stop` SHALL exit with status 1 without signalling and keep the PID file, and `status` SHALL report unknown with exit status 4.

#### Scenario: Unverifiable identity
- **WHEN** the PID file names a live process but no `<proc>/<pid>` entry exists
- **THEN** `stop` and `start` exit with status 1 without signalling it and keep the PID file, and `status` exits with status 4

#### Scenario: Status of a root daemon by a non-root user
- **WHEN** a non-root user runs `status` while the PID file names a live root-owned daemon
- **THEN** `status` reports unknown with exit status 4 and does not claim the daemon is stopped

### Requirement: BusyBox init script recognises legacy PID files
For a PID file with no recorded executable (written by an earlier script), the BusyBox init script SHALL count the process as the daemon when its executable is the current `DAEMON` (with or without ` (deleted)`), or when its executable name starts with `iroh-tunnel` and its command line ends with this role's `run --config <config>` for this script's config; other processes SHALL be stale and an unreadable identity unverifiable.

#### Scenario: Legacy PID file of the same binary
- **WHEN** the PID file holds only a PID whose process runs the current `DAEMON`, and `restart` runs
- **THEN** the old process is stopped and the daemon is started with a PID file in the new format

#### Scenario: Legacy PID file after a binary path change
- **WHEN** the PID file holds only a PID whose process is an `iroh-tunnel` executable at another path running this role with `run --config <config>`, in either the combined (`iroh-tunnel <role> run`) or the standalone (`iroh-tunnel-<role> run`) form, and `restart` runs
- **THEN** the old process is stopped before the daemon is started

#### Scenario: Legacy PID file of an unrelated process
- **WHEN** the PID file holds only a PID whose process is neither the current `DAEMON` nor an `iroh-tunnel` executable running this role and config (for example the other role, or another config)
- **THEN** `stop` does not signal it and `start` launches the daemon

#### Scenario: Re-install migrates an existing registration
- **WHEN** `service install` runs over a BusyBox registration written by an earlier version
- **THEN** the script is rewritten with literal values and identity checks, and its `restart` replaces the old daemon
