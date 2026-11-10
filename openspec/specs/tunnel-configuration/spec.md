# tunnel-configuration Specification

## Purpose

Define the TOML role configuration documents (services, peers, relays, secret key, UDP flow settings) and their validation, atomic saving, and default or overridden configuration and state locations per platform. The `config` subcommands that edit these files are owned by `config-management-commands`.

## Requirements

### Requirement: Role configuration documents are loaded from TOML and validated
The system SHALL load a serve or access configuration from a TOML file, apply defaults to absent sections, and reject a file that cannot be read, cannot be parsed, or fails validation, naming the file path in the error.
Evidence: src/config.rs:318-338, src/config.rs:44-48, src/config.rs (unit tests serve_load_valid, serve_load_missing_file_errors, serve_load_bad_toml_errors), tests/config_paths.rs:140-173

#### Scenario: Minimal serve config gets defaults
- **WHEN** a serve config contains a service with only name, protocol and port
- **THEN** the service host defaults to `127.0.0.1`

#### Scenario: Missing file
- **WHEN** the config path does not exist
- **THEN** loading fails with "failed to read config" including the path

#### Scenario: Malformed TOML
- **WHEN** the file is not valid TOML
- **THEN** loading fails with "failed to parse config" including the path, exits with the config-error code 2, and the file is left unchanged

### Requirement: Service names are restricted to ALPN-safe identifiers
The system SHALL reject any service name that does not match `^[a-z0-9-]+$` or is longer than 63 bytes, and SHALL reject duplicate service names within one role configuration.
Evidence: src/config.rs:22-29, src/config.rs:448-456, src/config.rs (unit tests validation_rejects_uppercase_name, validation_rejects_duplicate_name)

#### Scenario: Uppercase name
- **WHEN** a service is named `Postgres`
- **THEN** validation fails with "invalid service name"

#### Scenario: Duplicate name
- **WHEN** two services share the name `db`
- **THEN** validation fails with "duplicate service name"

### Requirement: Service ports must be non-zero
The system SHALL reject a service whose port is 0 in both roles.
Evidence: src/config.rs:458-463, src/config.rs (unit test validation_rejects_zero_port)

#### Scenario: Zero port
- **WHEN** a service has port 0
- **THEN** validation fails with "invalid port: must be 1-65535"

### Requirement: Service protocol is tcp or udp with a loopback default host
The system SHALL accept `protocol` values `tcp` and `udp` (lowercase) on every service, and SHALL default `host` to `127.0.0.1` when absent.
Evidence: src/config.rs:44-48, src/config.rs:263-268, src/config.rs (unit tests protocol_serializes_lowercase, serve_load_valid)

#### Scenario: Protocol serialization
- **WHEN** a TCP or UDP service is serialized
- **THEN** the output contains `protocol = "tcp"` or `protocol = "udp"`

### Requirement: Relay URLs must use https
The system SHALL reject any relay URL in `[node] relay_urls`, in either role, and in an access service's `relay_urls` override, that does not start with `https://`; the service-level error names the service.
Evidence: src/config.rs:467-478, src/config.rs:502-535, src/config.rs (unit tests validation_rejects_non_https_relay, access_validation_rejects_non_https_service_relay)

#### Scenario: Insecure node relay
- **WHEN** a config lists `http://insecure.relay` in `[node] relay_urls`
- **THEN** validation fails with "invalid relay_url"

#### Scenario: Insecure per-service relay
- **WHEN** an access service `web` lists an `http://` relay URL
- **THEN** validation fails with "invalid relay_url" and the message names `'web'`

### Requirement: Access services pin the serve peer by node id and may override relays
The system SHALL require each access service to carry a `node_id` of 52 characters (base32) or 64 characters (hex), and SHALL accept an optional per-service `relay_urls` list that, when empty or absent, leaves the node-level relays in effect.
Evidence: src/config.rs:31-40, src/config.rs:276-292, src/config.rs:515-520, examples/access.toml:1-35, src/config.rs (unit tests access_validation_rejects_short_node_id, access_validation_accepts_hex_node_id)

#### Scenario: Short node id
- **WHEN** an access service has `node_id = "tooshort"`
- **THEN** validation fails with "invalid node_id"

#### Scenario: Hex node id
- **WHEN** an access service has a 64-character node_id
- **THEN** validation succeeds

### Requirement: Relay token is a single non-blank, whitespace-free secret
The system SHALL accept an optional `[node] relay_token` and SHALL reject a token that is empty, blank, or contains whitespace.
Evidence: src/config.rs:74-100, src/config.rs (unit tests relay_token_validation_rejects_empty_and_whitespace, relay_token_accepts_plain_secret), examples/serve.toml:13-17

#### Scenario: Whitespace token
- **WHEN** `relay_token` is `""`, `"   "` or `"has space"`
- **THEN** validation fails mentioning `relay_token`

#### Scenario: Plain token
- **WHEN** `relay_token` is `deadbeef1234`
- **THEN** validation succeeds

