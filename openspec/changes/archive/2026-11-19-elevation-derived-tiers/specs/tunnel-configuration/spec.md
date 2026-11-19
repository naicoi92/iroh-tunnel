## ADDED Requirements

### Requirement: Tier is derived from privilege
The system SHALL run every command in the system tier when the process is root (effective uid 0, including under `sudo`) on Linux or macOS, or holds an elevated token that is a member of Administrators on Windows ("Run as administrator"), and in the user tier otherwise. The tier SHALL NOT be selectable by flag, prompt, environment variable or configuration in production builds.

#### Scenario: Non-root user
- **WHEN** a non-root user runs `iroh-tunnel serve config path`
- **THEN** the user-tier default path is printed

#### Scenario: sudo
- **WHEN** `sudo iroh-tunnel serve config path` runs on Linux
- **THEN** `/etc/iroh-tunnel/serve.toml` is printed, regardless of `HOME` and `SUDO_USER`

#### Scenario: Administrator without elevation
- **WHEN** a member of Administrators runs `iroh-tunnel serve config path` from a non-elevated PowerShell
- **THEN** the user-tier default path is printed

#### Scenario: Run as administrator
- **WHEN** `iroh-tunnel serve config path` runs from an elevated PowerShell
- **THEN** `%ProgramData%\iroh-tunnel\config\serve.toml` is printed

## MODIFIED Requirements

### Requirement: Failed macOS migration never creates a substitute identity
The system SHALL, in the user tier, fail commands (`keygen`, `config set`, `run`) when the legacy configuration exists but cannot be copied, with an error naming the legacy path and stating "no new identity was created", and SHALL NOT create the new config file. The system tier SHALL NOT consult the legacy location.
Evidence: src/paths.rs:116-146, tests/config_paths.rs:249-290

#### Scenario: Unreadable legacy
- **WHEN** the legacy file has mode 000 and `serve config keygen` runs in the user tier
- **THEN** it fails, stderr names the legacy path and contains "no new identity", and the new config does not exist

#### Scenario: System tier ignores the legacy file
- **WHEN** an unreadable legacy file exists in the invoking user's home and `sudo iroh-tunnel serve config keygen` runs on macOS
- **THEN** it succeeds and writes the system-tier config

### Requirement: Default configuration directory per platform
The system SHALL place `<role>.toml`, in the user tier, under `~/.config/iroh-tunnel` on macOS and Linux (ignoring `XDG_CONFIG_HOME`) and under `LocalAppData\iroh-tunnel\config` on Windows. In the system tier it SHALL place it under `/etc/iroh-tunnel` on Linux, `/Library/Application Support/iroh-tunnel/config` on macOS and `%ProgramData%\iroh-tunnel\config` on Windows (the ProgramData known folder), without consulting `HOME`, `SUDO_USER` or any user profile.
Evidence: src/paths.rs:1-10, src/paths.rs:47-54, tests/config_paths.rs:49-68, tests/config_paths_windows.rs:36-48

#### Scenario: Unix default
- **WHEN** `serve config path` or `access config path` runs in the user tier with `HOME` set and `XDG_CONFIG_HOME` pointing elsewhere
- **THEN** it prints `$HOME/.config/iroh-tunnel/<role>.toml` and creates nothing under the XDG directory

#### Scenario: Windows default
- **WHEN** `config path` runs in the user tier on Windows
- **THEN** it prints `<LocalAppData>\iroh-tunnel\config\<role>.toml`

#### Scenario: System tier defaults
- **WHEN** `config path` runs in the system tier
- **THEN** it prints `/etc/iroh-tunnel/<role>.toml` on Linux, `/Library/Application Support/iroh-tunnel/config/<role>.toml` on macOS and `<ProgramData>\iroh-tunnel\config\<role>.toml` on Windows

#### Scenario: System tier ignores the invoking user's home
- **WHEN** `sudo -E iroh-tunnel serve config path` runs with `HOME=/home/alice` and `SUDO_USER=alice`
- **THEN** the printed path does not contain `/home/alice`

