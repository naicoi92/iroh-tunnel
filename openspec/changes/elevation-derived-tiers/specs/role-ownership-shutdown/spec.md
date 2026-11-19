## ADDED Requirements

### Requirement: Other-tier instance notice
The system SHALL print one stderr notice, before binding an endpoint or invoking a service manager, when the same role runs in the other tier, for `run` (configured and quick mode), `status` and `service install|start|restart|status`. The notice SHALL NOT name another account or read the other tier's owner record, and SHALL NOT change the exit code; when the other tier's state cannot be determined, no notice SHALL be printed and the command SHALL continue.

#### Scenario: User run while the system service runs
- **WHEN** the system-tier serve holds its lock and a non-root user runs `iroh-tunnel serve run`
- **THEN** stderr contains a line saying serve is also running as a system service, and the user process starts

#### Scenario: System status while a user instance runs
- **WHEN** a user-tier serve is running and holds `serve.user.lock` shared, and `sudo iroh-tunnel serve status` runs
- **THEN** stderr contains a line saying serve is also running in a user session, and it names no user

#### Scenario: Probe failure
- **WHEN** the other tier's lock file exists but cannot be opened or tested, and `serve run` starts
- **THEN** no notice is printed, the failure is logged at debug level, and the role starts

#### Scenario: Accepted blind spot
- **WHEN** a user-tier serve started while `serve.user.lock` did not exist, and a system-tier serve starts later
- **THEN** the system serve starts and no notice is printed

#### Scenario: Nothing running in the other tier
- **WHEN** no instance of the role runs in the other tier
- **THEN** no notice is printed

#### Scenario: Port clash is explained
- **WHEN** the system-tier access listens on `127.0.0.1:8080` and a user starts access with a service on the same address
- **THEN** stderr contains the other-tier notice before the bind error, and the bind error names `127.0.0.1:8080`

## MODIFIED Requirements

### Requirement: Concurrent starts admit exactly one owner
The system SHALL admit exactly one owner when several processes of the same role start at the same time in the same tier (for the user tier, for the same user); every other contender in that tier exits with the conflict status.
Evidence: src/ownership.rs:200-215, tests/role_ownership.rs:276-304 (concurrent_starts_admit_exactly_one_owner)

#### Scenario: Four racing serve starts
- **WHEN** four serve processes (mixed standalone and all-in-one) of one user start simultaneously with the same key
- **THEN** three exit with code 6 and exactly one keeps running

### Requirement: Single running process per role
The system SHALL refuse to start a `serve` or `access` role while another process of the same role is running in the same tier (for the user tier, for the same user), regardless of executable name, `--config` path or state directory override, and in either start order, exiting with the conflict status (exit code 6) and an "already running" message.
Evidence: src/ownership.rs:320-345, tests/role_ownership.rs:206-238 (second_serve_is_refused_across_binaries_overrides_and_start_order)

#### Scenario: All-in-one first, standalone second
- **WHEN** `iroh-tunnel serve` is running and `iroh-tunnel-serve` is started in the same tier with a different state directory override
- **THEN** the second process exits with code 6 and stderr contains "already running"

#### Scenario: Standalone first, all-in-one second
- **WHEN** `iroh-tunnel-serve` is running and `iroh-tunnel serve` is started in the same tier with a different state directory override
- **THEN** the second process exits with code 6 and stderr contains "already running"

#### Scenario: Second system-tier instance
- **WHEN** the system-tier serve is running and `sudo iroh-tunnel serve run` starts
- **THEN** the second process exits with code 6 and stderr contains "already running"

### Requirement: Ownership scope is the user's default state directory
The system SHALL scope the user-tier role lock to the user's default state directory (`<default state dir>/<role>.lock`) and the system-tier role lock to the machine lock directory (`<machine dir>/<role>.lock`), both independent of `--config` and `IROH_TUNNEL_STATE_DIR`.
Evidence: src/ownership.rs:1-30, src/ownership.rs:88-100, tests/role_ownership.rs:206-238

#### Scenario: Different state overrides share one lock
- **WHEN** two serve processes of one user are started with different `IROH_TUNNEL_STATE_DIR` values
- **THEN** only the first runs and the second is refused as already running

#### Scenario: System lock ignores the state override
- **WHEN** two system-tier serve processes are started with different `IROH_TUNNEL_STATE_DIR` values
- **THEN** only the first runs and the second is refused as already running

