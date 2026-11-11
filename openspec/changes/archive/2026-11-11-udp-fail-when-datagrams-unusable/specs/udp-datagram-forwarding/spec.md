## REMOVED Requirements

### Requirement: Unusable datagrams are dropped, not failed at setup

**Reason**: It recorded a deviation from the integration contract (issue #152): a connection that never supported datagrams dropped every datagram as `limit_shrink` with no setup error. The deviation is resolved; the behavior is now specified by "Connections that cannot carry datagrams fail at setup" (setup failure) and "Datagram limit lost mid-session is dropped per datagram" (mid-session loss, including the scenario "Limit reported as absent").

**Migration**: Operators who relied on the `limit_shrink` counter to spot a peer without datagram support read the setup error in the logs of either role instead. `limit_shrink` keeps counting mid-session drops. No config or status schema change.

## MODIFIED Requirements

### Requirement: No flow survives reconnect
The system SHALL end all flows when the peer is lost or reconnects, with quarantined sockets remaining only as sinks, SHALL evict a dead connection on the first send error, and SHALL dial afresh on the next datagram so forwarding resumes with a new incarnation, except while Access waits after a datagram setup failure (see "Access waits before redialing after a datagram setup failure").
Evidence: docs/udp-integration-contract.md:32, docs/udp-integration-contract.md:34, src/udp.rs:476-488, tests/udp_tunnel.rs:807

#### Scenario: Serve restarts
- **WHEN** Serve restarts while a client keeps sending
- **THEN** Access dials a new connection and the client receives replies again

## ADDED Requirements

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
