# tcp-tunnel-forwarding Specification

## Purpose

Define TCP forwarding between Access and Serve: service ALPN addressing, one stream per local TCP client over a shared connection, byte-transparent copy with half-close, dial retry, deadlines and failure handling. UDP forwarding is owned by `udp-datagram-forwarding`.

## Requirements

### Requirement: Service ALPN addressing
The system SHALL address each service on the QUIC connection by the ALPN `iroh-tunnel/{name}` and SHALL perform no protocol negotiation, so the ALPN is the same regardless of multiplexing.
Evidence: src/proto.rs:17-35, src/serve.rs:115-126, src/access.rs:542-565, src/proto.rs (unit tests alpn_for_builds_prefixed_string, name_from_alpn_rejects_other_protocols)

#### Scenario: Access dials the service ALPN
- **WHEN** Access dials a service named `postgres`
- **THEN** the connection is requested with ALPN bytes `iroh-tunnel/postgres`

#### Scenario: Foreign ALPN is not a service name
- **WHEN** an ALPN does not start with `iroh-tunnel/` or is not valid UTF-8
- **THEN** no service name is derived from it

### Requirement: Serve routes connections to services by ALPN
The system SHALL register one ALPN per configured service on the Serve endpoint and SHALL route each accepted connection to the local target of the service matching its negotiated ALPN.
Evidence: src/serve.rs:115-126, src/serve.rs:143-160, src/serve.rs:605-618, tests/serve_access_tunnel.rs:59-205

#### Scenario: Matching service
- **WHEN** a peer connects with the ALPN of a configured service
- **THEN** Serve logs "peer connected via ..." and handles its streams against that service's `host:port` target

### Requirement: Unknown ALPN connections are dropped
The system SHALL drop an accepted connection whose ALPN matches no configured service and log a warning, without affecting other connections.
Evidence: src/serve.rs:612-618
Confidence: inferred

#### Scenario: Unknown service ALPN
- **WHEN** a connection arrives with an ALPN that is not in the Serve service table
- **THEN** Serve logs "connection with unknown ALPN for service '<name>', dropping" and continues accepting

### Requirement: One stream per local TCP client
The system SHALL carry each local TCP client accepted by Access as one bidirectional QUIC stream on the service's connection, and Serve SHALL connect each accepted stream to the service's local TCP target.
Evidence: src/access.rs:673-715, src/access.rs:949-975, src/serve.rs:704-780, tests/serve_access_tunnel.rs:59-205, tests/configured_multiplex.rs:211-336

#### Scenario: Round trip
- **WHEN** a TCP client connects to the Access listen address and sends bytes
- **THEN** the bytes arrive at the Serve target, and the target's reply bytes return to the client

### Requirement: Byte-transparent bidirectional copy
The system SHALL copy bytes verbatim in both directions between the local TCP socket and the tunnel stream, with no framing or transformation, and SHALL handle payloads larger than a single buffer intact.
Evidence: src/pipe.rs:1-12, src/pipe.rs:44-62, src/pipe.rs (unit tests tcp_copies_bytes_in_both_directions, tcp_larger_payload_roundtrips_intact), tests/serve_access_tunnel.rs:59-205

#### Scenario: Large payload
- **WHEN** 50,000 bytes are sent in each direction through the pipe
- **THEN** each side receives exactly the bytes sent, in order

### Requirement: TCP half-close propagation
The system SHALL propagate end-of-stream in one direction by shutting down the matching write side while continuing to carry data in the opposite direction, and SHALL finish only after both directions complete.
Evidence: src/pipe.rs:14-22, src/pipe.rs:76-121, src/pipe.rs (unit test tcp_half_close_propagates_eof)

#### Scenario: Client half-closes
- **WHEN** the local client shuts down its write side without sending data
- **THEN** the far side reads EOF, and the far side can still send data back to the client until it closes its own write side

#### Scenario: Copy error still closes the write side
- **WHEN** a copy in one direction fails with an error
- **THEN** the write side of that direction is still shut down and the first error is returned

### Requirement: Multiplexed channels share one connection per service
The system SHALL reuse one QUIC connection per Access service for all concurrent and later local clients, dialing at most one connection even when multiple clients arrive concurrently, and SHALL ignore any legacy `multiplex` setting.
Evidence: src/access.rs:518-531, src/access.rs:580-600, src/proto.rs:1-12, tests/serve_access_tunnel.rs:352-421, tests/configured_multiplex.rs:211-336, tests/config_multiplex_compat.rs:8-60

#### Scenario: Two concurrent clients
- **WHEN** two local clients connect to the same Access service at the same time
- **THEN** two streams are carried on one connection and closing one does not affect the other

#### Scenario: Legacy multiplex false
- **WHEN** the configuration sets `multiplex = false` (or true, or omits it)
- **THEN** sequential channels still reuse the same connection

### Requirement: Connection survives idle periods
The system SHALL keep the multiplexed connection usable after an idle period longer than the QUIC default idle timeout, so a later channel rides the same connection.
Evidence: tests/serve_access_tunnel.rs:422-465

#### Scenario: 60 seconds idle
- **WHEN** no traffic flows for 60 seconds and a new client then connects
- **THEN** the new channel uses the same connection and both old and new channels still echo data

### Requirement: Dial retry with exponential backoff
The system SHALL retry failed dials with backoff of 1s, 2s, 4s, 8s, 16s then a 30s cap, bound each attempt to 5s, 10s, 15s then 20s, and stop at the caller's deadline, logging "reconnected after N attempts" on success after failures.
Evidence: src/role_run.rs:85-170, src/role_run.rs (unit tests backoff_schedule_doubles_then_caps_at_max, attempt_timeout_grows_then_caps, connect_with_retry_stops_at_the_deadline), tests/configured_multiplex.rs:473-540

