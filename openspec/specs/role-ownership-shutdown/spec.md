# role-ownership-shutdown Specification

## Purpose

TBD

## Requirements

### Requirement: Single running process per role
The system SHALL refuse to start a `serve` or `access` role while another process of the same role is running for the same user, regardless of executable name, `--config` path or state directory override, and in either start order, exiting with the conflict status (exit code 6) and an "already running" message.
Evidence: src/ownership.rs:320-345, tests/role_ownership.rs:206-238 (second_serve_is_refused_across_binaries_overrides_and_start_order)

#### Scenario: All-in-one first, standalone second
- **WHEN** `iroh-tunnel serve` is running and `iroh-tunnel-serve` is started with a different state directory override
- **THEN** the second process exits with code 6 and stderr contains "already running"

#### Scenario: Standalone first, all-in-one second
- **WHEN** `iroh-tunnel-serve` is running and `iroh-tunnel serve` is started with a different state directory override
- **THEN** the second process exits with code 6 and stderr contains "already running"

### Requirement: Ownership scope is the user's default state directory
The system SHALL scope the per-user role lock to the user's default state directory (`<default state dir>/<role>.lock`), independent of `--config` and `IROH_TUNNEL_STATE_DIR`.
Evidence: src/ownership.rs:1-30, src/ownership.rs:88-100, tests/role_ownership.rs:206-238

#### Scenario: Different state overrides share one lock
- **WHEN** two serve processes of one user are started with different `IROH_TUNNEL_STATE_DIR` values
- **THEN** only the first runs and the second is refused as already running

### Requirement: Concurrent starts admit exactly one owner
The system SHALL admit exactly one owner when several processes of the same role start at the same time; every other contender exits with the conflict status.
Evidence: src/ownership.rs:200-215, tests/role_ownership.rs:276-304 (concurrent_starts_admit_exactly_one_owner)

#### Scenario: Four racing serve starts
- **WHEN** four serve processes (mixed standalone and all-in-one) start simultaneously with the same key
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
The system SHALL refuse to start a role whose public node id equals the public node id of the other role running at the user or machine tier, with a conflict naming "the same public identity" and suggesting `config keygen`.
Evidence: src/ownership.rs:436-470, tests/role_ownership.rs:264-272

#### Scenario: Access started with the serve key
- **WHEN** serve is running with key K and access is started with the same key K
- **THEN** access exits with code 6 and stderr contains "same public identity"

### Requirement: Ownership is not stopped automatically
The system SHALL only refuse a conflicting start and SHALL NOT stop another process; the refusal message names the role, scope, owner (pid and node id when known) and how to stop it manually (Ctrl+C or `service stop`).
Evidence: src/ownership.rs:30-34, src/ownership.rs:263-305
Confidence: inferred

#### Scenario: Refusal message content
- **WHEN** a start is refused because the role runs
- **THEN** the message reads "<role> is already running (<scope>; <owner>); stop it first (...); nothing is stopped automatically"

### Requirement: Machine-tier lock between system service and user processes
The system SHALL prevent a system process and a user process of one role from running at the same time, using a machine lock that its creator holds exclusively and other processes hold shared, regardless of start order. The tier SHALL exist on Linux, macOS and Windows.

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

### Requirement: Non-elevated start does not create a machine lock
The system SHALL NOT create the machine lock file or directory from a non-elevated process, and SHALL proceed without machine-tier ownership when the lock file does not exist.
Evidence: src/ownership.rs:380-420
Confidence: inferred

#### Scenario: No machine lock file
- **WHEN** a non-elevated user starts a role and no machine lock file exists
- **THEN** the role starts holding only the user-tier lock

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
The system SHALL publish a `<role>.owner.json` next to the lock containing role, pid, public node id (once known) and start time, SHALL NOT write any secret key to it, and SHALL NOT use it as proof of liveness.
Evidence: src/ownership.rs:22-34, src/ownership.rs:237-260, src/ownership.rs:60-70, src/ownership.rs:572-580 (owner_record_round_trips_without_secret_material)

#### Scenario: Owner record round trip
- **WHEN** an owner record is written with node id "abc"
- **THEN** reading it yields the current pid and node id "abc", and the file contains no secret material

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
The system SHALL refuse `service start` with the conflict status and an "already running" message while the role is running.
Evidence: src/ownership.rs:548-560, src/app.rs:342, tests/role_ownership.rs:478-491 (service_start_is_refused_while_the_role_runs)

#### Scenario: Lock held by another process
- **WHEN** the user-tier `serve.lock` is held and `serve service start` is run
- **THEN** exit code is 6 and stderr contains "already running"

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
