## MODIFIED Requirements

### Requirement: Failed macOS migration never creates a substitute identity
The system SHALL fail commands (`keygen`, `config set`, `run`) when the legacy configuration exists but cannot be copied, with an error naming the legacy path and stating "no new identity was created", and SHALL NOT create the new config file.
Evidence: src/paths.rs:116-146, tests/config_paths.rs:249-290

#### Scenario: Unreadable legacy
- **WHEN** the legacy file has mode 000 and `serve config keygen` runs
- **THEN** it fails, stderr names the legacy path and contains "no new identity", and the new config does not exist

### Requirement: Random key generation prints an identity without touching configuration
The system SHALL, for `<role> config keygen --print`, print a `Secret key:` and a matching `Node ID:`, differing on every run, SHALL NOT create or modify any configuration or directory, SHALL NOT echo the secret to stderr, and SHALL reject `--config` or `--yes` combined with `--print`.
Evidence: tests/config_paths.rs:402-471

#### Scenario: Existing config untouched
- **WHEN** `serve config keygen --print` runs with an existing config
- **THEN** the key decodes to the printed Node ID, the config and HOME listing are unchanged

#### Scenario: Conflicting option
- **WHEN** `--config` and `--print` are both given
- **THEN** the command fails with a "cannot be used with" error and creates nothing