### Requirement: Concurrent stream budget is optional and at least 1
The system SHALL treat `[node] max_concurrent_streams` as optional (absent means no override) and SHALL reject the value 0 in both roles.
Evidence: src/config.rs:62-72, src/config.rs:103-108, src/config.rs (unit tests node_max_concurrent_streams_defaults_to_none, node_max_concurrent_streams_parses, node_max_concurrent_streams_rejects_zero)

#### Scenario: Absent
- **WHEN** the field is absent
- **THEN** it is unset and validation passes

#### Scenario: Zero
- **WHEN** the value is 0 for serve or access
- **THEN** validation fails mentioning `max_concurrent_streams`

### Requirement: UDP flow settings apply only to UDP services and are range-checked
The system SHALL accept optional `idle_timeout`, `max_flows`, `max_flows_per_peer` and `queue_depth` only on UDP services, SHALL reject 0 and values above the limits (`idle_timeout` 86400, `max_flows` and `max_flows_per_peer` 1,000,000, `queue_depth` 65,536), SHALL reject `max_flows_per_peer` greater than `max_flows`, and SHALL reject `max_flows_per_peer` and `queue_depth` on access services.
Evidence: src/config.rs:110-134, src/config.rs:186-250, src/config.rs (unit tests)

#### Scenario: Setting on TCP service
- **WHEN** a TCP service sets `max_flows`
- **THEN** validation fails stating the settings apply only to `protocol = "udp"`

#### Scenario: Out-of-range value
- **WHEN** a UDP service sets `queue_depth = 65537` (or 0)
- **THEN** validation fails naming `queue_depth`; `65536` is accepted

#### Scenario: Access service per-peer fields
- **WHEN** an access UDP service sets `queue_depth` or `max_flows_per_peer`
- **THEN** validation fails naming the field, while `max_flows` is accepted

### Requirement: UDP flow settings have documented defaults
The system SHALL resolve absent UDP settings to an idle timeout of 60 seconds, 256 maximum flows, 64 flows per peer and a queue depth of 64, SHALL clamp the default per-peer cap to an explicitly smaller `max_flows`, and SHALL omit unset UDP settings when the configuration is serialized.
Evidence: src/config.rs:136-184, src/config.rs (unit tests)

#### Scenario: Defaults
- **WHEN** a UDP service sets none of the four fields
- **THEN** limits resolve to 60 s, 256, 64 and 64

#### Scenario: Small cap clamps default
- **WHEN** only `max_flows = 10` is set
- **THEN** the effective per-peer cap is 10

### Requirement: Legacy multiplex field is ignored and dropped on save
The system SHALL ignore a legacy `multiplex` field on access services when loading and SHALL NOT write it back when the configuration is saved, preserving all other settings.
Evidence: src/config.rs:255-262, tests/config_multiplex_compat.rs:1-60, tests/config_multiplex_compat.rs:146-260, examples/access.toml:25

#### Scenario: Legacy value on load
- **WHEN** an access service contains `multiplex = true`, `false` or `"auto"`
- **THEN** the file loads and, after a config command saves it, no `multiplex` key remains and other settings are unchanged

#### Scenario: Rejected edit
- **WHEN** a config command is rejected
- **THEN** the original legacy file and key are preserved

### Requirement: Secret key is a base64 32-byte value generated and persisted on first use
The system SHALL interpret `[node] secret_key` as base64 of exactly 32 bytes, SHALL generate a fresh key when it is empty and persist it back into the config file (for both serve and access roles, so the node id is stable across restarts), SHALL reuse an existing key without rewriting the file, and SHALL reject invalid base64 or a wrong length.
Evidence: src/config.rs:348-366, src/config.rs:419-441, src/config.rs (unit tests)

#### Scenario: Empty key
- **WHEN** `secret_key` is empty and the key is resolved
- **THEN** a new key is generated and written to the file, and reloading yields the same public key

#### Scenario: Bad key
- **WHEN** `secret_key` is not base64, or decodes to other than 32 bytes
- **THEN** an error "invalid secret_key" (not valid base64 / expected 32 bytes) is reported

### Requirement: Saving a configuration is atomic, locked and round-trips
The system SHALL write a saved configuration as pretty TOML atomically and owner-only under a writer lock, and a saved configuration SHALL reload to an equal value.
Evidence: src/config.rs:338-346, src/config.rs (unit test save_roundtrip_writes_file), tests/config_multiplex_compat.rs:221-250

#### Scenario: Round trip
- **WHEN** a serve configuration is saved and loaded
- **THEN** the loaded value equals the saved one

#### Scenario: Write failure
- **WHEN** the target cannot be written (read-only file)
- **THEN** the error is reported and the existing file is preserved

### Requirement: Shipped example configurations describe the demo pair
The system SHALL ship `examples/serve.toml` (throwaway demo `secret_key`, `web` TCP service on host `web` port 80) and `examples/access.toml` (empty `relay_urls`, `web` service exposed on `0.0.0.0:8080` that dials that serve node id) as a consistent compose demo.
Evidence: examples/serve.toml:1-24, examples/access.toml:1-35

#### Scenario: Demo pairing
- **WHEN** the access example is used with the serve example
- **THEN** the access `node_id` equals the NodeId derived from the serve example key, per the file comments

