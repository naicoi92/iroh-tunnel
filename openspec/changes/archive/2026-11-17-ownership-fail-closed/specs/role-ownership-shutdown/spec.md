## MODIFIED Requirements

### Requirement: Same public identity rejected across roles
The system SHALL refuse to start a role whose public node id equals the public node id of the other role running at the user or machine tier, with a conflict naming "the same public identity" and suggesting `config keygen`. The system SHALL fail closed when the identity of a running other role cannot be determined: if the other role's lock is held exclusively at a tier and its owner record at that tier exists but still cannot be read or parsed after a brief retry (four attempts at 25 ms intervals), the start SHALL be refused with the conflict status (exit code 6) and a message naming the record path. An owner record that is absent, or present without a node id, SHALL NOT refuse the start; within one tier this is safe because every start publishes its own node id before it inspects the other role. An owner record SHALL be consulted only while the other role's lock at that tier is held exclusively; a record at a tier whose lock is free, or at the machine tier held only shared by user processes, SHALL NOT be consulted. Known limitation: a user-session process of one role started before a system service of the other role with the same key is not detected by the system start, because per-user records of other accounts are not inspected; the reverse start order is refused.
Evidence: crates/host/src/ownership.rs:785-850, tests/role_ownership.rs:241-273 (roles_with_distinct_keys_coexist_but_a_shared_key_is_rejected), tests/role_ownership.rs:276-303 (access_is_refused_when_the_running_serve_record_is_corrupt)

#### Scenario: Access started with the serve key
- **WHEN** serve is running with key K and access is started with the same key K
- **THEN** access exits with code 6 and stderr contains "same public identity"

#### Scenario: Running other role with a corrupt owner record
- **WHEN** serve is running and its `serve.owner.json` is replaced with content that is not a valid owner record, and access is started with any key
- **THEN** access exits with code 6 and stderr names the path of `serve.owner.json` and says the identity cannot be determined

#### Scenario: Running other role with an unreadable owner record
- **WHEN** the other role's lock is held exclusively and its owner record path exists but cannot be read as a file (for example it is a directory)
- **THEN** publishing the identity fails with a conflict naming the record path

#### Scenario: Running other role that has not published yet
- **WHEN** the other role's lock is held exclusively and its owner record is absent or has no node id
- **THEN** publishing the identity succeeds

#### Scenario: Corrupt record of a role that is not running
- **WHEN** the other role's lock is not held and a corrupt owner record for it is left in the lock directory
- **THEN** publishing the identity succeeds

#### Scenario: Corrupt machine record while the machine lock is held only shared
- **WHEN** the other role's machine lock is held only shared and a corrupt owner record for it is left in the machine directory
- **THEN** publishing the identity succeeds

### Requirement: Owner record is informational and public only
The system SHALL publish a `<role>.owner.json` next to the lock containing role, pid, public node id (once known) and start time, SHALL NOT write any secret key to it, and SHALL NOT use it as proof of liveness. The record SHALL be published in the user tier and, only when the process holds the machine lock exclusively, in the machine tier; a shared machine-tier holder SHALL NOT write a machine record. Publishing SHALL be a precondition of running: when a record cannot be written at claim time or when the node id is published, the start SHALL be refused with an error naming the record path. On Unix a record in the machine tier SHALL be readable by other accounts regardless of the process umask; on Windows it inherits the machine directory's access rules.
Evidence: crates/host/src/ownership.rs:466-527, crates/host/src/ownership.rs:609-651, crates/host/src/ownership.rs:763-771, crates/host/src/ownership.rs:1127 (owner_record_round_trips_without_secret_material)

#### Scenario: Owner record round trip
- **WHEN** an owner record is written with node id "abc"
- **THEN** reading it yields the current pid and node id "abc", and the file contains no secret material

#### Scenario: Owner record cannot be written
- **WHEN** a role claims or publishes its identity and its owner record cannot be written in a tier it holds exclusively
- **THEN** the start fails with an error naming the owner record path

#### Scenario: Machine record readable under a restrictive umask
- **WHEN** an owner record is written in the machine tier on Unix by a process whose umask is 077
- **THEN** the record's mode grants read to other users

### Requirement: Machine-tier lock between system service and user processes
The system SHALL prevent a system process and a user process of one role from running at the same time, using a machine lock that its creator holds exclusively and other processes hold shared, regardless of start order. The tier SHALL exist on Linux, macOS and Windows. A failure to resolve the machine lock directory on a platform that has a machine tier (on Windows, the `%ProgramData%` known folder) SHALL be an error with context in claim, probe, status and install, never a fallback to the user tier alone; only a platform without a machine tier SHALL have none.
Evidence: crates/host/src/ownership.rs:144-244, crates/host/src/ownership.rs:906-932, crates/host/src/service/install.rs:512, tests/role_ownership.rs:505 (system_tier_lock_refuses_user_start_and_blocks_a_system_start)

#### Scenario: Machine lock location
- **WHEN** a machine-tier lock is resolved
- **THEN** its path is `/run/iroh-tunnel/<role>.lock` on Linux, `/var/run/iroh-tunnel/<role>.lock` on macOS and `%ProgramData%\iroh-tunnel\<role>.lock` on Windows

#### Scenario: System lock held, user start
- **WHEN** the machine lock is held exclusively on a read-only file and a user serve is started
- **THEN** it exits with code 6 and stderr contains "system service"

#### Scenario: User running, system start
- **WHEN** a user serve process is running
- **THEN** the machine lock cannot be taken exclusively by another holder because the user process holds it shared

#### Scenario: Windows user task while the system service runs
- **WHEN** the Windows system service of a role is running and the same role is started in the logged-in user's session
- **THEN** the user start exits with code 6 and stderr contains "system service"

#### Scenario: Windows ProgramData cannot be resolved
- **WHEN** the `%ProgramData%` resolver fails
- **THEN** machine directory resolution returns an error mentioning `%ProgramData%`, and claim and probe through that resolution return the error without taking or checking the user-tier lock

#### Scenario: Install lock directories when ProgramData cannot be resolved
- **WHEN** the lock directories used by install and status are resolved and the machine directory resolver fails
- **THEN** resolution returns the error instead of lock directories without a machine tier
