## ADDED Requirements

### Requirement: Windows installer protects the install directory it creates
The Windows installer SHALL, from an elevated session, leave an install directory that it creates (the default or a `-BinDir` path) writable only by SYSTEM, the Administrators group and TrustedInstaller, owned by the Administrators group, with no entry granting the installing user's own SID write access. It SHALL NOT change the owner or access of a directory that already existed.

#### Scenario: Fresh default install
- **WHEN** `install.ps1` creates `$env:ProgramFiles\iroh-tunnel` from an elevated session
- **THEN** the directory and the installed executables pass the Windows system-service binary trust check

#### Scenario: Parent grants write to Users
- **WHEN** `install.ps1` creates `-BinDir` from an elevated session under a parent that grants Users modify access
- **THEN** the created directory grants Users only read and execute, and no untrusted principal holds a write right

#### Scenario: Existing directory untouched
- **WHEN** `install.ps1` installs into a directory that already exists
- **THEN** its owner and access entries are unchanged

### Requirement: Non-elevated installs keep default directory access
The Windows installer SHALL, from a non-elevated session, create a new install directory with default access, as before this change. A binary installed there SHALL remain refused by the system-service binary trust check.

#### Scenario: Non-elevated install into a new directory
- **WHEN** `install.ps1 -BinDir <new dir under the user's profile>` runs from a non-elevated session
- **THEN** the install succeeds and the directory's owner is not changed to Administrators