#### Scenario: Unreachable peer
- **WHEN** the peer is unreachable
- **THEN** Access logs "connect attempt N timed out" for successive attempts, with consecutive timed-out attempts at least about 5.5s apart

#### Scenario: Recovery
- **WHEN** a dial succeeds after one or more failures
- **THEN** Access logs "reconnected after N attempts"

### Requirement: Client connect deadline
The system SHALL give each new local client a 30-second budget covering lock wait, dial, backoff and opening the stream, and on expiry SHALL close that client and log "client connection deadline expired" while Access keeps running. An established stream is not subject to this budget.
Evidence: src/access.rs:901-925, src/access.rs:697-707, tests/configured_multiplex.rs:473-540, tests/configured_multiplex.rs:684-700

#### Scenario: Peer never available
- **WHEN** a client connects while the Serve peer is unavailable
- **THEN** the client is closed between 28s and 40s after connecting and Access remains running

#### Scenario: Recovery before deadline
- **WHEN** the peer becomes available before the deadline
- **THEN** the waiting client is connected and exchanges data

#### Scenario: Established client past deadline
- **WHEN** a client already has an established stream and 30 seconds pass
- **THEN** the stream stays usable

### Requirement: Independent client deadlines
The system SHALL track the connect deadline separately for each local client, so staggered clients expire on their own schedules.
Evidence: src/access.rs:697-707, tests/configured_multiplex.rs:684-688

#### Scenario: Staggered clients
- **WHEN** clients connect at different times while the peer is unavailable
- **THEN** each is closed relative to its own connect time

### Requirement: Single redial when opening a stream fails
The system SHALL, when opening a stream on the cached connection fails, evict only that connection, redial once and retry opening; if the retry fails the client's channel is closed with an error.
Evidence: src/access.rs:949-975, src/access.rs:646-653, src/access.rs (unit test invalidate_only_evicts_the_failed_connection)

#### Scenario: Cached connection died
- **WHEN** `open_bi` fails on the cached connection
- **THEN** Access logs "multiplexed open_bi failed ... redialing once", evicts it and opens the stream on a fresh connection

### Requirement: Local target failure resets only that channel
The system SHALL limit the local TCP connect for each stream to 10 seconds, and on refusal, error or timeout SHALL reset only that channel while the shared connection and sibling channels keep working.
Evidence: src/serve.rs:672-701, src/serve.rs:725-745, src/serve.rs (unit test connect_local_timeout_resets_only_that_channel), tests/configured_multiplex.rs:1290-1375

#### Scenario: Refused target
- **WHEN** the local target refuses the connection for one stream
- **THEN** the client of that stream is closed, Serve logs "failed to connect local service, resetting stream", and an established sibling stream continues to carry data in both directions

#### Scenario: Slow target does not block siblings
- **WHEN** one stream's local connect is slow
- **THEN** other streams on the same connection are still accepted

### Requirement: Peer loss closes channels without replay
The system SHALL, when the connection is lost, abort the affected channels and close their local sockets, and SHALL NOT replay bytes already read; a later client uses a fresh connection after the peer returns.
Evidence: src/serve.rs:765-790, src/access.rs:580-600, tests/configured_multiplex.rs:338-471

#### Scenario: Serve killed mid-transfer
- **WHEN** Serve is killed while two channels are active
- **THEN** both clients and both target sockets close, Access keeps running, and after Serve restarts with the same identity a new client works without old bytes reaching the target

### Requirement: Delivery acknowledgement after copy
The system SHALL, after a pipe finishes, wait up to 2 seconds for the peer to acknowledge the final stream bytes, and SHALL NOT treat a missing acknowledgement as success.
Evidence: src/pipe.rs:140-186, src/serve.rs:748-775, src/access.rs:932-946, src/pipe.rs (unit tests await_delivery_*)

#### Scenario: No acknowledgement
- **WHEN** no acknowledgement arrives within 2 seconds
- **THEN** the outcome is unconfirmed; Access logs "peer lost: no delivery acknowledgement within grace" and reports a tunnel error, Serve logs a warning, and nothing is retransmitted

#### Scenario: Peer stops the stream
- **WHEN** the peer sends STOP_SENDING with code 42 after the pipe completes
- **THEN** Access logs "tunnel error: peer stopped client stream: 42" and the client sees FIN

### Requirement: Last response delivered before shutdown exit
The system SHALL deliver the final response of the last active channel before a role exits on a shutdown signal, without waiting the full drain grace.
Evidence: src/serve.rs:748-775, tests/configured_multiplex.rs:1000-1062

#### Scenario: Shutdown right after last response
- **WHEN** a role is stopped just after the last channel's response is written
- **THEN** the client receives the whole response and the role exits successfully in under 3 seconds

### Requirement: Access listener failure handling
The system SHALL log an error and stop serving a service when its listen address cannot be bound, and SHALL log accept errors, retrying after 1 second, without terminating the listener.
Evidence: src/access.rs:673-715
Confidence: inferred

#### Scenario: Bind failure
- **WHEN** the listen address cannot be bound
- **THEN** Access logs "failed to bind <addr>" and that service's listener ends

### Requirement: Connect and disconnect logging
The system SHALL log "peer connected via <paths>" on Serve and an established-connection line on Access when a connection is made, and a disconnect line when the connection closes.
Evidence: src/serve.rs:634-655, src/access.rs:600-630, src/role_run.rs:68-82, tests/configured_multiplex.rs:338-471

#### Scenario: One connection, many streams
- **WHEN** several clients use one service
- **THEN** Serve logs "peer connected via" once for that connection
