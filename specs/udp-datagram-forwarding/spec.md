# udp-datagram-forwarding Specification

## Purpose

Define UDP forwarding between Access and Serve: datagram framing and size limit, Flows with per-client assignment and monotonic admission, caps, idle expiry, Quarantine of expired upstream sockets, and the rule that no Flow survives reconnect. TCP forwarding is owned by `tcp-tunnel-forwarding`.

## Requirements

### Requirement: Datagram framing
The system SHALL carry each UDP client payload through the tunnel as one QUIC DATAGRAM of the form `[flow id: u32 big-endian][payload]`, with the payload passed through unchanged, an empty payload being valid in both directions and delivered as a zero-length UDP datagram, and datagram boundaries preserved.
Evidence: src/udp.rs:12, src/udp.rs:272-290, src/udp.rs:516-531, docs/udp-integration-contract.md:7-8, docs/udp-integration-contract.md:14, tests/udp_tunnel.rs:369, tests/udp_tunnel.rs:399

#### Scenario: Payload round trip
- **WHEN** a client sends a datagram to the Access listener and the target echoes it
- **THEN** the client receives a reply whose bytes equal the original payload

#### Scenario: Empty and binary payloads
- **WHEN** a client sends an empty datagram, a 256-byte binary datagram and a 900-byte datagram back-to-back
- **THEN** the client receives each as its own datagram with unchanged content

### Requirement: Malformed datagram rejection
The system SHALL drop a received tunnel datagram shorter than the 4-byte flow header and count it as `malformed`.
Evidence: src/udp.rs:278-286, src/udp.rs:532-538, docs/udp-integration-contract.md:22

#### Scenario: Short datagram
- **WHEN** a tunnel datagram of 0 to 3 bytes is received
- **THEN** it is dropped and the `malformed` counter increases

### Requirement: Payload size limit without fragmentation
The system SHALL recompute the UDP payload limit as maximum QUIC datagram size minus 4 on every send, drop oversize payloads whole in either direction, and count them as `oversize`; absent or at-most-4 maxima are unusable.
Evidence: src/udp.rs:6-8, src/udp.rs:305-311, src/udp.rs:540-551, docs/udp-integration-contract.md:7, docs/udp-integration-contract.md:13, docs/udp-integration-contract.md:20-21, tests/udp_tunnel.rs:580

#### Scenario: Oversize client datagram
- **WHEN** a client sends a 4000-byte datagram to Access
- **THEN** Access drops it, counts `oversize`, and the target never receives it

#### Scenario: Unusable maximum
- **WHEN** the connection's maximum datagram size is 4 or less, or absent
- **THEN** there is no usable payload limit and datagrams are not carried

### Requirement: Per-client flow assignment on Access
The system SHALL give each distinct client endpoint of an Access UDP service its own flow, created on its first datagram, kept while it is active, with identifiers assigned in strictly increasing order and never reused within a session incarnation.
Evidence: src/udp.rs:313-360, src/udp.rs:553-563, src/udp.rs:578-592, docs/udp-integration-contract.md:9, tests/udp_tunnel.rs:399

#### Scenario: Returning client after expiry
- **WHEN** a client's flow expires and the client sends again
- **THEN** it receives a new flow with a higher identifier than the expired one

#### Scenario: Active client keeps flow
- **WHEN** a client with a live flow sends again
- **THEN** it keeps its existing flow identifier

### Requirement: Flow identifier exhaustion
The system SHALL refuse to issue a flow identifier once all 32-bit identifiers of the incarnation are spent rather than wrap, SHALL evict the connection so the next datagram dials afresh with an empty flow table, and SHALL count it as `cap`.
Evidence: src/udp.rs:324-334, src/udp.rs:616-628, docs/udp-integration-contract.md:33

#### Scenario: Identifiers run out
- **WHEN** the next identifier would exceed u32::MAX
- **THEN** no flow is issued, the incarnation ends and `cap` increases

