# Tunnel

Language for exposing a service on one host to clients on another host.

## Language

**Serve**:
The role that makes a service on its host available through a tunnel.

**Access**:
The role that provides a local endpoint for clients to reach a service through a tunnel.

## UDP forwarding

**Flow**:
One client's UDP conversation with the target, identified by the Access listener and the client's address and port. Each flow is bound to its own upstream endpoint on Serve.
_Avoid_: Connection, session, mapping

**Session incarnation**:
One lifetime of the connection between an Access and a Serve. No flow survives into a later incarnation.
_Avoid_: Connection generation, epoch

**Quarantine**:
The period after a flow expires during which its upstream endpoint is not reused.
_Avoid_: Cooldown, grace period

**Datagram limit**:
The largest UDP payload the tunnel can currently carry intact between an Access and a Serve. It can change during a session incarnation.
_Avoid_: MTU, max packet size

**Oversized datagram**:
A client UDP datagram larger than the current datagram limit. It is dropped and counted, never delivered in part.
_Avoid_: Large packet, truncated datagram
