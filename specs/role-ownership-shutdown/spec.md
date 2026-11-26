# role-ownership-shutdown Specification

## Purpose

Define how a single running process owns each role (Serve or Access) through lock and owner records, including the machine-tier lock for system services. Stop requests begin a graceful drain, and a repeated signal after the grace period or the stuck-shutdown watchdog forces exit. Status file contents are owned by `connection-path-status`.

## Requirements

### Requirement: Single running process per role
Except for explicitly isolated GitHub Actions invocations, the system SHALL refuse to start a `serve` or `access` role while another process of the same role is running in the same tier (for the user tier, for the same user), regardless of executable name, `--config` path or state directory override, and in either start order, exiting with the conflict status (exit code 6) and an "already running" message.
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
Outside explicitly isolated GitHub Actions invocations, the system SHALL scope the user-tier role lock to the user's default state directory (`<default state dir>/<role>.lock`) and the system-tier ownership lock to the machine lock directory (`<machine dir>/<role>.claim.lock`, with the detection lock `<machine dir>/<role>.lock` beside it), both independent of `--config` and `IROH_TUNNEL_STATE_DIR`.
Evidence: src/ownership.rs:1-30, src/ownership.rs:88-100, tests/role_ownership.rs:206-238

#### Scenario: Different state overrides share one lock
- **WHEN** two serve processes of one user are started with different `IROH_TUNNEL_STATE_DIR` values
- **THEN** only the first runs and the second is refused as already running

#### Scenario: System lock ignores the state override
- **WHEN** two system-tier serve processes are started with different `IROH_TUNNEL_STATE_DIR` values
- **THEN** only the first runs and the second is refused as already running

### Requirement: GitHub Actions ownership scope
An explicitly isolated GitHub Actions invocation SHALL claim its user-tier role within its own ownership scope on every supported runner platform. This SHALL NOT change ordinary CLI ownership, system-tier ownership, or the default behavior of `IROH_TUNNEL_STATE_DIR`.
Evidence: issue #268

#### Scenario: Separate action scopes coexist
- **WHEN** two action invocations of the same role start with distinct action scopes and otherwise valid configurations
- **THEN** neither invocation is refused because the other holds ownership

#### Scenario: Ordinary ownership is unchanged
- **WHEN** two ordinary CLI invocations of the same role start as the same user with different `IROH_TUNNEL_STATE_DIR` values
- **THEN** the second invocation is still refused with exit code 6

### Requirement: Concurrent starts admit exactly one owner
The system SHALL admit exactly one owner when several processes of the same role start at the same time in the same tier (for the user tier, for the same user); every other contender in that tier exits with the conflict status.
Evidence: src/ownership.rs:200-215, tests/role_ownership.rs:276-304 (concurrent_starts_admit_exactly_one_owner)

#### Scenario: Four racing serve starts
- **WHEN** four serve processes (mixed standalone and all-in-one) of one user start simultaneously with the same key
- **THEN** three exit with code 6 and exactly one keeps running

### Requirement: Refused start causes no side effects
The system SHALL leave the running owner's status snapshot and any configuration file unchanged when a start is refused, and SHALL NOT generate or persist a secret key during a refused run.
Evidence: src/ownership.rs:320-335, tests/role_ownership.rs:307-333 (refused_config_run_neither_persists_a_key_nor_touches_the_owner)

#### Scenario: Refused configured run with empty secret_key
- **WHEN** a configured `serve run --config` with an empty `secret_key` is refused because the role is running
- **THEN** the config file content is unchanged and the owner's `serve-status.json` is byte-identical

### Requirement: Different roles coexist with distinct identities
The system SHALL allow a `serve` and an `access` process to run at the same time provided they use different public node identities.
Evidence: src/ownership.rs:436-470, tests/role_ownership.rs:241-262 (roles_with_distinct_keys_coexist_but_a_shared_key_is_rejected)

#### Scenario: Serve and access with distinct keys
- **WHEN** serve is running and access is started with a different key
- **THEN** access starts and reaches the running state

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