### Requirement: Serve upstream socket per flow
The system SHALL give each admitted flow its own connected upstream UDP socket to the service's fixed target, so that the target sees a distinct, stable source endpoint per flow, and SHALL key the flow table by (peer, flow id) so equal identifiers from different Access peers are never cross-delivered.
Evidence: docs/udp-integration-contract.md:10-12, src/udp.rs:400-410, tests/udp_tunnel.rs:444, tests/udp_tunnel.rs:502

#### Scenario: Distinct stable upstream endpoints
- **WHEN** two clients each send several datagrams
- **THEN** the target sees one constant source endpoint per client and the two endpoints differ

#### Scenario: Two Access peers
- **WHEN** clients behind two Access peers sharing one Serve each use flow id 0
- **THEN** each client receives only its own replies

### Requirement: Monotonic flow admission on Serve
The system SHALL admit a new flow on Serve only when its identifier is higher than every identifier already seen from that peer in the incarnation, and SHALL drop a datagram whose identifier is at or below that mark with no live flow (expired, quarantined or refused), counting it as `unknown_flow` and never rebinding it.
Evidence: src/udp.rs:400-456, src/udp.rs:631-656, src/udp.rs:728-736, src/udp.rs:764-778, docs/udp-integration-contract.md:11-12, docs/udp-integration-contract.md:23

#### Scenario: Lower identifier
- **WHEN** a datagram arrives with an identifier lower than the highest seen and no live flow
- **THEN** it is dropped as `unknown_flow`

#### Scenario: Expired identifier with drifted timers
- **WHEN** Serve has expired a flow but Access still sends its identifier
- **THEN** Serve does not rebind it and drops the datagram

#### Scenario: Refused identifier
- **WHEN** a flow was refused and later datagrams carry the same identifier
- **THEN** they are dropped as `unknown_flow`, and a higher identifier is still admittable

### Requirement: Service and per-peer flow caps on Serve
The system SHALL cap flows per Serve service (default 256) and per peer (default 64), with the service cap shared across all peers; at a cap Serve SHALL refuse the new flow, drop its datagram, count it as `cap`, and never evict an existing flow. Slots SHALL be released when flows close.
Evidence: src/udp.rs:19-25, src/udp.rs:113-141, src/udp.rs:738-746, docs/udp-integration-contract.md:27, docs/udp-integration-contract.md:44, tests/udp_tunnel.rs:628, tests/udp_tunnel.rs:667

#### Scenario: Service cap reached
- **WHEN** `max_flows = 2` and a third client sends
- **THEN** the third client gets no reply, `cap` increases, and the two earlier flows keep working

#### Scenario: Per-peer cap
- **WHEN** one Access peer has used its `max_flows_per_peer` share and sends from another client
- **THEN** that flow is refused while another peer is still served

### Requirement: Access flow cap
The system SHALL cap live flows per Access UDP service (default 256, from `max_flows`), refusing a new client over the cap by dropping its datagram at Access before it reaches Serve and counting `cap`, while known clients and running flows remain unaffected.
Evidence: src/udp.rs:19-21, src/udp.rs:336-352, src/udp.rs:594-614, docs/udp-integration-contract.md:36, docs/udp-integration-contract.md:48, tests/udp_tunnel.rs:864

#### Scenario: Over Access cap
- **WHEN** `max_flows = 2` on Access and a third client sends
- **THEN** it gets no reply, the target sees no new datagram, `cap` increases, and the two running flows still round-trip

### Requirement: Refused flows consume Access state silently
The system SHALL give no refusal signal on the wire when Serve refuses a flow; Access SHALL keep the flow until idle expiry and later datagrams of that identifier are dropped as `unknown_flow`.
Evidence: docs/udp-integration-contract.md:9, docs/udp-integration-contract.md:28

#### Scenario: Serve refuses a flow
- **WHEN** Serve refuses a flow at its cap
- **THEN** Access still counts the flow as live until it idles out and no error reaches the client

