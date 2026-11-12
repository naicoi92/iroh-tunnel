# unix-service-binary-trust Specification

## Purpose

Define how a root-run service on Linux or macOS (system scope, plus every scope on BusyBox init) runs only a binary that root alone can change. Install registers that binary directly, or a root-owned managed copy of the running executable; CLI start and restart re-check the binary the service manager has loaded; uninstall removes the copy once nothing references it. Service registration and the command surface are owned by `system-service-management`, and the Windows counterpart by `windows-service-trust`.

## Requirements

### Requirement: Root-run Unix registrations trust only root for the binary
The system SHALL, for a registration whose service runs as root on Linux or macOS (system scope on systemd and launchd, every scope on BusyBox init), trust the service binary only when the binary file and every directory above it are owned by root and are not writable by group or others, except for the sticky-directory rule of "Binary check walks the path without following links". The user pinned for the config SHALL NOT be trusted for the binary.

#### Scenario: Binary in a root-owned directory
- **WHEN** the binary is `/usr/local/bin/iroh-tunnel`, owned by root with mode 0755, and every directory above it is owned by root and not group- or world-writable
- **THEN** the binary check passes

#### Scenario: Binary owned by the pinned user
- **WHEN** the binary is owned by the user whose config the service is pinned to
- **THEN** the binary check fails although the same user is trusted for the config

#### Scenario: Binary below a user-owned directory
- **WHEN** the binary file is owned by root but a directory above it is owned by a non-root user
- **THEN** the binary check fails naming that directory, its owner and its mode

#### Scenario: Group-writable binary
- **WHEN** the binary is owned by root with mode 0775
- **THEN** the binary check fails naming the binary path and `0775`

### Requirement: Binary check walks the path without following links
The system SHALL perform the binary check by descending from `/` one component at a time without following symbolic links. A sticky group- or world-writable directory SHALL be accepted only when the next component is owned by root. A path that is relative, contains `.` or `..`, contains a symbolic link, or whose final component is not a regular file SHALL fail the check.

#### Scenario: Symbolic link in the checked path
- **WHEN** the path handed to the check contains a symbolic link
- **THEN** the binary check fails stating the path is not reachable without following a symlink

#### Scenario: Sticky directory with a root-owned child
- **WHEN** the binary lies below a root-owned sticky world-writable directory whose next component is owned by root and every other component passes
- **THEN** the binary check passes

#### Scenario: Sticky directory with a user-owned child
- **WHEN** the binary lies below a sticky world-writable directory whose next component is owned by a non-root user
- **THEN** the binary check fails

### Requirement: System install registers a trusted binary or a managed copy
The system SHALL, when installing a root-run registration on Linux or macOS, canonicalize the resolved binary and apply the root-only binary check to the canonical path. When the check passes, the system SHALL register the canonical path. When it fails, the system SHALL copy the executable that is running the install into the role's managed copy and register that copy. The system SHALL NOT change the owner or mode of the original binary or any of its directories.

#### Scenario: Install from a root-owned location
- **WHEN** `sudo iroh-tunnel serve service install --system` runs and the resolved binary is `/usr/local/bin/iroh-tunnel`, which passes the binary check
- **THEN** the registration runs `/usr/local/bin/iroh-tunnel`, no managed copy is written, and no managed-copy note is printed

#### Scenario: Install from a root-owned symbolic link
- **WHEN** the resolved binary is a root-owned symbolic link to a root-owned regular file in root-owned directories
- **THEN** the registration runs the canonical target path

#### Scenario: Install from a Homebrew prefix
- **WHEN** `sudo iroh-tunnel serve service install --system` runs from a binary under a Homebrew prefix owned by the user
- **THEN** the registration runs the managed copy for serve, the copy is owned by root with mode 0755 and has the same content as the running executable, and the Homebrew binary's owner and mode are unchanged

#### Scenario: Install from a cargo bin directory
- **WHEN** the resolved binary is `~/.cargo/bin/iroh-tunnel` of the invoking user
- **THEN** the registration runs the managed copy and the original file is unchanged

