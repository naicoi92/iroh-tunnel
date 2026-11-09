## MODIFIED Requirements

### Requirement: Step returns only when the tunnel is usable
Each action SHALL complete its main step only when the tunnel is usable, within 60 seconds. Serve is usable when its status file reports its NodeId and its home relay. Access over TCP is usable when its listener is bound and its connection to the Serve peer is established, which the action triggers by opening and closing one local connection to `listen` (to the loopback address of the same family when `listen` is `0.0.0.0` or `[::]`).
Evidence: design.md D7; issue #183
Confidence: inferred

#### Scenario: Access ready
- **WHEN** an Access TCP step completes successfully
- **THEN** a connection opened to `listen` by the next step reaches the remote service without waiting for a dial

#### Scenario: Serve ready
- **WHEN** a Serve step completes successfully
- **THEN** its `node-id` output is set and its node is registered with its home relay, so a client using the same relays can dial it through that relay without a retry window for relay registration

### Requirement: Start failures are reported with the tunnel log
When the tunnel is not usable within 60 seconds, exits before it is usable, or logs that its listener failed to bind, the main step SHALL fail with the reason, the process exit code and error class when it exited, the last tunnel status, and the end of the tunnel log. When a Serve step times out after its status file reported `node_id` but no home relay, the reason SHALL state that no home relay was registered and name the `relay-url` and `relay-token` inputs.
Evidence: design.md D7; docs/monitoring.md (exit codes); issue #183
Confidence: inferred

#### Scenario: Readiness timeout
- **WHEN** an Access step cannot reach the Serve peer within 60 seconds
- **THEN** the step fails with a message saying it was not ready in time, the last tunnel status and the end of the tunnel log

#### Scenario: Serve times out without a home relay
- **WHEN** a Serve step's status file reports `node_id` but `home_relay` stays `null` for 60 seconds, for example because `relay-url` points at an unreachable relay
- **THEN** the step fails with a message saying it was not ready in time and no home relay was registered, naming `relay-url` and `relay-token`, with the last tunnel status and the end of the tunnel log, and `node-id` is not set

#### Scenario: Tunnel exits during start
- **WHEN** the tunnel process exits before it is usable because its config is rejected
- **THEN** the step fails at once with the exit code, its error class and the end of the tunnel log

#### Scenario: Listen port taken
- **WHEN** the Access listener cannot bind because `listen` is already in use (the process keeps running and logs `failed to bind`)
- **THEN** the step fails at once naming `listen` and quoting the bind error, without waiting for the 60-second limit, and the post step stops the process

## ADDED Requirements

### Requirement: Serve readiness waits for the home relay
The Serve action SHALL treat its tunnel as usable only when the status file reports both `node_id` and a non-empty `home_relay`. The docs SHALL say that Serve waits for its home relay, and SHALL NOT tell callers to retry early dials for relay registration.
Evidence: issue #183; #187 (`home_relay` reported within one status flush, about 5 s); `action-src/build.ts` (minimum version 0.6.0)
Confidence: inferred

#### Scenario: Home relay not yet reported
- **WHEN** the Serve status file has `node_id` and `home_relay: null`
- **THEN** the step keeps waiting and does not set `node-id`

#### Scenario: Home relay reported
- **WHEN** the Serve status file has `node_id` and `home_relay: https://relay.example.com./`
- **THEN** the step completes and sets `node-id`

#### Scenario: Docs describe Serve readiness
- **WHEN** a caller reads the Behavior section of the docs page
- **THEN** it says Serve is ready once its status file reports a NodeId and a home relay, and does not advise retrying dials for relay registration

## REMOVED Requirements

### Requirement: Serve readiness does not wait for a relay

**Reason**: The rule existed only because the status file did not report the home relay (#177). Since #187 it does, so Serve readiness now waits for it; its scenario "home_relay null" (step completes) is replaced by "Home relay not yet reported" (step keeps waiting).

**Migration**: Replaced by "Serve readiness waits for the home relay". Callers no longer need a dial-retry window for relay registration; a Serve step whose relay never registers now fails after 60 s instead of completing.
