## MODIFIED Requirements

### Requirement: Update acts only on the invoked executable
The system SHALL provide `update` (options `--to VERSION`, `-i/--insecure`, `-y/--yes`, `--force`) at top level of `iroh-tunnel` and as a role command of standalone binaries, replacing only the running executable, and SHALL reject `<role> update` on the all-in-one binary with a config error telling the user to run `iroh-tunnel update`. It SHALL reject `--force` without `--to` as a usage error with exit code 2.
Evidence: src/cli.rs:56-73,213-214, src/app.rs:111-113,209-218
Confidence: inferred

#### Scenario: All-in-one role update
- **WHEN** `iroh-tunnel serve update` is run
- **THEN** it exits 2 with "update acts on the invoked executable"

#### Scenario: Standalone update
- **WHEN** `its update` is run
- **THEN** the update runs for executable name `iroh-tunnel-serve`

#### Scenario: Force without a version
- **WHEN** `iroh-tunnel update --force` is run
- **THEN** it exits 2 with a usage error naming `--to`, and nothing is downloaded or replaced
