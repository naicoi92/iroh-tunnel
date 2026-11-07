## ADDED Requirements

### Requirement: Windows installer protects the install directory it creates
The Windows installer SHALL leave an install directory that it creates, whether the default or a `-BinDir` path, writable only by SYSTEM, the Administrators group and TrustedInstaller. Its owner SHALL be the Administrators group, and no entry SHALL grant the installing user's own SID write access. The installer SHALL NOT change the owner or access of a directory that already existed.

#### Scenario: Fresh default install
- **WHEN** `install.ps1` creates `$env:ProgramFiles\iroh-tunnel` from an elevated session
- **THEN** the directory and the installed executables pass the Windows system-service binary trust check

#### Scenario: Existing directory untouched
- **WHEN** `install.ps1` installs into a directory that already exists
- **THEN** its owner and access entries are unchanged