### Requirement: Same public identity rejected across roles
The system SHALL refuse to start a role whose public node id equals the public node id of the other role running in the same tier, with a conflict naming "the same public identity" and suggesting `config keygen`. The system SHALL fail closed when the identity of a running other role in that tier cannot be determined: if the other role's lock is held exclusively and its owner record exists but still cannot be read or parsed after a brief retry (four attempts at 25 ms intervals), the start SHALL be refused with the conflict status (exit code 6) and a message naming the record path. An owner record that is absent, or present without a node id, SHALL NOT refuse the start; this is safe because every start publishes its own node id before it inspects the other role. An owner record SHALL be consulted only while the other role's lock is held exclusively. Records of the other tier SHALL NOT be consulted: each tier has its own config and key, so identities are not compared across tiers.
Evidence: crates/host/src/ownership.rs:785-850, tests/role_ownership.rs:241-273 (roles_with_distinct_keys_coexist_but_a_shared_key_is_rejected), tests/role_ownership.rs:276-303 (access_is_refused_when_the_running_serve_record_is_corrupt)

#### Scenario: Access started with the serve key
- **WHEN** serve is running with key K and access is started in the same tier with the same key K
- **THEN** access exits with code 6 and stderr contains "same public identity"

#### Scenario: Running other role with a corrupt owner record
- **WHEN** serve is running and its `serve.owner.json` is replaced with content that is not a valid owner record, and access is started in the same tier with any key
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
- **WHEN** a user-tier role starts while only the presence file `<other role>.user.lock` is held shared and a corrupt `<other role>.owner.json` is left in the machine lock directory
- **THEN** publishing the identity succeeds, because the machine lock directory is not the starting process's tier

#### Scenario: Other tier is not compared
- **WHEN** the system-tier serve runs with key K and a user starts access with key K
- **THEN** access starts

### Requirement: Machine-tier lock between system service and user processes
The system SHALL keep a machine lock directory on Linux, macOS and Windows holding the system tier's claim lock `<role>.claim.lock`, its detection lock `<role>.lock`, its owner record, and a presence file `<role>.user.lock`. The system tier SHALL hold `<role>.claim.lock` exclusively and `<role>.lock` shared while it runs; user-tier processes SHALL hold the presence file shared, through a read-only open, while they run. `<role>.claim.lock` SHALL be accessible only to root (Unix, mode 0600) or SYSTEM and Administrators (Windows), so no non-admin account can block a system start; `<role>.lock` and the presence file SHALL be readable by every account regardless of the creator's umask, so holding them can at most cause a false notice. A system-tier process and user-tier processes of one role SHALL run at the same time; these files SHALL serve only to detect the other tier. Failing to resolve the machine directory (on Windows, the `%ProgramData%` known folder) SHALL fail a system-tier claim, status or install, never falling back to the user tier, and SHALL only suppress the notice in the user tier.
Evidence: crates/host/src/ownership.rs:144-244, crates/host/src/ownership.rs:906-932, crates/host/src/service/install.rs:512, tests/role_ownership.rs:505 (system_tier_lock_refuses_user_start_and_blocks_a_system_start)

#### Scenario: Machine lock location
- **WHEN** a machine-tier lock is resolved
- **THEN** its path is `/run/iroh-tunnel/<role>.lock` on Linux, `/var/run/iroh-tunnel/<role>.lock` on macOS and `%ProgramData%\iroh-tunnel\<role>.lock` on Windows

#### Scenario: System lock held, user start
- **WHEN** the system-tier lock is held exclusively and a user serve is started
- **THEN** the user serve starts and stderr contains the other-tier notice naming the system service

#### Scenario: User running, system start
- **WHEN** a user serve process holds the presence file shared and a system-tier serve starts
- **THEN** the system serve starts and stderr contains the other-tier notice naming a user session

#### Scenario: Several user holders
- **WHEN** two users run serve, holding the presence file shared, and one of them stops
- **THEN** a system-tier probe still reports a user-session instance; after the second stops or is killed with `kill -9`, it reports none

#### Scenario: Readable under a restrictive umask
- **WHEN** a system-tier process with umask 077 creates `<role>.lock` and `<role>.user.lock` on Unix
- **THEN** both files grant read to other accounts

#### Scenario: Non-admin holder cannot block the system service
- **WHEN** a non-admin account holds `<role>.lock` shared and a system-tier serve starts
- **THEN** the system serve starts, and the non-admin account cannot open `<role>.claim.lock`

