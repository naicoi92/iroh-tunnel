## ADDED Requirements

### Requirement: Windows service logs refused status reports
Once the service log is open, the system SHALL log every status report the Service Control Manager refuses at warning level or above, naming the reported state and, for a pending report, its checkpoint. These lines SHALL follow the service log's existing rules: a quiet run keeps only errors, and an untrusted machine directory gets no log.

#### Scenario: Refusal is logged
- **WHEN** the Service Control Manager refuses a stop-pending report after the service log is open
- **THEN** the service log names the stop-pending state and its checkpoint

### Requirement: Windows service retries running and stopped reports
The system SHALL retry a refused running or stopped report a bounded number of times. When the running report is still refused, it SHALL request the graceful stop and end the service with service-specific code 101, and MUST NOT leave it in start-pending. When the stopped report is still refused, the process SHALL exit with code 101 without sending it again. A refused pending report SHALL NOT be retried on its own; the next pending report carries a fresh checkpoint.

#### Scenario: Running report refused once
- **WHEN** the Service Control Manager refuses the first running report of a starting service
- **THEN** the report is retried, the service is reported running, and the refusal is in the service log

#### Scenario: Running report always refused
- **WHEN** the Service Control Manager refuses every attempt of the running report
- **THEN** the role drains, the service is reported stopped with service-specific code 101, and no start-pending report follows

#### Scenario: Stopped report refused once
- **WHEN** the Service Control Manager refuses the first stopped report of a finished run
- **THEN** the stopped report is retried and accepted with the run's exit mapping

#### Scenario: Stopped report always refused
- **WHEN** the Service Control Manager refuses every attempt of the stopped report
- **THEN** the refusals are logged, the stopped report is not sent again, and the process exits with code 101

#### Scenario: Pending report refused
- **WHEN** the Service Control Manager refuses a start-pending report
- **THEN** that report is not sent again and the next tick reports a higher checkpoint

#### Scenario: Stop request while the running report is retried
- **WHEN** a stop request arrives while the running report is being retried
- **THEN** the stop is handled after the retry settles: a clean stop when the running report was accepted, or the stop already in progress with code 101 when it was not

### Requirement: Windows service internal faults end the service with fixed codes
The system SHALL end the Windows system service with a service-specific failure when its own bookkeeping fails: code 100 when the internal service state is poisoned by a panic, which MUST NOT be continued on, and code 102 when the pending-checkpoint ticker cannot start, which MUST NOT abort the process. Codes 100, 101 and 102 MUST NOT be used for any other exit.

#### Scenario: Poisoned service state
- **WHEN** the internal service state is poisoned by a panic while the service runs
- **THEN** the service is reported stopped with service-specific code 100 and the process exits

#### Scenario: Status ticker cannot start
- **WHEN** the pending-checkpoint ticker thread cannot be created at service start
- **THEN** the role does not run and the service is reported stopped with service-specific code 102

### Requirement: Windows service fault codes take precedence over the exit mapping
A recorded fault code (100, 101 or 102) SHALL replace the run's own exit mapping from "Windows service recovery and exit mapping", including a clean exit after the drain. A fault SHALL be recorded only before the role runs or after it reported running, so it never meets an already-running exit (code 6). Fault codes SHALL trigger the shared recovery policy like any other service-specific failure.

#### Scenario: Fault overrides a clean drain
- **WHEN** a fault is recorded and the drained run then exits with code 0
- **THEN** the service is reported stopped with the fault's code, not as a clean stop

#### Scenario: Fault triggers recovery
- **WHEN** the service ends with code 101 and the restart policy is enabled
- **THEN** the Service Control Manager restarts it after the policy delay
