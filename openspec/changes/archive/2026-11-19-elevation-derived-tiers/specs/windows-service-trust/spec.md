## ADDED Requirements

### Requirement: Only admin principals are trusted for a Windows system service
The system SHALL, for a Windows system service, trust only SYSTEM, the Administrators group and TrustedInstaller for the config file, the state directory, the registered service binary and every directory above each of them. No user SID SHALL be trusted for any of them.

#### Scenario: Config in the system-tier location
- **WHEN** the config sits in `%ProgramData%\iroh-tunnel\config` with the DACL created by an elevated process
- **THEN** the trust check passes

#### Scenario: Config in a user's profile
- **WHEN** an elevated install names `--config` in a user's `%LOCALAPPDATA%` folder owned by that user
- **THEN** the trust check fails naming the path and the user's account and SID, and nothing is registered

#### Scenario: Binary writable by a user
- **WHEN** the registered binary or one of its parent directories grants a non-admin user write access
- **THEN** the trust check for the binary fails

## MODIFIED Requirements

### Requirement: Trust check timing
The system SHALL run the config and state check at system install and at every start of the system service by the Service Control Manager, before the config is parsed; a foreground elevated `run` SHALL NOT run it. It SHALL run the binary check at system install and in the CLI `service start` and `service restart` verbs run from an elevated terminal, against the binary recorded in the registration. A start by the Service Control Manager at boot or through recovery actions SHALL NOT be described as binary-checked.

#### Scenario: Unsafe config at service start
- **WHEN** the config becomes writable by Everyone after install and the service starts
- **THEN** the service log contains a `permission error` naming the config path, and the service stops without serving

#### Scenario: Unsafe binary at CLI start
- **WHEN** an untrusted SID gains write access to the registered binary and `service start` runs from an elevated terminal
- **THEN** the command exits with code 3 naming the binary path and the service is not started

#### Scenario: Unsafe binary at install
- **WHEN** `service install` runs from an elevated terminal with a binary in a user-writable directory
- **THEN** the command exits with code 3 naming the binary path and nothing is registered

### Requirement: Never repair access on checked files
The system SHALL NOT change the owner or DACL of a checked config, state directory or any of their ancestors. A refusal SHALL report the path, the owner as account name and SID, and the offending SID and rights, as a permission error (exit code 3) for CLI operations or as a logged permission error for the service.

#### Scenario: Fixed condition
- **WHEN** the unsafe access is removed by an administrator and the service is restarted
- **THEN** the service runs again and the owner and DACL are exactly as the administrator set them

## RENAMED Requirements

- FROM: `### Requirement: Never repair access on user files`
- TO: `### Requirement: Never repair access on checked files`

## REMOVED Requirements

### Requirement: Trusted principals for a Windows system service

**Reason**: The pinned user SID is no longer trusted; replaced by "Only admin principals are trusted for a Windows system service".

**Migration**: Keep system-tier configs in `%ProgramData%\\iroh-tunnel\\config` created from an elevated terminal.