### Requirement: Default configuration directory per platform
The system SHALL place `<role>.toml` under `~/.config/iroh-tunnel` on macOS and Linux (ignoring `XDG_CONFIG_HOME`) and under `LocalAppData\iroh-tunnel\config` on Windows.
Evidence: src/paths.rs:1-10, src/paths.rs:47-54, tests/config_paths.rs:49-68, tests/config_paths_windows.rs:36-48

#### Scenario: Unix default
- **WHEN** `serve config path` or `access config path` runs with `HOME` set and `XDG_CONFIG_HOME` pointing elsewhere
- **THEN** it prints `$HOME/.config/iroh-tunnel/<role>.toml` and creates nothing under the XDG directory

#### Scenario: Windows default
- **WHEN** `config path` runs on Windows
- **THEN** it prints `<LocalAppData>\iroh-tunnel\config\<role>.toml`

### Requirement: Default state directory per platform
The system SHALL place `<role>-status.json` under `~/.local/state/iroh-tunnel` on macOS and Linux (ignoring `XDG_STATE_HOME`) and under `LocalAppData\iroh-tunnel\state` on Windows.
Evidence: src/paths.rs:76-95, tests/config_paths.rs:70-80, tests/config_paths_windows.rs:50-67

#### Scenario: Unix status location
- **WHEN** `serve status` runs with no status file and a clean HOME
- **THEN** its error names `$HOME/.local/state/iroh-tunnel/serve-status.json`

### Requirement: Explicit overrides win over default locations
The system SHALL use the `--config` path instead of the default config file, and the `IROH_TUNNEL_STATE_DIR` directory instead of the default state directory, without touching the defaults or running macOS migration.
Evidence: src/paths.rs:5-8, tests/config_paths.rs:82-111, tests/config_paths.rs:380-400, tests/config_paths_windows.rs:69-112

#### Scenario: Config override
- **WHEN** `serve config keygen --config <custom>` runs
- **THEN** the file is created at the custom path and not at the default path

#### Scenario: State override
- **WHEN** `access status` runs with `IROH_TUNNEL_STATE_DIR=<dir>`
- **THEN** the error names `<dir>/access-status.json`

### Requirement: Unusable home directory is a config error with guidance
The system SHALL fail with the config-error exit code 2 when the home directory (or LocalAppData on Windows) is missing or not absolute, with a message that names `HOME` and the `--config` / `IROH_TUNNEL_STATE_DIR` overrides, and SHALL NOT create anything.
Evidence: src/paths.rs:20-44, tests/config_paths.rs:113-130

#### Scenario: Relative HOME
- **WHEN** `HOME=relative-home` and `serve config path` runs
- **THEN** it exits with code 2, stderr mentions `HOME` and `--config`, and no `relative-home` directory is created

### Requirement: Unreadable or invalid default config reports its path
The system SHALL name the config file path in the error when the default configuration cannot be read or parsed, and SHALL leave the file unchanged.
Evidence: tests/config_paths.rs:132-173

#### Scenario: Permission denied
- **WHEN** the config file has mode 000
- **THEN** `serve config list` fails and stderr contains the path

### Requirement: macOS legacy configuration is migrated by copy
The system SHALL copy legacy macOS config bytes to the new default only when absent, retain the original and avoid overwrites under concurrency, with private permissions and a migration notice; explicit `--config` SHALL bypass migration.
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

### Requirement: Failed macOS migration never creates a substitute identity
The system SHALL fail commands (`keygen`, `config set`, `run`) when the legacy configuration exists but cannot be copied, with an error naming the legacy path and stating "no new identity was created", and SHALL NOT create the new config file.
Evidence: src/paths.rs:116-146, tests/config_paths.rs:249-290

#### Scenario: Unreadable legacy
- **WHEN** the legacy file has mode 000 and `serve config keygen` runs
- **THEN** it fails, stderr names the legacy path and contains "no new identity", and the new config does not exist

### Requirement: Root service install uses the target user's home paths
The system SHALL, on Unix, provide config path `<home>/.config/iroh-tunnel/<role>.toml` and state dir `<home>/.local/state/iroh-tunnel` for an explicit home, without consulting `HOME` or migrating.
Evidence: src/paths.rs:56-74
Confidence: inferred

#### Scenario: Explicit home
- **WHEN** a system service is installed for another user's account
- **THEN** that user's home is used and the installer's home is never substituted

### Requirement: Random key generation prints an identity without touching configuration
The system SHALL, for `<role> config keygen --print`, print a `Secret key:` and a matching `Node ID:`, differing on every run, SHALL NOT create or modify any configuration or directory, SHALL NOT echo the secret to stderr, and SHALL reject `--config` or `--yes` combined with `--print`.
Evidence: tests/config_paths.rs:402-471

#### Scenario: Existing config untouched
- **WHEN** `serve config keygen --print` runs with an existing config
- **THEN** the key decodes to the printed Node ID, the config and HOME listing are unchanged

#### Scenario: Conflicting option
- **WHEN** `--config` and `--print` are both given
- **THEN** the command fails with a "cannot be used with" error and creates nothing