### Requirement: Ownership is not stopped automatically
The system SHALL only refuse a conflicting start and SHALL NOT stop another process; the refusal message names the role, scope, owner (pid and node id when known) and how to stop it manually (Ctrl+C or `service stop`).
Evidence: src/ownership.rs:30-34, src/ownership.rs:263-305
Confidence: inferred

#### Scenario: Refusal message content
- **WHEN** a start is refused because the role runs
- **THEN** the message reads "<role> is already running (<scope>; <owner>); stop it first (...); nothing is stopped automatically"

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

### Requirement: Crash leftovers never block or masquerade as a live owner
The system SHALL release role locks when the owning process dies (including `kill -9`), so a leftover lock file or status snapshot SHALL NOT block a restart, and `status` SHALL report the role as "not running" with a "stale" indication.
Evidence: src/ownership.rs:1-10, tests/role_ownership.rs:361-386 (kill_nine_leftover_neither_blocks_restart_nor_reads_as_live)

#### Scenario: Restart after SIGKILL
- **WHEN** a running serve is killed with SIGKILL and `status` is run
- **THEN** output contains "serve is not running" and "stale", and a new serve starts successfully

### Requirement: Status does not trust a pid alone
The system SHALL decide whether a role is running from its lock, not from a recorded pid, so a status snapshot whose pid belongs to an unrelated live process is reported as "not running" and "stale".
Evidence: src/ownership.rs:481-510, tests/role_ownership.rs:389-409 (status_does_not_trust_a_reused_pid)

#### Scenario: Reused pid
- **WHEN** a status snapshot records the pid of a live unrelated process and no role holds the lock
- **THEN** `status` reports "serve is not running" and "stale"

### Requirement: Status reports the live owner across state overrides
The system SHALL report a running role and its owner pid from `status` even when queried with a different state directory override than the one the role was started with.
Evidence: src/ownership.rs:481-510, tests/role_ownership.rs:412-432 (running_role_status_names_role_and_owner_pid)

#### Scenario: Status from another state directory
- **WHEN** serve runs with state directory A and `status` is run with state directory B
- **THEN** output says running, contains the owner pid, and does not contain "not running"

### Requirement: Owner record is informational and public only
The system SHALL publish a `<role>.owner.json` next to the lock of the process's own tier containing role, pid, public node id (once known) and start time, SHALL NOT write any secret key to it, and SHALL NOT use it as proof of liveness, except for the upgrade transition: an admin-owned record naming a live pid, while `<role>.lock` is held exclusively, identifies an earlier-version system instance (see "Machine-tier lock between system service and user processes"). A user-tier process SHALL NOT write a record in the machine lock directory. Publishing SHALL be a precondition of running: when a record cannot be written at claim time or when the node id is published, the start SHALL be refused with an error naming the record path. On Unix a record in the machine lock directory SHALL be readable by other accounts regardless of the process umask; on Windows it inherits the machine directory's access rules.
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

### Requirement: Stale owner record replaced on claim
The system SHALL overwrite any owner record left by a crashed predecessor at the moment a new claim succeeds, before the new identity is published.
Evidence: src/ownership.rs:346-353
Confidence: inferred

#### Scenario: Leftover record
- **WHEN** a role starts after a crashed predecessor left an owner record
- **THEN** the record names the new process pid

### Requirement: Graceful stop removes owner record but keeps lock file
The system SHALL remove its own owner record on graceful stop only while the record still names its pid, and SHALL leave the lock file in place.
Evidence: src/ownership.rs:422-437, tests/role_ownership.rs:336-358 (graceful_stop_removes_the_owner_record_but_keeps_the_lock_file)

#### Scenario: SIGINT stop
- **WHEN** a running serve receives SIGINT and exits
- **THEN** `serve.owner.json` no longer exists and `serve.lock` still exists

### Requirement: Lock files are not followed through symlinks
The system SHALL open lock files without following a symlink at the lock path on Unix.
Evidence: src/ownership.rs:134-148
Confidence: inferred

#### Scenario: Symlink at lock path
- **WHEN** the lock path is a symlink
- **THEN** opening the lock fails instead of following the link