### Requirement: Idle expiry refreshed by traffic in either direction
The system SHALL end a flow that is silent in both directions for the service's idle timeout (default 60 s, `idle_timeout`), treating a datagram in either direction as traffic that refreshes the timer, and SHALL sweep at most every 5 s and never less often than the timeout (minimum 100 ms).
Evidence: src/udp.rs:14-17, src/udp.rs:249-258, src/udp.rs:565-576, src/udp.rs:658-673, src/udp.rs:781-790, docs/udp-integration-contract.md:31, docs/udp-integration-contract.md:49

#### Scenario: Reply refreshes timer
- **WHEN** a reply passes 50 s after the last send
- **THEN** the flow does not expire at 100 s and expires at 110 s

#### Scenario: Short timeout
- **WHEN** the idle timeout is 1 s
- **THEN** the sweep runs about every 1 s

### Requirement: Quarantine of expired upstream sockets
The system SHALL quarantine expired Serve sockets as reply sinks for 30 seconds, count late replies without delivery and report them as quarantined, holding the service-cap slot but not the peer-cap slot until quarantine ends.
Evidence: src/udp.rs:56-58, src/udp.rs:142-153, src/udp.rs:244-247, src/udp.rs:748-762, docs/udp-integration-contract.md:31, docs/udp-integration-contract.md:38-40, tests/udp_tunnel.rs:722, tests/udp_tunnel.rs:769

#### Scenario: Late reply
- **WHEN** the target replies after the flow expired
- **THEN** `quarantine_reply` increases, `quarantined_flows` is 1, `active_flows` is 0, and the client receives nothing

#### Scenario: Slot held then freed
- **WHEN** `max_flows = 1` and the only flow is quarantined
- **THEN** a new flow is refused until quarantine ends, then admitted

### Requirement: No flow survives reconnect
The system SHALL end all flows when the peer is lost or reconnects, with quarantined sockets remaining only as sinks, SHALL evict a dead connection on the first send error, and SHALL dial afresh on the next datagram so forwarding resumes with a new incarnation, except while Access waits after a datagram setup failure (see "Access waits before redialing after a datagram setup failure").
Evidence: docs/udp-integration-contract.md:32, docs/udp-integration-contract.md:34, src/udp.rs:476-488, tests/udp_tunnel.rs:807

#### Scenario: Serve restarts
- **WHEN** Serve restarts while a client keeps sending
- **THEN** Access dials a new connection and the client receives replies again

### Requirement: Per-flow bounded queue on Serve
The system SHALL hold datagrams for a Serve flow in a bounded queue (default 64, `queue_depth`, minimum 1), deliver them in arrival order, drop the newest datagram when the queue is full counting `queue_full`, never wait for a slow flow, and not let one stuck flow delay another.
Evidence: src/udp.rs:26-28, src/udp.rs:187-243, src/udp.rs:794-860, docs/udp-integration-contract.md:29, tests/udp_tunnel.rs:831

#### Scenario: Queue full
- **WHEN** a queue of depth 2 holds two datagrams and a third is offered
- **THEN** the third is dropped and the first two are delivered in order

#### Scenario: Flooding flow
- **WHEN** one client floods its flow
- **THEN** another client's flow still gets replies

#### Scenario: Writer gone
- **WHEN** a flow's writer has ended
- **THEN** pushing reports the flow closed

### Requirement: Upstream errors keep the flow
The system SHALL, on an upstream send error on Serve, drop that datagram and count `upstream_error` while keeping the flow, and SHALL treat connection-refused, reset, aborted, interrupted and would-block receive errors on a UDP socket as transient rather than ending the flow.
Evidence: src/udp.rs:36-41, src/udp.rs:288-303, src/udp.rs:675-691, docs/udp-integration-contract.md:24, tests/udp_tunnel.rs:539

#### Scenario: Target down at first
- **WHEN** the target is not listening and clients keep sending, then the target comes up
- **THEN** the same client gets a reply on its flow

