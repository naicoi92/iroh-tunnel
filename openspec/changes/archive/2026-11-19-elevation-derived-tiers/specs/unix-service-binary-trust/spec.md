## ADDED Requirements

### Requirement: Root-run Unix registrations trust only root
The system SHALL, for a registration whose service runs as root on Linux or macOS (every system-tier registration on systemd, launchd and BusyBox init), trust the service binary only when the binary file and every directory above it are owned by root and are not writable by group or others, except for the sticky-directory rule of "Binary check walks the path without following links". No non-root user SHALL be trusted for the binary.

#### Scenario: Binary in a root-owned directory
- **WHEN** the binary is `/usr/local/bin/iroh-tunnel`, owned by root with mode 0755, and every directory above it is owned by root and not group- or world-writable
- **THEN** the binary check passes

#### Scenario: Binary owned by a user
- **WHEN** the binary is owned by a non-root user
- **THEN** the binary check fails naming the binary path and its owner

#### Scenario: Binary below a user-owned directory
- **WHEN** the binary file is owned by root but a directory above it is owned by a non-root user
- **THEN** the binary check fails naming that directory, its owner and its mode

#### Scenario: Group-writable binary
- **WHEN** the binary is owned by root with mode 0775
- **THEN** the binary check fails naming the binary path and `0775`

### Requirement: Managed copy is written only after every install check
The system SHALL decide whether a managed copy is needed before any effect, and SHALL write the managed copy only after every install check has passed. A managed copy written before a later registration step fails SHALL be reported among the changes already made.

#### Scenario: Failed check writes no copy
- **WHEN** an install that needs a managed copy fails the system-tier config trust check
- **THEN** no managed copy is created or changed

#### Scenario: Registration write fails after the copy
- **WHEN** the managed copy was written and writing the registration then fails
- **THEN** the failure lists the managed copy among the changes already applied

## MODIFIED Requirements

### Requirement: Service-manager starts are not binary-checked
The system SHALL NOT check its own binary when the service process starts, so starts by the service manager at boot, on restart-on-failure or through `launchctl`, `systemctl` or the init script directly run without the binary check. Documentation SHALL NOT describe those starts as binary-checked. Whether such a start serves is governed by the config rules of `system-service-management`, including "Legacy system registrations are refused with guidance".

#### Scenario: Boot start of a legacy registration
- **WHEN** a system registration written before this change, carrying a trust pin and a user-writable binary, is started by the service manager at boot
- **THEN** the run path performs no binary check and the process exits with the legacy-registration configuration error

#### Scenario: Boot start of a current registration
- **WHEN** a registration written by this version is started by the service manager at boot
- **THEN** the run path performs no binary check

### Requirement: System install registers a trusted binary or a managed copy
The system SHALL, when installing a root-run registration on Linux or macOS, canonicalize the resolved binary and apply the root-only binary check to the canonical path. When the check passes, the system SHALL register the canonical path. When it fails, the system SHALL copy the executable that is running the install into the role's managed copy and register that copy. The system SHALL NOT change the owner or mode of the original binary or any of its directories.

#### Scenario: Install from a root-owned location
- **WHEN** `sudo iroh-tunnel serve service install` runs and the resolved binary is `/usr/local/bin/iroh-tunnel`, which passes the binary check
- **THEN** the registration runs `/usr/local/bin/iroh-tunnel`, no managed copy is written, and no managed-copy note is printed

#### Scenario: Install from a root-owned symbolic link
- **WHEN** the resolved binary is a root-owned symbolic link to a root-owned regular file in root-owned directories
- **THEN** the registration runs the canonical target path

#### Scenario: Install from a Homebrew prefix
- **WHEN** `sudo iroh-tunnel serve service install` runs from a binary under a Homebrew prefix owned by the user
- **THEN** the registration runs the managed copy for serve, the copy is owned by root with mode 0755 and has the same content as the running executable, and the Homebrew binary's owner and mode are unchanged

#### Scenario: Install from a cargo bin directory
- **WHEN** the resolved binary is `~/.cargo/bin/iroh-tunnel` of the invoking user
- **THEN** the registration runs the managed copy and the original file is unchanged

### Requirement: Managed copy location and lifetime
The system SHALL keep one managed copy per role, named `iroh-tunnel`, in `/var/lib/iroh-tunnel/bin/<role>/` on Linux and `/Library/Application Support/iroh-tunnel/bin/<role>/` on macOS. Only `service install` of a root-run registration SHALL write or replace a managed copy, and it SHALL print a note naming the copy, its source, and that `sudo iroh-tunnel <role> service install` must be re-run to update the binary the service runs.

#### Scenario: Install note
- **WHEN** an install registers a managed copy
- **THEN** stdout names the copy path and its source and mentions re-running `sudo iroh-tunnel <role> service install` after upgrading, without `--system`

#### Scenario: Copy per role
- **WHEN** serve and access are both installed with managed copies, and serve is installed again from a different executable
- **THEN** only the serve copy changes and the access registration still runs an unchanged access copy

#### Scenario: Re-install refreshes the copy
- **WHEN** the role already has a managed copy and `sudo iroh-tunnel <role> service install` runs again from a newer executable whose resolved binary still fails the binary check
- **THEN** the managed copy is replaced with the newer executable before the service is restarted

#### Scenario: Re-install from a trusted location
- **WHEN** the role is registered with its managed copy and the elevated `service install` runs again with a resolved binary that passes the binary check
- **THEN** the registration names the canonical resolved path, the managed copy is not refreshed, and it is removed once no registration of the role names it