### Requirement: Busy lock retried briefly before refusal
The system SHALL retry a busy exclusive claim up to four attempts at 25 ms intervals before treating the lock as a real owner, so a transient status probe does not cause a false refusal while a genuine owner is refused in roughly 75 ms.
Evidence: src/ownership.rs:195-215
Confidence: inferred

#### Scenario: Real owner
- **WHEN** another process holds the lock
- **THEN** the start is refused after about 75 ms

### Requirement: Service start refused while role runs
The system SHALL refuse `service start` with the conflict status and an "already running" message while the role is running in the same tier. A role running only in the other tier SHALL NOT refuse it.
Evidence: src/ownership.rs:548-560, src/app.rs:342, tests/role_ownership.rs:478-491 (service_start_is_refused_while_the_role_runs)

#### Scenario: Lock held by another process
- **WHEN** the user-tier `serve.lock` is held and `serve service start` is run in the user tier
- **THEN** exit code is 6 and stderr contains "already running"

#### Scenario: Other tier running
- **WHEN** the system-tier serve is running and a user runs `serve service start`
- **THEN** the user service is started and stderr contains the other-tier notice

### Requirement: Stop signals begin a graceful drain
The system SHALL treat SIGINT (Ctrl+C), SIGTERM (SIGINT only on non-Unix) and, for a Windows system service, the Service Control Manager's stop, shutdown and pre-shutdown requests as stop requests. It SHALL log which one was received, and begin shutdown by stopping admission before draining active channels.

#### Scenario: SIGTERM
- **WHEN** a role receives SIGTERM
- **THEN** it logs "received SIGTERM" and starts the drain

#### Scenario: Service stop request
- **WHEN** a Windows system service receives a stop request from the Service Control Manager
- **THEN** it logs the stop request and starts the same drain

### Requirement: Single drain deadline of three seconds
The system SHALL give all services and connections one shared drain budget of 3 seconds measured from the first stop, with later stop requests not extending it.
Evidence: src/shutdown.rs:8-14, src/shutdown.rs:22-44, src/shutdown.rs:268-277 (begin_publishes_the_same_deadline_to_forced_exit)

#### Scenario: Repeated begin
- **WHEN** a stop is begun twice
- **THEN** the deadline set by the first call is kept

### Requirement: Remaining channels aborted at drain deadline
The system SHALL let owned channel tasks finish until the drain deadline and SHALL abort and join any still running afterwards; a task failure other than cancellation is logged as an error.
Evidence: src/shutdown.rs:46-85
Confidence: inferred

#### Scenario: Held tunnel
- **WHEN** a channel is still open when the deadline passes
- **THEN** it is aborted and shutdown continues

### Requirement: Parent loss cancels admission
The system SHALL treat loss of the stop sender as a stop so no owned task outlives its parent.
Evidence: src/shutdown.rs:44-58
Confidence: inferred

#### Scenario: Sender dropped
- **WHEN** the stop sender is dropped
- **THEN** waiting tasks observe stop immediately

### Requirement: Repeated stop signal forces exit after the grace
The system SHALL ignore a stop signal received within the drain grace and, for a stop signal received after the grace has elapsed, abandon shutdown with exit code 130.
Evidence: src/shutdown.rs:96-110, src/shutdown.rs:170-200, src/shutdown.rs:225-235 (signal_inside_grace_is_ignored_and_one_after_it_forces_exit, exit_codes_are_130_for_signal_and_1_for_watchdog)

#### Scenario: Signal at 50 ms and 400 ms with deadline at 300 ms
- **WHEN** signals arrive at 50 ms and 400 ms
- **THEN** the first is ignored and the second yields a forced exit with code 130

### Requirement: Stuck shutdown watchdog
The system SHALL abandon a shutdown that has not finished 10 seconds after the drain deadline, exiting with code 1; the forced-exit logic SHALL stay inactive until a drain has begun.
Evidence: src/shutdown.rs:86-110, src/shutdown.rs:176-185, src/shutdown.rs:238-247 (watchdog_fires_after_grace_plus_extra_without_signals)

#### Scenario: No signals after drain
- **WHEN** a drain began and nothing finishes or signals before deadline plus the watchdog period
- **THEN** the forced exit is the watchdog with code 1

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