#### Scenario: Permission error
- **WHEN** a receive error is of kind PermissionDenied or InvalidInput
- **THEN** it is not transient

### Requirement: Bind or descriptor exhaustion refuses the flow
The system SHALL, when Serve cannot open an upstream socket, refuse the flow, count `cap`, name the cause in the log, and keep the process running.
Evidence: docs/udp-integration-contract.md:35, src/udp.rs:46-50

#### Scenario: Socket cannot be opened
- **WHEN** binding the upstream socket fails
- **THEN** the flow is refused and the process continues

### Requirement: Foreign-source replies are not observable
The system SHALL ignore replies from a source other than the target because upstream sockets are connected and the operating system drops them; the `foreign_source` counter exists but is never incremented.
Evidence: src/udp.rs:51-54, docs/udp-integration-contract.md:30

#### Scenario: Reply from another source
- **WHEN** a datagram from a non-target source reaches the upstream socket
- **THEN** it is not delivered and `foreign_source` stays 0

### Requirement: UDP status reporting
The system SHALL report for each UDP service in the status files the active flow count (excluding quarantined), the quarantined flow count, the current datagram limit (absent when unusable), and one non-negative drop counter for each of oversize, malformed, unknown_flow, upstream_error, cap, queue_full, limit_shrink, quarantine_reply and foreign_source, with counters never underflowing.
Evidence: src/udp.rs:55-186, src/udp.rs:693-726, tests/udp_tunnel.rs:580, tests/udp_tunnel.rs:628

#### Scenario: Counters by reason
- **WHEN** two `oversize` and one `cap` drops are recorded
- **THEN** the snapshot shows oversize 2, cap 1, malformed 0

#### Scenario: Excess close
- **WHEN** more flows are closed than are open
- **THEN** active flows is 0 and does not underflow

### Requirement: Path independence
The system SHALL forward UDP round trips over both a relay-only path and a direct path, and the status file SHALL report which transport is used.
Evidence: tests/udp_tunnel.rs:932, docs/udp-integration-contract.md:3

#### Scenario: Relay-only then direct
- **WHEN** endpoints are forced relay-only, then default endpoints are used
- **THEN** both round trips succeed and Access status reports only relay transports in the first case and a direct active transport in the second

### Requirement: UDP service limit configuration bounds
The system SHALL accept bounded UDP-only flow settings, restrict peer caps and queues to Serve, reject peer caps above explicit service caps and keep quarantine fixed rather than configurable.
Evidence: docs/udp-integration-contract.md:44, src/config.rs:110-250, tests/udp_tunnel.rs:628,722

#### Scenario: Configured bounds
- **WHEN** UDP limits are configured
- **THEN** idle_timeout is at most 86400 seconds, flow caps at most 1,000,000 and queue_depth at most 65,536; max_flows_per_peer and queue_depth are Serve-only

#### Scenario: Per-peer cap above service cap
- **WHEN** a UDP service sets `max_flows_per_peer` above an explicit `max_flows`
- **THEN** the configuration is rejected at load

#### Scenario: UDP field on TCP service
- **WHEN** a TCP service sets any of these fields
- **THEN** the configuration is rejected

### Requirement: Connections that cannot carry datagrams fail at setup
The system SHALL, right after the handshake of a UDP service connection, classify it on each side as unable to carry datagrams when its maximum QUIC datagram size is absent or at most 4 bytes. Such a side MUST close only that connection with the dedicated application close code and reason `udp datagrams unsupported`, log an error naming the service and peer, and forward no datagram on it; the process and its other connections and services keep running.
Source: issue #152, design.md D2-D5

#### Scenario: Usability boundary
- **WHEN** the maximum QUIC datagram size of a new UDP service connection is absent, 4 bytes, or 5 bytes
- **THEN** it is classified as unable, unable, and able to carry datagrams respectively

