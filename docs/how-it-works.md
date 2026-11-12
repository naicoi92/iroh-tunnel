# How it works

- [Roles](#roles)
- [Connection path](#connection-path)
- [One connection, many channels](#one-connection-many-channels)
- [Connection lifecycle and timeouts](#connection-lifecycle-and-timeouts)
- [One process per role](#one-process-per-role)
- [Shutdown](#shutdown)
- [Security model](#security-model)
- [Version compatibility](#version-compatibility)
- [When to use something else](#when-to-use-something-else)

## Roles

```
   Access host (your laptop)                  Serve host (behind NAT)
  ┌────────────────────────┐                ┌────────────────────────┐
  │                        │  one Iroh QUIC │                        │
  │ psql ──▶ :55432 ───────┼── connection ──┼───▶ :5432 ── postgres  │
  │         access         │ per service,   │          serve         │
  │                        │ one stream per │                        │
  │                        │ client         │                        │
  └────────────────────────┘                └────────────────────────┘
```

- **Serve** runs next to a service and publishes it into Iroh under the
  node's NodeId and the service name. The service itself can stay bound to
  localhost.
- **Access** dials that NodeId and service name and exposes the service on a
  local port. Local clients connect to it as if the service ran there.

Each service is addressed by the ALPN `iroh-tunnel/<name>`. Serve registers
one ALPN per configured service and drops connections for names it does not
serve.

## Connection path

Iroh connects the two nodes with QUIC over TLS 1.3, using the NodeId as the
TLS identity. Both nodes register with a relay (the n0 public relays unless
you [configure your own](configuration.md#relays)). The relay helps them find
each other and attempts a hole punch; if a direct UDP path works, traffic moves
to it, otherwise it stays on the relay. Relays only ever carry encrypted bytes.

You can see which path a connection uses in the
[status files and logs](monitoring.md): `relay=<url>`, `direct=<addr>`, or
both while iroh keeps the relay as fallback.

## One connection, many channels

Each Access service keeps one long-lived Iroh connection to its Serve peer.
Every local TCP client gets its own QUIC bidirectional stream on that
connection, so the relay session and the QUIC/TLS handshake are paid once,
not per client. This suits database pools, HTTP clients and parallel SSH
sessions.

- Both roles send a QUIC keep-alive every 5 seconds, so an idle connection
  stays up.
- The transport allows 100 concurrent streams per connection by default;
  see [`max_concurrent_streams`](configuration.md#node-settings).
- If the connection dies, its channels see EOF, as a port forward should; the
  next client dials a new connection.
- There is no setting to turn reuse off. The legacy `multiplex` key is
  ignored.

UDP services use QUIC datagrams on the same kind of per-service connection;
see [UDP](udp.md).

## Connection lifecycle and timeouts

These are implementation defaults, not settings, and not delivery
guarantees.

| Stage | Behavior |
|---|---|
| New Access client | 30 seconds to get a working stream, including waiting for the shared connection, retries and backoff |
| Dial attempt | 5, 10, 15, then at most 20 seconds per attempt |
| Failed dial | Exponential backoff 1, 2, 4, 8, 16, then 30 seconds (no jitter) |
| Serve to local target | 10 seconds to connect; a refused or timed-out connect closes only that channel |
| End of a channel | Each side waits at most 2 seconds for the peer to acknowledge the last bytes; on Access a missing acknowledgement is a peer-loss error, not a clean close |
| Established channel | No lifetime limit; peer loss closes the affected channels without replaying data |

TCP half-close is propagated: when one side finishes writing, the other sees
EOF and can still send its reply.

## One process per role

A role (`serve` or `access`) runs as one process per user, whichever
executable, config file or state directory started it. A second start, or
`service start` while the role runs, is refused with exit code `6`; nothing is
stopped for you. On Linux and macOS a machine-level lock also keeps a system
service and a user process of the same role apart. A Serve and an Access may
run together, but not with the same NodeId.

Locks are kernel advisory locks, so a crash or `kill -9` never blocks the next
start. `<role> status` uses these locks, not the recorded pid, to decide
whether the role is running.

## Shutdown

`Ctrl+C` or `SIGTERM` (Ctrl+C only on Windows) stops new channels and gives
active ones a shared 3-second grace to finish. Channels still open after that
are closed. A second stop signal within the grace is ignored; one after it
exits at once with status `130`. If shutdown has not finished 10 seconds after
the grace, a watchdog exits with status `1`. Either way, the role removes its
own status file first.

## Security model

- **Encrypted end to end.** Traffic between the two nodes is QUIC/TLS 1.3.
  Relays see only encrypted bytes and are not trusted.
- **Access pins Serve.** Access dials a specific NodeId, which is a
  self-certifying public key, so it reaches the intended Serve node or
  nothing.
- **NodeId is public identity, not target authorization.** Serve has no peer
  allowlist: any peer that knows the NodeId and the service name can connect
  to the exposed service. Treat the NodeId as an address, keep the secret key
  private, and let the target service authenticate its clients (database
  passwords, SSH keys, HTTP auth).
- **The Access listener is not authenticated.** Anyone who can reach
  `host:port` on the Access host reaches the remote service. Keep the
  default `127.0.0.1` unless you mean to share it.
- **Release integrity.** Release checksums are signed keyless with Sigstore;
  `update` verifies them. See [Verifying a release](install.md#verifying-a-release).

Report vulnerabilities as described in [SECURITY.md](../SECURITY.md).

## Version compatibility

The wire protocol has no negotiation: the ALPN `iroh-tunnel/<name>` has not
changed since 0.1.

| Peers | Status |
|---|---|
| 0.6 ↔ 0.5.x and 0.4.x, either role, TCP | Expected to work. A development build passed one TCP exchange per pairing against the published 0.4.0 and 0.4.2 releases on macOS arm64 (#85), before the iroh 1.4 upgrade; not re-run on the 0.6.0 release |
| 0.6 Access → Serve older than 0.2.0 | Not supported. Old Serve accepts one stream per connection, so a second client hangs. Upgrade Serve |
| UDP | Needs 0.5 or later on both sides |
| Self-hosted iroh-relay 1.x | Supported; see [Self-hosted relay](self-hosted-relay.md) |

When upgrading a pair, upgrade Serve first.

## When to use something else

iroh-tunnel forwards fixed TCP or UDP ports between two machines, with no
server or account of your own. It is not the right tool when you need:

- HTTP edge features (TLS certificates, custom domains, WAF): use cloudflared
  or ngrok;
- a full L3 network between many machines: use Tailscale or another
  WireGuard mesh;
- classic port mapping through a server you already run: frp or bore fit.
