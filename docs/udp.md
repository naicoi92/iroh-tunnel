# UDP forwarding (experimental)

TCP is the supported protocol. UDP forwarding works end to end, but it is
best effort, has known limits, and has not been measured under packet loss,
delay, congestion or path migration. Both peers need 0.5.0 or later.

```sh
iroh-tunnel serve  -n dns -t 127.0.0.1:5353 -p udp
iroh-tunnel access -n dns -l 127.0.0.1:5353 -N <SERVE_NODE_ID> -p udp
```

In a config file, set `protocol = "udp"` on the Serve and the Access service.

- [What to expect](#what-to-expect)
- [Limits](#limits)
- [Tuning](#tuning)
- [Diagnostics](#diagnostics)

## What to expect

- **Best effort.** Datagrams may be lost, reordered or duplicated, as with
  plain UDP. The tunnel adds no reliability, ordering or deduplication.
- **Boundaries and payload are preserved.** Each datagram arrives whole and
  unchanged, or not at all. The tunnel never parses the payload.
- **Per-client flows.** Each client address and port is one flow with its own
  upstream socket on Serve, so replies reach the client that asked. A client
  that changes its source port starts a new flow. A flow ends after 60 seconds
  without traffic in either direction.
- **The target does not see the client.** It sees Serve's upstream socket,
  which NAT may rewrite. ACLs or sessions keyed on the client address will not
  work.
- **Single reply source.** Serve accepts replies only from the configured
  target address and port. Protocols that answer from another address or port
  are not supported.
- **No fragmentation.** A datagram larger than the connection's current limit
  is dropped. The limit depends on the path (about 1.1 KiB on a relay path in
  measurements) and can change during a session.
- **Not for hostile networks.** The caps below bound memory and descriptors,
  but the target must still protect itself; the Access listener does not
  authenticate clients.

## Limits

| Limit | Default |
|---|---|
| Flows per service (Serve and Access) | 256 |
| Flows per peer (Serve) | 64 |
| Queue per flow (Serve) | 64 datagrams; when full, the newest is dropped |
| Idle timeout | 60 seconds |
| Quarantine after a flow ends | 30 seconds |

A new flow over a cap is dropped and counted; running flows are never
evicted. A per-flow queue keeps one slow target from stalling other flows.

After a flow ends, its upstream socket stays open for 30 seconds so that a
late reply is unlikely to reach a new client; quarantined flows count against
the caps. This is not isolation: once the socket is released, the operating
system may give the same local port to a later flow.

These defaults are estimates and have not been tuned against real workloads.

## Tuning

UDP services accept optional limits. They are rejected on TCP services, and
`0` or out-of-range values are rejected when the config loads.

| Key | Roles | Range |
|---|---|---|
| `idle_timeout` | both | 1 to 86400 seconds |
| `max_flows` | both | 1 to 1,000,000 |
| `max_flows_per_peer` | Serve | 1 to 1,000,000 |
| `queue_depth` | Serve | 1 to 65,536 |

`max_flows_per_peer` may not exceed `max_flows`; when only `max_flows` is set
below 64, the per-peer cap follows it.

```toml
[[services]]
name = "dns"
protocol = "udp"
port = 53
max_flows = 128
max_flows_per_peer = 32
```

## Diagnostics

Each UDP service in `serve-status.json` and `access-status.json` carries a
`udp` block: active and quarantined flows, the current datagram limit, and a
drop counter for each reason: `oversize`, `malformed`, `unknown_flow`,
`upstream_error`, `cap`, `queue_full`, `limit_shrink`, `quarantine_reply`,
`foreign_source`. See [Monitoring](monitoring.md).

`foreign_source` stays at 0 today: Serve connects each upstream socket to the
target, so the operating system drops replies from other sources before the
tunnel sees them.

The wire format and admission rules are in the
[UDP integration contract](udp-integration-contract.md).
