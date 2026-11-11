## Design Summary

**Intent.** Close a silent-failure gap in experimental UDP forwarding (#152). When a connection cannot carry QUIC datagrams at all, Access and Serve today drop every datagram, count it as `limit_shrink` and log a warning, so the operator gets no clear error. The goal is a clear failure at setup. It covers peers that will rarely be seen today: a different implementation on the same ALPN, or a future config change that disables datagrams. Between two current iroh-tunnel binaries both sides enable datagrams, and QUIC guarantees an MTU of at least 1200 bytes.

**Size.** A small change, confined to the UDP code paths of the engine plus spec, contract doc and tests.

**What changes for the operator.**
- On a UDP service, when a newly established connection cannot carry datagrams, that one connection is closed with a dedicated application close code and reason, and both sides log a clear error that names the service and the peer.
- The role process and its other services keep running. One misbehaving peer never takes Serve down.
- Access does not redial in a tight loop: after such a failure it waits a fixed interval before dialing that service again. Datagrams from local clients that arrive while it waits are dropped.
- `limit_shrink` means again only "the limit became unusable mid-session"; a connection that never supports datagrams no longer feeds it. A payload over a smaller but still usable limit stays `oversize`, as today.

**Goals**
- Detect "datagrams unusable" once, right after the handshake of a UDP service connection, on both sides. Each side only sees the peer's transport parameters, so both must check.
- Make the failure visible in logs on both sides, including on the side that receives the close.
- Cover it end to end with a real-binary test whose peer has datagrams disabled.

**Non-goals**
- No new status file field and no host API change. A status signal can be added later if a frontend needs one.
- No wire-format or protocol change (no probe message).
- No change to TCP services or to the dialer shared by TCP and UDP.
- No detection of datagrams blocked somewhere on the network path while the transport parameters claim support.

**Anti-goals**
- A peer that cannot carry datagrams must never stop the Serve process or other services.
- Access must not hammer the peer with redials.
- `limit_shrink` must not lose its mid-session meaning.

**Existing requirements this change alters**
- `udp-datagram-forwarding` / *Unusable datagrams are dropped, not failed at setup*: replaced. The current text records the deviation ("does NOT currently fail the session at setup"); the new requirement specifies failing the connection at setup, and keeps per-datagram drops counted as `limit_shrink` only for a limit that becomes unusable mid-session.
- `udp-datagram-forwarding` / *Payload size limit without fragmentation*, scenario *Unusable maximum*: unchanged in meaning (absent or ≤ 4 maxima are unusable); it now also drives the setup check.
- `workspace-boundaries` / *Drop counter docs expose the unusable-datagram deviation*: the `limit_shrink` rustdoc no longer describes the deviation, because it is resolved; the requirement is modified or removed accordingly.
- `docs/udp-integration-contract.md` failure table row "Datagrams unusable on the connection" is rewritten to the implemented behavior.
- `connection-path-status` / *UDP diagnostics block*: unchanged; no schema change.

## Alternatives Considered

### Alternative A: Close only that connection, log on both sides (chosen behavior)
- **Approach**: Close the affected connection with a dedicated close code; log an error on both sides; Access backs off before redialing; process keeps running.
- **Pros**: Small; matches how other connection errors are surfaced today (logs); no schema change.
- **Cons**: Not visible in the status file or a frontend, only in logs.
- **Why chosen**: See Agreed Approach.

### Alternative B: As A, plus a status file field
- **Approach**: Additionally report "datagrams unusable" per UDP service in the status files.
- **Pros**: Readable by `status` and future frontends.
- **Cons**: Changes the status schema, host API and goldens (`connection-path-status`); larger scope.
- **Why not chosen**: No current consumer needs it; it can follow later without undoing A.

### Alternative C: Exit the role process with an error exit code
- **Approach**: Treat unusable datagrams as fatal for the role.
- **Pros**: Impossible to miss.
- **Cons**: One foreign peer could stop Serve for all peers and services; Access dials lazily, so the failure happens at runtime, not at start.
- **Why not chosen**: Violates the anti-goal of isolating one peer's failure.

Where to place the check was also weighed:

- **Pure classification in the engine `udp` module, called at the two UDP-only entry points** (Serve when it starts handling a UDP connection, Access when its UDP loop adopts a new connection), with Access's back-off state kept in its UDP loop. *Chosen*: confined to UDP, unit-testable, no TCP or wire impact.
- **Inside the shared Access dialer, gated by protocol**: keeps "waiting" state next to dialing, but makes the TCP/UDP-shared dialer aware of UDP and risks TCP behavior. Not chosen.
- **Application-level probe after connect**: would also catch on-path datagram blocking, but adds a protocol message and changes the wire format. Not chosen.

## Agreed Approach

Alternative A for behavior, with the check as a pure classification in the engine `udp` module invoked at the two UDP-only entry points. It closes the gap the integration contract already promised, keeps the failure isolated to one connection, leaves TCP, the shared dialer, the wire format and the status schema untouched, and restores `limit_shrink` to its mid-session meaning.

## Key Decisions

- Purpose: fail clearly at setup (user decision).
- Behavior: Alternative A (user decision).
- Check placement: pure classifier in the engine `udp` module, called from the UDP-only Serve and Access paths; Access back-off lives in its UDP loop (user decision).
- "Setup" is right after the QUIC handshake of a UDP service connection, when the peer's transport parameters are known (my call: earlier there is no data, the first datagram is too late).
- Access back-off before redialing a service whose connection failed this way is a fixed 30 seconds, not configurable (my call; proposed to the user without objection).
- Client datagrams arriving during the back-off are dropped with a rate-limited log and are not counted in any drop counter, so no status schema change is needed and `limit_shrink` keeps its meaning (my call; proposed without objection).
- A dedicated application close code with the reason `udp datagrams unsupported`; the side receiving that close recognises it, logs the cause and, on Access, enters the back-off (my call; proposed without objection).
- The end-to-end test disables datagrams on one side through a `test-utils`-only seam; production builds cannot disable datagrams (my call).

## Open Questions

- None blocking. The concrete close code value, log wording and test seam shape are left to design.
