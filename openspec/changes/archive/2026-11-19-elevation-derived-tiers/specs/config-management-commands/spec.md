## MODIFIED Requirements

### Requirement: Config file target resolution
The system SHALL operate every `config` subcommand on the file given by `-c/--config`, and SHALL fall back to the role's default config path of the invoking tier only when `--config` is absent.
Evidence: src/config_cmd.rs:360-372, src/config_cmd.rs:326-331, tests/config_lifecycle.rs:577-636, tests/config_lifecycle_windows.rs:247

#### Scenario: Explicit path wins
- **WHEN** keygen, set, rm, list, show, path, check and edit are each run with `--config custom.toml`
- **THEN** all succeed against `custom.toml` and `config path` prints exactly that path
- **AND** the default config file and default directory are not created or touched

#### Scenario: Access role uses the same flag
- **WHEN** `access config set -c access.toml -n web -l 8080 -N <id>` is run
- **THEN** the entry is written to `access.toml`

#### Scenario: Elevated keygen writes the system config
- **WHEN** `sudo iroh-tunnel serve config keygen` runs on Linux and `/etc/iroh-tunnel` does not exist
- **THEN** `/etc/iroh-tunnel` is created owned by root with mode 0755, `/etc/iroh-tunnel/serve.toml` is created owned by root with mode 0600, and nothing is written in the invoking user's home

### Requirement: Saves are atomic and owner-only
The system SHALL write config files via a sibling temp file renamed into place so that readers or a killed writer never observe a truncated or partial document, SHALL create config and lock files with mode 0600 on Unix, and SHALL refuse to overwrite a read-only config file. A config written by an elevated process SHALL be owned by root (Unix) or inherit the admin-only DACL of the system-tier config directory (Windows); an elevated write SHALL be refused, before writing, when that directory grants read access to non-admin principals.
Evidence: src/config_store.rs:128-175, src/config_store.rs:67-76, tests/config_lifecycle.rs:280-338, tests/config_lifecycle.rs:191-210, tests/config_lifecycle.rs:424-433

#### Scenario: Killed writer
- **WHEN** `config set` processes are killed at random points while a reader polls the file
- **THEN** the reader always sees a complete valid TOML document

#### Scenario: Permissions tightened
- **WHEN** a valid edit or key write is saved to a file that was mode 0644
- **THEN** the file is mode 0600 afterwards (Unix)

#### Scenario: Read-only target
- **WHEN** a mutation targets a read-only config file
- **THEN** it fails with a message that the file is read-only and the file is unchanged

#### Scenario: Non-elevated user cannot read the system config
- **WHEN** a system-tier config was written by an elevated process and a non-elevated user runs `cat` on it (Unix) or opens it (Windows)
- **THEN** access is denied