### Requirement: Managed copy location and lifetime
The system SHALL keep one managed copy per role, named `iroh-tunnel`, in `/var/lib/iroh-tunnel/bin/<role>/` on Linux and `/Library/Application Support/iroh-tunnel/bin/<role>/` on macOS. Only `service install` of a root-run registration SHALL write or replace a managed copy, and it SHALL print a note naming the copy, its source, and that `service install --system` must be re-run to update the binary the service runs.

#### Scenario: Install note
- **WHEN** an install registers a managed copy
- **THEN** stdout names the copy path and its source and mentions re-running `service install --system` after upgrading

#### Scenario: Copy per role
- **WHEN** serve and access are both installed with managed copies, and serve is installed again from a different executable
- **THEN** only the serve copy changes and the access registration still runs an unchanged access copy

#### Scenario: Re-install refreshes the copy
- **WHEN** the role already has a managed copy and `service install --system` runs again from a newer executable whose resolved binary still fails the binary check
- **THEN** the managed copy is replaced with the newer executable before the service is restarted

#### Scenario: Re-install from a trusted location
- **WHEN** the role is registered with its managed copy and `service install --system` runs again with a resolved binary that passes the binary check
- **THEN** the registration names the canonical resolved path, the managed copy is not refreshed, and it is removed once no registration of the role names it

### Requirement: Managed directory must be safe before any registration
The system SHALL create missing directories of the managed path as root with mode 0755, SHALL NOT change the owner or mode of an existing directory, and SHALL verify the role's managed directory and the published copy with the root-only check. When a directory fails the check, install SHALL fail with a permission error (exit code 3) naming it, its owner and its mode, and SHALL NOT write or change any registration.

#### Scenario: Managed directory pre-created by a user
- **WHEN** the role's managed directory exists and is owned by a non-root user, and an install needs a managed copy
- **THEN** the command exits with code 3 naming the directory, its owner and its mode, no registration is written, and the directory's owner and mode are unchanged

### Requirement: Managed copy is published link-safely
The system SHALL write the managed copy as a new file created exclusively without following links inside the verified directory, set its mode to exactly 0755 whatever the umask, and rename it into place relative to that directory, so a link planted at the copy's name is replaced rather than written through. A failure before the rename SHALL leave any previous copy unchanged, remove the temporary file, and write no registration.

#### Scenario: Planted link at the copy name
- **WHEN** a symbolic link exists at the managed copy's name and an install writes the copy
- **THEN** the link is replaced by a regular file and the link's target is unchanged

#### Scenario: Copy fails before rename
- **WHEN** writing the new copy fails (for example the disk is full) during a re-install
- **THEN** the command fails, the previous managed copy is unchanged, no temporary file remains, and the registration is unchanged

### Requirement: Install plans without writing a managed copy
The system SHALL decide whether a managed copy is needed before any effect, and SHALL write the managed copy only after every install check has passed and no prompt (scope, invoking user, retarget) is pending. A managed copy written before a later registration step fails SHALL be reported among the changes already made.

#### Scenario: Declined retarget writes no copy
- **WHEN** an install that needs a managed copy stops for a retarget confirmation that is declined
- **THEN** no managed copy is created or changed

#### Scenario: Registration write fails after the copy
- **WHEN** the managed copy was written and writing the registration then fails
- **THEN** the failure lists the managed copy among the changes already applied

### Requirement: CLI start and restart re-check the registered binary
The system SHALL, for `service start` and `service restart` of a root-run registration on Linux or macOS, read the binary the service manager will actually run from its effective (loaded) definition with a read-only query, and apply the root-only binary check before any start, restart or stop command is issued. Start and restart SHALL NOT copy, refresh or replace the registered binary. Without a registration the verb SHALL behave as before.

#### Scenario: Restart after an upgrade of a managed install
- **WHEN** the role runs from its managed copy, the user's Homebrew binary was upgraded, and `service restart --system` runs
- **THEN** the managed copy is unchanged and the service is restarted from it

#### Scenario: Effective definition per backend
- **WHEN** the binary is read for systemd, launchd or BusyBox
- **THEN** it is the loaded `ExecStart` program including drop-ins for systemd, the program of the loaded job for launchd, and `DAEMON` of the init script for BusyBox

#### Scenario: Loaded definition differs from the file
- **WHEN** the loaded launchd job or systemd unit runs a user-writable binary while the file on disk names a trusted one, and `service start --system` runs
- **THEN** the check is applied to the loaded binary and the command exits with code 3