#### Scenario: Serve peer does not allow datagrams
- **WHEN** Access establishes a connection for a UDP service to a Serve peer that does not allow datagrams
- **THEN** Access closes that connection with the dedicated close code, logs an error naming the service and the peer, and the target receives no datagram

#### Scenario: Access peer does not allow datagrams
- **WHEN** Serve accepts a connection for a UDP service from an Access peer that does not allow datagrams
- **THEN** Serve closes that connection with the dedicated close code and logs an error naming the service and the peer

#### Scenario: Failure is isolated to the connection
- **WHEN** Serve closes one peer's UDP connection at setup because that peer does not allow datagrams
- **THEN** the Serve process keeps running, and its other services and other peers' connections keep forwarding

#### Scenario: Setup failure does not count as limit shrink
- **WHEN** a connection fails the setup check
- **THEN** `limit_shrink` of that service does not increase because of it

### Requirement: The closed side reports a datagram setup failure
The system SHALL, when a UDP service connection is closed by its peer with the dedicated datagram setup failure close code, log an error on the receiving side, Access or Serve, naming the service and the peer and stating that the peer reported datagrams as unsupported.
Source: issue #152, design.md D3-D4

#### Scenario: Access receives the close
- **WHEN** Serve closes an Access UDP connection at setup because the Access peer does not allow datagrams
- **THEN** Access logs an error naming the service and the Serve peer that states datagrams are unsupported, and starts the redial wait

#### Scenario: Serve receives the close
- **WHEN** Access closes a UDP connection at setup because the Serve peer does not allow datagrams
- **THEN** Serve logs an error naming the service and the Access peer that states datagrams are unsupported

### Requirement: Datagram limit lost mid-session is dropped per datagram
The system SHALL, when the datagram limit of a connection that passed the setup check becomes unusable during the session incarnation, drop each affected datagram individually, count it as `limit_shrink` and log a warning, without closing the connection for this reason. A payload larger than a smaller but still usable datagram limit remains an oversized datagram counted as `oversize`.
Source: issue #152, docs/udp-integration-contract.md, design.md D6

#### Scenario: Limit lost mid-session
- **WHEN** the datagram limit observed for a datagram on a connection that passed the setup check is unusable
- **THEN** that datagram is dropped, `limit_shrink` increases, a warning is logged, and the connection stays open

#### Scenario: Smaller usable limit counts as oversize
- **WHEN** the datagram limit shrinks mid-session but stays usable and a payload exceeds it
- **THEN** that datagram is dropped and counted as `oversize`, not `limit_shrink`

#### Scenario: Limit reported as absent
- **WHEN** the datagram limit is set to none
- **THEN** the status snapshot reports no datagram limit

### Requirement: Access waits before redialing after a datagram setup failure
The system SHALL, after an Access UDP service connection fails the datagram setup check locally or is closed by Serve with the dedicated datagram setup failure close code, not dial that service again for a fixed 30 seconds. Client datagrams arriving during that wait MUST be dropped without dialing and MUST NOT be counted in any drop counter. The wait MUST NOT affect other services of the same Access.
Source: issue #152, design.md D4

#### Scenario: No immediate redial
- **WHEN** a client keeps sending to an Access UDP service whose connection just failed the datagram setup check
- **THEN** Access does not dial that service again within 30 seconds and drops those datagrams

#### Scenario: One warning per wait
- **WHEN** several client datagrams are dropped during one wait
- **THEN** exactly one warning is logged for that wait, stating when dialing resumes, and later drops in the same wait log below warning level

#### Scenario: Drops during the wait are not counted
- **WHEN** client datagrams are dropped during the wait
- **THEN** no UDP drop counter of that service increases because of them

#### Scenario: Dialing resumes after the wait
- **WHEN** 30 seconds have passed since the setup failure and a client sends a datagram
- **THEN** Access dials the service afresh

#### Scenario: Other services are unaffected
- **WHEN** one Access UDP service is waiting after a datagram setup failure
- **THEN** other services of the same Access, TCP or UDP, keep dialing and forwarding normally
