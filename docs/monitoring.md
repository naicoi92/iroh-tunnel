# Monitoring and logs

- [`status` command](#status-command)
- [Status files](#status-files)
- [Log levels](#log-levels)
- [Log events](#log-events)
- [Exit codes](#exit-codes)

## `status` command

`iroh-tunnel <role> status` prints the role's status file as tables;
`--json` prints the file unchanged.

```console
$ iroh-tunnel serve status
node_id: 1aa27080cc694eb1e756c5e9260eb267208f7b677e6a74e53146bf417fb31f84
home_relay: https://use1-1.relay.n0.iroh.link./
pid: 8412  started_at: 1755648000

SERVICE  PROTOCOL  LOCAL ADDR      STREAMS
-------  --------  --------------  -------
echo     tcp       127.0.0.1:8080  2

PEER       SERVICES  TRANSPORTS
---------  --------  -------------------------------------------------
acc01c0d…  echo      relay https://use1-1.relay.n0.iroh.link./ [active]

$ iroh-tunnel access status
node_id: acc01c0d4c694eb1e756c5e9260eb267208f7b677e6a74e53146bf417fb31f84
pid: 8460  started_at: 1755648010

SERVICE  LISTEN          PEER       TRANSPORTS
-------  --------------  ---------  -------------------------------------------------
echo     127.0.0.1:8080  1aa27080…  relay https://use1-1.relay.n0.iroh.link./ [active]
                                    direct 192.168.1.10:52618
```

Whether the role is running is decided from its ownership lock, not from the
pid in the file:

| Situation | Result |
|---|---|
| Running, file present | The tables (or the file with `--json`) |
| Not running, no file | Exit `1`: `serve is not running (no serve-status.json found at <path>)` |
| Running, no file at this path | Exit `0`, a line naming the owner and noting another state directory may be in use; with `--json`, an object with `running: true` and `snapshot: null` |
| File present but stale (role stopped, or another pid owns the role) | The snapshot, plus a warning on stderr |

## Status files

Each running role writes a JSON snapshot to its state directory:
`serve-status.json` and `access-status.json`, side by side. See
[File locations](configuration.md#file-locations); `IROH_TUNNEL_STATE_DIR`
puts the files directly in another directory.

The files are written atomically (temporary file, fsync, rename), refreshed
about every 5 seconds and rewritten only when something changed. A role
removes its own file on shutdown.

### Serve

Top level: `node_id`, `home_relay` (or `null`), `pid`, `started_at` (Unix
seconds), `services` and `connections`.

- `services[]`: `name`, `protocol`, `local_addr` (the target) and
  `active_connections`, which counts active streams (channels), not Iroh
  connections. UDP services add a [`udp` block](udp.md#diagnostics).
- `connections[]`: one entry per connected peer.

```json
{
  "peer": "1aa27080cc694eb1e756c5e9260eb267208f7b677e6a74e53146bf417fb31f84",
  "services": ["echo"],
  "transports": [
    { "kind": "relay",  "addr": "https://use1-1.relay.n0.iroh.link./", "active": true },
    { "kind": "direct", "addr": "192.168.1.10:52618",                 "active": true }
  ],
  "local_bound_addrs": ["0.0.0.0:57382", "[::]:52921"]
}
```

- `peer` is the remote node's full NodeId; `services` lists the services it
  uses.
- `transports` are the paths iroh knows for that peer. A peer can be
  relay-only, direct-only, or have both active while the relay stays as
  fallback. `active` reflects iroh's snapshot at the last refresh.
- `local_bound_addrs` are the endpoint's local UDP socket candidates, one per
  address family. They are endpoint-wide, not per path.

A peer disappears from `connections` when its last connection closes.

### Access

Top level: `node_id`, `pid`, `started_at` and one `services[]` row per
configured service:

```json
{
  "name": "echo",
  "listen_addr": "127.0.0.1:8080",
  "peer": "1aa27080cc694eb1e756c5e9260eb267208f7b677e6a74e53146bf417fb31f84",
  "transports": [
    { "kind": "relay", "addr": "https://use1-1.relay.n0.iroh.link./", "active": true }
  ],
  "local_bound_addrs": ["0.0.0.0:57382", "[::]:52921"]
}
```

- `peer` is the configured Serve NodeId, present before the first connection.
- `transports` are the live paths of the service's shared connection; empty
  until the first local client triggers a dial, or while it is disconnected.

## Log levels

Logs go to stderr. The default level is `info`, which includes peer connect
and disconnect lines.

| Flag | Level |
|---|---|
| `-q` | errors only |
| none or `-v` | info |
| `-vv` | debug |
| `-vvv` | trace |

`RUST_LOG` overrides all flags, for example
`RUST_LOG=iroh_tunnel=debug iroh-tunnel serve run`. `--color auto|always|never`
controls colors.

## Log events

Connection lines carry the remote peer's full NodeId in `peer=`, so you can
grep the same id on the other host. Transports are shown as `relay=<url>`
and `direct=<addr>`, comma-separated.

| Role | Event | Example |
|---|---|---|
| Access | Connected | `INFO connected to serve peer (multiplexed, 1aa27080…) via relay=https://use1-1.relay.n0.iroh.link./ peer=<serve id> svc_name=echo` |
| Access | Path changed | `INFO path changed relay→direct (now active: direct=203.0.113.7:41641) peer=<serve id> svc_name=echo` |
| Access | Disconnected | `INFO disconnected from serve peer (service echo, multiplexed) peer=<serve id>` |
| Serve | Peer connected | `INFO peer connected via relay=https://use1-1.relay.n0.iroh.link./ peer=<access id> service=echo` |
| Serve | Peer disconnected | `INFO peer disconnected (service echo) peer=<access id>` |

- **Connected** is logged on the first dial and again after every redial.
- **Path changed** is checked every 5 seconds and logged once per change:
  `relay→direct` after a successful hole punch, `direct→relay` when the
  direct path dies.
- `via paths pending` means iroh had no active path yet; the status file or a
  later `path changed` line shows the real path.
- Only the remote side's transports are known; iroh does not expose which
  local address a path uses.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Success |
| `1` | General error; also a shutdown abandoned by the watchdog |
| `2` | Configuration: invalid, missing or unusable config or arguments |
| `3` | Permission: the OS refused access, or a path failed the trust policy |
| `4` | Iroh: the endpoint could not be created or reached |
| `5` | Service management failed or is not supported here |
| `6` | Conflict: the role, or its public identity, is already running |
| `130` | Stopped by a second signal after the shutdown grace |
