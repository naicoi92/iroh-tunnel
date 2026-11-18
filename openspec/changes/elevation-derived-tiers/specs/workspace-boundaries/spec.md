## MODIFIED Requirements

### Requirement: Exit classification survives package boundaries
`CliError` SHALL live in `iroh-tunnel-host` and keep its downcast identity through `anyhow` context, so the CLI maps success 0, unclassified 1, config 2, permission 3, Iroh 4, service 5 and conflict 6. `iroh-tunnel-engine` MUST NOT import `CliError`.
Source: refactor-codebase design.md Decision 8; openspec/specs/cli-entrypoints

#### Scenario: Permission failure keeps exit code 3
- **WHEN** `sudo iroh-tunnel serve service install --config <path>` runs and the config is owned by a non-root user
- **THEN** the process exits with code 3

#### Scenario: Ownership conflict keeps exit code 6
- **WHEN** `iroh-tunnel serve run --config <valid config>` starts while another Serve role of the same user is running in the user tier
- **THEN** the process exits with code 6

#### Scenario: Invalid configuration keeps exit code 2
- **WHEN** `iroh-tunnel serve config list` loads a config file that cannot be parsed
- **THEN** the process exits with code 2
