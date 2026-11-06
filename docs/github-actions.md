# GitHub Actions

Two actions put a private service on a runner, or a runner service in front of
other machines, without opening any port.

| Action | Uses | What it does |
|---|---|---|
| Access | `naicoi92/iroh-tunnel@v0` (or `naicoi92/iroh-tunnel/access@v0`) | Opens a local port on the runner that reaches a remote Serve node |
| Serve | `naicoi92/iroh-tunnel/serve@v0` | Exposes a service on the runner under a NodeId |

Each run gets a fresh identity; there is no secret-key input. Anyone who knows
a NodeId and service name can connect, so the target must enforce its own
access protection (see the
[security model](how-it-works.md#security-model)).

## Access

Reach a private service from the job. `node-id` is the Serve node's NodeId.

```yaml
steps:
  - uses: naicoi92/iroh-tunnel@v0
    with:
      name: postgres
      node-id: ${{ vars.POSTGRES_NODE_ID }}
      listen: 127.0.0.1:15432
  - run: psql -h 127.0.0.1 -p 15432 -c 'select 1'
```

## Serve

Expose a service that runs in the job. The `node-id` output is what Access
clients dial.

```yaml
steps:
  - uses: actions/checkout@v7
  - run: ./start-dev-server.sh &   # listens on 127.0.0.1:8080
  - id: tunnel
    uses: naicoi92/iroh-tunnel/serve@v0
    with:
      name: web
      target: 127.0.0.1:8080
  - run: echo "Dial ${{ steps.tunnel.outputs.node-id }}"
```

A hostname `target` such as a service label resolves only in a `container:`
job. A job that runs directly on the host maps `ports:` and uses
`127.0.0.1:<port>` instead.

```yaml
jobs:
  serve-host:
    runs-on: ubuntu-latest
    services:
      postgres:
        image: postgres:17
        env:
          POSTGRES_PASSWORD: postgres
        ports:
          - 5432:5432
    steps:
      - uses: naicoi92/iroh-tunnel/serve@v0
        with:
          name: postgres
          target: 127.0.0.1:5432
```

In a `container:` job, use the service label as the hostname:

```yaml
jobs:
  serve:
    runs-on: ubuntu-latest
    container: ubuntu:24.04
    services:
      postgres:
        image: postgres:17
        env:
          POSTGRES_PASSWORD: postgres
    steps:
      - uses: naicoi92/iroh-tunnel/serve@v0
        with:
          name: postgres
          target: postgres:5432
```

## Two services

Use one step per service; every step has its own NodeId.

```yaml
steps:
  - uses: naicoi92/iroh-tunnel@v0
    with:
      name: postgres
      node-id: ${{ vars.POSTGRES_NODE_ID }}
      listen: 127.0.0.1:15432
  - uses: naicoi92/iroh-tunnel@v0
    with:
      name: redis
      node-id: ${{ vars.REDIS_NODE_ID }}
      listen: 127.0.0.1:16379
```

## Own relay

Without `relay-url`, the n0 public relays are used. For your own relay (see
[Self-hosted relay](self-hosted-relay.md)), set the same relays on Serve and
Access:

```yaml
steps:
  - uses: naicoi92/iroh-tunnel@v0
    with:
      name: postgres
      node-id: ${{ vars.POSTGRES_NODE_ID }}
      listen: 127.0.0.1:15432
      relay-url: https://relay.example.com
      relay-token: ${{ secrets.IROH_RELAY_TOKEN }}
```

`relay-url` takes `https` URLs, one per line. Pass `relay-token` from a secret.
Serve takes the same `relay-url` and `relay-token` inputs:

```yaml
steps:
  - uses: naicoi92/iroh-tunnel/serve@v0
    with:
      name: postgres
      target: 127.0.0.1:5432
      relay-url: https://relay.example.com
      relay-token: ${{ secrets.IROH_RELAY_TOKEN }}
```

## Serve and Access in one job

```yaml
steps:
  - id: serve
    uses: naicoi92/iroh-tunnel/serve@v0
    with:
      name: web
      target: 127.0.0.1:8080
  - uses: naicoi92/iroh-tunnel@v0
    with:
      name: web
      node-id: ${{ steps.serve.outputs.node-id }}
      listen: 127.0.0.1:18080
```

## Inputs and outputs

### Access

| Input | Required | Default | Description |
|---|---|---|---|
| `name` | yes | | Service name; must match the Serve side |
| `node-id` | yes | | NodeId of the Serve node |
| `listen` | yes | | Local `host:port` to listen on |
| `protocol` | no | `tcp` | `tcp` or `udp` |
| `relay-url` | no | n0 public relays | `https://` relay URLs, one per line |
| `relay-token` | no | | Relay authentication token; use a secret |
| `version` | no | the action's version | `X.Y.Z`, `vX.Y.Z` (optional `-pre` suffix) or `latest` |

| Output | Description |
|---|---|
| `node-id` | NodeId of this Access node |

### Serve

| Input | Required | Default | Description |
|---|---|---|---|
| `name` | yes | | Service name; Access nodes dial it |
| `target` | yes | | Local `host:port`, IP address or hostname |
| `protocol` | no | `tcp` | `tcp` or `udp` |
| `relay-url` | no | n0 public relays | `https://` relay URLs, one per line |
| `relay-token` | no | | Relay authentication token; use a secret |
| `version` | no | the action's version | `X.Y.Z`, `vX.Y.Z` (optional `-pre` suffix) or `latest` |

| Output | Description |
|---|---|
| `node-id` | NodeId of this Serve node |

## Behavior

- **Readiness (60 s limit).** Serve is ready when its status file has a
  `node_id`. It does not wait for relay registration, so an embedded client
  should retry dials for a few seconds
  ([#177](https://github.com/naicoi92/iroh-tunnel/issues/177)). There is no
  `home-relay` output.
- **Timeout.** If the 60 s limit is reached, the step fails and prints the
  last status and the end of the log.
- **Access, TCP.** The step binds the listener, opens and closes one probe
  connection to trigger the dial, then waits for an active transport.
- **Access, UDP.** Ready means the listener is bound.
- **Job end.** The tunnel stops and the final status is always printed. The
  log is printed when the job failed, with `secret_key` and `relay_token`
  redacted.

## Versions

| Reference | Runs |
|---|---|
| `@v0` | The latest 0.x stable release |
| `@vX.Y.Z` | That exact release |
| `version:` input | Overrides either; `latest` means the latest stable release |

The minimum version is 0.6.0, the first release that ships the actions.

## Platforms

| Runner | Supported |
|---|---|
| Linux amd64, arm64 | Yes (glibc; musl inside Alpine containers) |
| macOS arm64 | Yes |
| Windows amd64, arm64 | Yes |
| macOS Intel, Linux riscv64 | No |

On Windows, use at most one Access step and one Serve step per job.

## Embedded clients

A program in the job can skip the Access step and dial the Serve node
directly with an iroh endpoint:

- Dial the `node-id` with ALPN `iroh-tunnel/<name>`.
- TCP: open one bidirectional QUIC stream per TCP connection and carry raw
  bytes with no framing (`crates/engine/src/tcp.rs`, `crates/engine/src/proto.rs`).
- UDP framing is in the [UDP integration contract](udp-integration-contract.md).
- Use the same relays as the Serve step, and retry the dial for a few seconds
  after Serve starts.