### Requirement: Default state directory per platform
The system SHALL place `<role>-status.json`, in the user tier, under `~/.local/state/iroh-tunnel` on macOS and Linux (ignoring `XDG_STATE_HOME`) and under `LocalAppData\iroh-tunnel\state` on Windows. In the system tier it SHALL place it under `/run/iroh-tunnel` on Linux, `/Library/Application Support/iroh-tunnel/state` on macOS (whose `/var/run` is group-writable and fails the system-tier trust walk) and `%ProgramData%\iroh-tunnel\state` on Windows.
Evidence: src/paths.rs:76-95, tests/config_paths.rs:70-80, tests/config_paths_windows.rs:50-67

#### Scenario: Unix status location
- **WHEN** `serve status` runs in the user tier with no status file and a clean HOME
- **THEN** its error names `$HOME/.local/state/iroh-tunnel/serve-status.json`

#### Scenario: System tier status location
- **WHEN** `sudo iroh-tunnel serve status` runs on Linux with no status file
- **THEN** its error names `/run/iroh-tunnel/serve-status.json`

#### Scenario: macOS system tier status location
- **WHEN** `sudo iroh-tunnel serve status` runs on macOS with no status file
- **THEN** its error names `/Library/Application Support/iroh-tunnel/state/serve-status.json`

### Requirement: Unusable home directory is a config error with guidance
The system SHALL, in the user tier, fail with the config-error exit code 2 when the home directory (or LocalAppData on Windows) is missing or not absolute, with a message that names `HOME` and the `--config` / `IROH_TUNNEL_STATE_DIR` overrides, and SHALL NOT create anything. In the system tier an unusable home directory SHALL NOT affect path resolution.
Evidence: src/paths.rs:20-44, tests/config_paths.rs:113-130

#### Scenario: Relative HOME
- **WHEN** `HOME=relative-home` and `serve config path` runs in the user tier
- **THEN** it exits with code 2, stderr mentions `HOME` and `--config`, and no `relative-home` directory is created

#### Scenario: System tier without HOME
- **WHEN** `serve config path` runs in the system tier with `HOME` unset
- **THEN** it succeeds and prints the system-tier path

### Requirement: macOS legacy configuration is migrated by copy
The system SHALL, in the user tier only, copy legacy macOS config bytes to the new default only when absent, retain the original and avoid overwrites under concurrency, with private permissions and a migration notice; explicit `--config` SHALL bypass migration. The system tier SHALL NOT run this migration.
Evidence: src/paths.rs:97-185, tests/config_paths.rs:176-376

#### Scenario: Migration permissions and cleanup
- **WHEN** legacy `~/Library/Application Support/iroh-tunnel/<role>.toml` is migrated
- **THEN** the destination has mode 0600 under a mode-0700 parent, stderr reports migration and no temporary files remain

#### Scenario: Legacy copy
- **WHEN** a legacy config exists and no new config exists and `config show --reveal` runs
- **THEN** output equals the legacy content, the new file has mode 0600, the legacy file and its permissions and mtime are unchanged, and stderr contains "migrated"

#### Scenario: New config wins
- **WHEN** both exist and the legacy file/dir is unreadable
- **THEN** the command succeeds using the new config

#### Scenario: Concurrent migration
- **WHEN** 12 processes run concurrently
- **THEN** all succeed, the new config equals the legacy content, and no leftover files remain

#### Scenario: Explicit config
- **WHEN** `--config` is passed
- **THEN** migration does not run and no new default config is created

#### Scenario: System tier does not migrate
- **WHEN** `sudo iroh-tunnel serve config path` runs on macOS while a legacy user config exists in root's or the invoking user's home
- **THEN** nothing is copied and no migration notice is printed

## REMOVED Requirements

### Requirement: Root service install uses the target user's home paths

**Reason**: The system tier no longer uses any user's home; system services read machine-wide paths.

**Migration**: Create the system-tier config with elevation (`sudo iroh-tunnel <role> config keygen` / `config set`, or copy the old file to the system path with owner root and mode 0600), then re-run `sudo iroh-tunnel <role> service install`.