#### Scenario: Earlier-version system instance still running
- **WHEN** a system-tier process of an earlier version holds `<role>.lock` exclusively, its root/Administrators-owned `<role>.owner.json` in the machine directory names a live pid, and a new system-tier serve starts
- **THEN** the new serve exits with code 6, stderr says a system instance from an earlier version is still running and must be stopped first, and the system-tier probe reports it as running

#### Scenario: Non-admin exclusive holder is not mistaken for an earlier version
- **WHEN** a non-admin account holds `<role>.lock` exclusively and no admin-owned owner record with a live pid exists
- **THEN** a system-tier serve starts

#### Scenario: Second system instance is refused through the claim lock
- **WHEN** a system-tier serve holds `<role>.claim.lock` and another system-tier serve starts
- **THEN** the second exits with code 6 and stderr contains "already running"

#### Scenario: Windows user task while the system service runs
- **WHEN** the Windows system service of a role is running and the same role is started in the logged-in user's session
- **THEN** the user start succeeds and stderr contains the other-tier notice

#### Scenario: Windows ProgramData cannot be resolved
- **WHEN** the `%ProgramData%` resolver fails
- **THEN** machine directory resolution returns an error mentioning `%ProgramData%`, a system-tier claim returns the error, and a user-tier claim proceeds without the notice

#### Scenario: Install lock directories when ProgramData cannot be resolved
- **WHEN** the lock directories used by a system-tier install and status are resolved and the machine directory resolver fails
- **THEN** resolution returns the error instead of lock directories without a machine tier
### Requirement: Non-elevated start does not create a machine lock
The system SHALL NOT create the machine lock directory, any lock file or the presence file from a user-tier process, and SHALL proceed holding only its user-tier lock when the presence file does not exist. A system-tier `run` or `service install` SHALL create the presence file of its role when it is missing, without holding it.
Evidence: src/ownership.rs:380-420
Confidence: inferred

#### Scenario: No machine lock file
- **WHEN** a non-elevated user starts a role and no presence file exists
- **THEN** the role starts holding only the user-tier lock and no file is created in the machine lock directory

#### Scenario: Presence file created by the system tier
- **WHEN** a system-tier `run` or `service install` of a role runs and `<role>.user.lock` does not exist
- **THEN** the file exists afterwards, readable by every account, and is not held by the system process

### Requirement: Owner record is informational and public only
The system SHALL publish a `<role>.owner.json` next to the lock of the process's own tier containing role, pid, public node id (once known) and start time, SHALL NOT write any secret key to it, and SHALL NOT use it as proof of liveness. A user-tier process SHALL NOT write a record in the machine lock directory. Publishing SHALL be a precondition of running: when a record cannot be written at claim time or when the node id is published, the start SHALL be refused with an error naming the record path. On Unix a record in the machine lock directory SHALL be readable by other accounts regardless of the process umask; on Windows it inherits the machine directory's access rules.
Evidence: crates/host/src/ownership.rs:466-527, crates/host/src/ownership.rs:609-651, crates/host/src/ownership.rs:763-771, crates/host/src/ownership.rs:1127 (owner_record_round_trips_without_secret_material)

#### Scenario: Owner record round trip
- **WHEN** an owner record is written with node id "abc"
- **THEN** reading it yields the current pid and node id "abc", and the file contains no secret material

#### Scenario: Owner record cannot be written
- **WHEN** a role claims or publishes its identity and its owner record cannot be written in its tier
- **THEN** the start fails with an error naming the owner record path

#### Scenario: Machine record readable under a restrictive umask
- **WHEN** an owner record is written in the machine lock directory on Unix by a process whose umask is 077
- **THEN** the record's mode grants read to other users

### Requirement: Service start refused while role runs
The system SHALL refuse `service start` with the conflict status and an "already running" message while the role is running in the same tier. A role running only in the other tier SHALL NOT refuse it.
Evidence: src/ownership.rs:548-560, src/app.rs:342, tests/role_ownership.rs:478-491 (service_start_is_refused_while_the_role_runs)

#### Scenario: Lock held by another process
- **WHEN** the user-tier `serve.lock` is held and `serve service start` is run in the user tier
- **THEN** exit code is 6 and stderr contains "already running"

#### Scenario: Other tier running
- **WHEN** the system-tier serve is running and a user runs `serve service start`
- **THEN** the user service is started and stderr contains the other-tier notice
