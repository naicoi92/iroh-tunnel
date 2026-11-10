# windows-service-trust Specification

## Purpose

Define the trust checks a Windows system service applies to the configuration and state paths it uses: trusted principals, owner and DACL of every path component, refused write and replacement rights, and verified handles only. Service registration itself is owned by `system-service-management`.

## Requirements

### Requirement: Trusted principals for a Windows system service
The system SHALL, for a Windows system service, trust only SYSTEM, the Administrators group, TrustedInstaller and the pinned user SID for the config file, the state directory and every directory above them. It SHALL trust only SYSTEM, the Administrators group and TrustedInstaller for the registered service binary and every directory above it. The pinned user SHALL NOT be trusted for the binary.

#### Scenario: Config in the pinned user's profile
- **WHEN** the config sits in the pinned user's default `%LOCALAPPDATA%` location with the default profile ACL
- **THEN** the trust check passes

#### Scenario: Binary writable by the pinned user
- **WHEN** the registered binary or one of its parent directories grants the pinned user write access
- **THEN** the trust check for the binary fails although the same user is trusted for the config

### Requirement: Path components are held and never followed
The system SHALL open every path component from the volume root down without following reparse points, and SHALL hold each component open without delete sharing while the chain is evaluated. Any component that is a reparse point (symbolic link, junction or mount point) SHALL be refused.

#### Scenario: Junction in the config path
- **WHEN** a directory above the config is replaced by a junction
- **THEN** the check fails with a permission error naming that path and stating it is a reparse point

### Requirement: Owner and DACL of every component
The system SHALL refuse a component whose DACL is missing or unreadable, or whose owner is outside the trusted set. Only allow entries that apply to the object itself SHALL count; inherit-only entries SHALL be ignored for the object they are attached to.

#### Scenario: Untrusted owner
- **WHEN** an ancestor directory is owned by an account outside the trusted set
- **THEN** the check fails naming the directory and its owner

#### Scenario: NULL DACL
- **WHEN** the config has a NULL DACL
- **THEN** the check fails naming the config path and stating that its access list is missing

#### Scenario: Inherit-only entry
- **WHEN** a directory carries an inherit-only CREATOR OWNER entry with full control
- **THEN** that entry does not make the directory itself untrusted

### Requirement: Write rights refused on the final object
The system SHALL refuse the final file or directory when an untrusted SID holds any of: write data, append data, write extended attributes, delete, `WRITE_DAC`, `WRITE_OWNER`, delete-child, generic write or generic all.

#### Scenario: Everyone may modify the config
- **WHEN** the config grants Everyone modify access
- **THEN** the check fails naming the config path, Everyone and the offending rights

### Requirement: Replacement rights refused on ancestors
The system SHALL refuse an ancestor directory when an untrusted SID holds delete, delete-child, `WRITE_DAC`, `WRITE_OWNER` or generic all on it. It SHALL accept create-file and create-subdirectory rights for untrusted SIDs on ancestors, because they cannot replace a trusted child.

#### Scenario: Create-subdirectory on the volume root
- **WHEN** Authenticated Users may create subdirectories on `C:\` and every deeper component is trusted
- **THEN** the check passes

#### Scenario: Delete-child on an ancestor
- **WHEN** an untrusted SID holds delete-child on a directory above the config
- **THEN** the check fails naming that directory

### Requirement: Trust check timing
The system SHALL run the config and state check at system install and at every start of the system service, before the config is parsed. It SHALL run the binary check at system install and in the CLI `service start --system` and `service restart --system` verbs, against the binary recorded in the registration. A start by the Service Control Manager at boot or through recovery actions SHALL NOT be described as binary-checked.

#### Scenario: Unsafe config at service start
- **WHEN** the config becomes writable by Everyone after install and the service starts
- **THEN** the service log contains a `permission error` naming the config path, and the service stops without serving

#### Scenario: Unsafe binary at CLI start
- **WHEN** an untrusted SID gains write access to the registered binary and `service start --system` runs
- **THEN** the command exits with code 3 naming the binary path and the service is not started

#### Scenario: Unsafe binary at install
- **WHEN** `service install --system` runs from a binary in a user-writable directory
- **THEN** the command exits with code 3 naming the binary path and nothing is registered

### Requirement: Verified handles only
The system SHALL parse the config only from the handle that passed the trust check. Under a system service, it SHALL write the status snapshot by re-verifying the state directory on every write, creating the temp file exclusively without following reparse points, and renaming it atomically relative to the verified directory.

#### Scenario: Planted reparse point at the snapshot name
- **WHEN** the user plants the status snapshot name as a symbolic link to a file only SYSTEM can write, and the service writes a snapshot
- **THEN** the link is replaced by a regular file and the target file is unchanged

### Requirement: Never repair access on user files
The system SHALL NOT change the owner or DACL of the user's config, state or any of their ancestors. A refusal SHALL report the path, the owner as account name and SID, and the offending SID and rights, as a permission error (exit code 3) for CLI operations or as a logged permission error for the service.

#### Scenario: Fixed condition
- **WHEN** the unsafe access is removed by the user and the service is restarted
- **THEN** the service runs again and the owner and DACL are exactly as the user set them