#### Scenario: Trusted registered binary
- **WHEN** the registered binary passes the binary check and `service start --system` runs
- **THEN** the service manager is asked to start the service as before

### Requirement: Unsafe registered binary refuses CLI start and restart
The system SHALL refuse a CLI start or restart whose registered binary fails the check, or cannot be read from the registration, with a permission error (exit code 3) naming the binary path, and its owner and mode where known, and telling the operator to re-run `sudo iroh-tunnel <role> service install --system`; no start, restart or stop command SHALL be issued, so a running service keeps running.

#### Scenario: Legacy registration with a user-writable binary
- **WHEN** a system registration written before this change runs `/home/alice/.cargo/bin/iroh-tunnel` and `sudo iroh-tunnel serve service start --system` runs
- **THEN** the command exits with code 3, stderr names that path and `service install --system`, and only the read-only definition query was run

#### Scenario: Directory loosened after install
- **WHEN** a directory above the registered binary becomes group-writable after install and `service restart --system` runs
- **THEN** the command exits with code 3 naming that directory, and the running service is neither stopped nor restarted

#### Scenario: Unreadable binary in the registration
- **WHEN** the registration is loaded but its effective definition names no single binary the system can extract
- **THEN** the command exits with code 3 telling the operator to re-run `service install --system`

### Requirement: Service-manager starts are not binary-checked
The system SHALL NOT check its own binary when the service process starts, so starts by the service manager at boot, on restart-on-failure or through `launchctl`, `systemctl` or the init script directly run without the binary check. Documentation SHALL NOT describe those starts as binary-checked.

#### Scenario: Boot start of a legacy registration
- **WHEN** a legacy system registration with a user-writable binary is started by the service manager at boot
- **THEN** the service runs as before this change and the run path performs no binary check

### Requirement: Uninstall removes the role's managed copy
The system SHALL, after removing a root-run registration on Linux or macOS, and after an install that registers a trusted binary directly, remove that role's managed copy only when no root-run registration of the role still references it (files, systemd drop-ins and loaded definitions), keeping it whenever that cannot be determined, and then the empty role directory. An absent copy SHALL NOT be an error; the other role's copy SHALL be kept.

#### Scenario: Uninstall with a managed copy
- **WHEN** serve was installed with a managed copy and `service uninstall --system` runs for serve
- **THEN** the serve copy and its empty role directory no longer exist and the access copy, if any, is unchanged

#### Scenario: Another registration still names the copy
- **WHEN** a BusyBox init script and a systemd system unit of the same role both name its managed copy and one of them is uninstalled
- **THEN** the managed copy is kept

#### Scenario: Drop-in still references the copy
- **WHEN** a systemd unit's file does not name the managed copy but a drop-in sets its `ExecStart` to the copy, and another registration of the role is uninstalled
- **THEN** the managed copy is kept

#### Scenario: Uninstall without a managed copy
- **WHEN** the role was registered with a trusted binary and `service uninstall --system` runs
- **THEN** uninstall succeeds as before

### Requirement: Managed copy removal is link-safe
The system SHALL remove a managed copy through a root-only verified directory handle without following links, and SHALL NOT repair the owner or mode of any directory.

#### Scenario: Link planted at the copy name before uninstall
- **WHEN** the managed copy name is a symbolic link when it is removed
- **THEN** the link itself is removed and its target is unchanged

#### Scenario: Cleanup fails after the registration is removed
- **WHEN** the registration was removed and removing the managed copy then fails because its directory is no longer safe
- **THEN** the command fails with a permission error naming the directory, lists the removed registration among the changes already applied, and does not change the directory's owner or mode

### Requirement: BusyBox user-scope requests are root-run registrations
The system SHALL apply the binary rules of this capability to a BusyBox install, start, restart and uninstall requested with user scope, because that request writes the system-wide init script run as root. This SHALL NOT change the config pin, config trust or retarget behaviour of that request.

#### Scenario: BusyBox user-scope install from a user-writable binary
- **WHEN** root runs `service install --user` on a BusyBox host and the resolved binary fails the binary check
- **THEN** the system-wide note is printed, the init script's `DAEMON` names the managed copy, and the config pin behaviour is as before