### Requirement: CLI start and restart re-check the registered binary
The system SHALL, for `service start` and `service restart` of a root-run registration on Linux or macOS, read the binary the service manager will actually run from its effective (loaded) definition with a read-only query, and apply the root-only binary check before any start, restart or stop command is issued. Start and restart SHALL NOT copy, refresh or replace the registered binary. Without a registration the verb SHALL behave as before. For BusyBox, the `DAEMON` value SHALL be read both from scripts that store it as a plain literal (unquoted safe characters, `'…'` segments and `\'`) and from scripts written by earlier versions that store it in double quotes; a value the shell would expand or reinterpret (`$`, backtick, `\`, an inner `"`, whitespace outside quotes, an unterminated quote) SHALL count as unreadable.

#### Scenario: Restart after an upgrade of a managed install
- **WHEN** the role runs from its managed copy, the user's Homebrew binary was upgraded, and `sudo iroh-tunnel serve service restart` runs
- **THEN** the managed copy is unchanged and the service is restarted from it

#### Scenario: Effective definition per backend
- **WHEN** the binary is read for systemd, launchd or BusyBox
- **THEN** it is the loaded `ExecStart` program including drop-ins for systemd, the program of the loaded job for launchd, and `DAEMON` of the init script for BusyBox

#### Scenario: Loaded definition differs from the file
- **WHEN** the loaded launchd job or systemd unit runs a user-writable binary while the file on disk names a trusted one, and `sudo iroh-tunnel serve service start` runs
- **THEN** the check is applied to the loaded binary and the command exits with code 3

#### Scenario: Trusted registered binary
- **WHEN** the registered binary passes the binary check and `sudo iroh-tunnel serve service start` runs
- **THEN** the service manager is asked to start the service as before

#### Scenario: BusyBox script generations
- **WHEN** the init script stores `DAEMON` as a single-quoted literal containing a space and a `'`, or as a legacy double-quoted path
- **THEN** the effective binary is that exact path

#### Scenario: BusyBox DAEMON with shell expansion
- **WHEN** the `DAEMON` value contains an unquoted `$` or backtick, or is a legacy double-quoted value containing `$`, a backtick, `\` or `"`
- **THEN** the effective binary is unknown and CLI start and restart are refused with exit code 3

### Requirement: Unsafe registered binary refuses CLI start and restart
The system SHALL refuse a CLI start or restart whose registered binary fails the check, or cannot be read from the registration, with a permission error (exit code 3) naming the binary path, and its owner and mode where known, and telling the operator to re-run `sudo iroh-tunnel <role> service install`; no start, restart or stop command SHALL be issued, so a running service keeps running.

#### Scenario: Legacy registration with a user-writable binary
- **WHEN** a system registration written before this change runs `/home/alice/.cargo/bin/iroh-tunnel` and `sudo iroh-tunnel serve service start` runs
- **THEN** the command exits with code 3, stderr names that path and `sudo iroh-tunnel serve service install`, and only the read-only definition query was run

#### Scenario: Directory loosened after install
- **WHEN** a directory above the registered binary becomes group-writable after install and `sudo iroh-tunnel serve service restart` runs
- **THEN** the command exits with code 3 naming that directory, and the running service is neither stopped nor restarted

#### Scenario: Unreadable binary in the registration
- **WHEN** the registration is loaded but its effective definition names no single binary the system can extract
- **THEN** the command exits with code 3 telling the operator to re-run `sudo iroh-tunnel <role> service install`

### Requirement: Uninstall removes the role's managed copy
The system SHALL, after removing a root-run registration on Linux or macOS, and after an install that registers a trusted binary directly, remove that role's managed copy only when no root-run registration of the role still references it (files, systemd drop-ins and loaded definitions), keeping it whenever that cannot be determined, and then the empty role directory. An absent copy SHALL NOT be an error; the other role's copy SHALL be kept.

#### Scenario: Uninstall with a managed copy
- **WHEN** serve was installed with a managed copy and `sudo iroh-tunnel serve service uninstall` runs
- **THEN** the serve copy and its empty role directory no longer exist and the access copy, if any, is unchanged

#### Scenario: Another registration still names the copy
- **WHEN** a BusyBox init script and a systemd system unit of the same role both name its managed copy and one of them is uninstalled
- **THEN** the managed copy is kept

#### Scenario: Drop-in still references the copy
- **WHEN** a systemd unit's file does not name the managed copy but a drop-in sets its `ExecStart` to the copy, and another registration of the role is uninstalled
- **THEN** the managed copy is kept

#### Scenario: Uninstall without a managed copy
- **WHEN** the role was registered with a trusted binary and `sudo iroh-tunnel serve service uninstall` runs
- **THEN** uninstall succeeds as before

## REMOVED Requirements

### Requirement: Root-run Unix registrations trust only root for the binary

**Reason**: No pinned user exists any more; replaced by "Root-run Unix registrations trust only root".

**Migration**: None needed.

### Requirement: Install plans without writing a managed copy

**Reason**: Retarget prompts no longer exist; replaced by "Managed copy is written only after every install check".

**Migration**: None needed.

### Requirement: BusyBox user-scope requests are root-run registrations

**Reason**: There are no user-scope requests any more; a non-root BusyBox operation is refused and every root one is a system-tier registration covered by the other requirements.

**Migration**: Run BusyBox service verbs with `sudo`.
