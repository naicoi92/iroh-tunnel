# Configuration

Quick mode (`serve -n NAME -t ADDR`, `access -n NAME -l ADDR -N NODE_ID`)
needs no file. For more than one service, a stable identity without
`--key-file`, custom relays or a [system service](services.md), each role
reads a TOML file.

- [File locations](#file-locations)
- [Managing the file with `config`](#managing-the-file-with-config)
- [Serve file](#serve-file)
- [Access file](#access-file)
- [`[node]` settings](#node-settings)
- [Identity and keys](#identity-and-keys)
- [Relays](#relays)
- [Validation rules](#validation-rules)
- [How files are written](#how-files-are-written)
- [Legacy settings](#legacy-settings)

## File locations

| OS | Config file | State directory (status files, locks) |
|---|---|---|
| Linux, macOS | `~/.config/iroh-tunnel/<role>.toml` | `~/.local/state/iroh-tunnel/` |
| Windows | `%LOCALAPPDATA%\iroh-tunnel\config\<role>.toml` | `%LOCALAPPDATA%\iroh-tunnel\state\` |

`<role>` is `serve` or `access`. `XDG_CONFIG_HOME` and `XDG_STATE_HOME` are
ignored. Every command that reads or writes a config accepts
`--config PATH` (`-c`); `IROH_TUNNEL_STATE_DIR` moves the status files.
`iroh-tunnel <role> config path` prints the file in use.

On macOS, versions before 0.5.0 kept the config under
`~/Library/Application Support/iroh-tunnel/`. When the new file is missing and
the legacy one exists, it is copied (never moved or overwritten, owner-only
permissions) and a notice is printed. If the copy fails, the command stops
with exit code `2` and creates no new identity.

Sample files: [`examples/serve.toml`](../examples/serve.toml) and
[`examples/access.toml`](../examples/access.toml), used by the
[compose demo](deployment.md#compose-demo).

## Managing the file with `config`

```sh
iroh-tunnel serve config keygen                       # create or rotate the identity
iroh-tunnel serve config set -n postgres -t 5432
iroh-tunnel serve config list
iroh-tunnel serve config show                         # secrets hidden; --reveal prints them
iroh-tunnel serve config rm -n postgres
iroh-tunnel serve config edit                         # $EDITOR, validated on exit
iroh-tunnel serve config check                        # validate only
iroh-tunnel serve config path

iroh-tunnel access config set -n postgres -l 55432 -N <SERVE_NODE_ID>
iroh-tunnel serve config set -n postgres -t db.internal:5433   # update: prints old -> new
```

`config set -n NAME` creates or updates one service. Serve takes
`-t`, `--target [HOST:]PORT`; Access takes `-l`, `--listen [HOST:]PORT` and
`-N`, `--node-id`. Both take `-p`, `--protocol` (`tcp` default, or `udp`). A
bare `PORT` means `127.0.0.1`; `HOST` may be an IPv4 address, a hostname or a
bracketed IPv6 address (`[::1]:5432`). Creating a service needs its address
(and the NodeId for Access); on update, only the options you pass change, and
`set` prints `Updated service ...` with `field: old -> new` lines, or
`Service ... unchanged`. `set` and `keygen` create the file and its directory
when missing. `list`, `show`, `rm`, `check` and `run` on a missing file fail
with exit code `2`, name the path, point to `config set` / `config keygen`,
and create nothing. `config check` only validates and prints
`OK: <path> is a valid <role> config.`

## Serve file

```toml
[node]
secret_key = "<base64>"           # see Identity and keys

[[services]]
name = "postgres"                 # also the ALPN Access dials
protocol = "tcp"                  # tcp | udp
host = "127.0.0.1"                # target host (default 127.0.0.1)
port = 5432                       # target port
```

For each service, Serve accepts tunnel streams for `name` and connects each
one to `host:port`. `host` may be a hostname, which is useful in containers
(`postgres.default.svc.cluster.local`).

## Access file

```toml
[node]
secret_key = "<base64>"

[[services]]
name = "postgres"                 # must match the Serve service name
node_id = "<serve NodeId>"        # 64 hex or 52 base32 characters
protocol = "tcp"
host = "127.0.0.1"                # local listen host (default 127.0.0.1)
port = 55432                      # local listen port
# relay_urls = ["https://relay.example.com"]   # per-service relay override
```

Access listens on `host:port` and forwards each client to the service
`name` on the Serve node `node_id`. Use `host = "0.0.0.0"` only when other
machines should reach the listener; the tunnel does not authenticate local
clients.

UDP services accept extra tuning keys; see [UDP](udp.md#tuning).

## `[node]` settings

| Key | Roles | Default | Meaning |
|---|---|---|---|
| `secret_key` | both | generated | Base64 of 32 bytes; defines the NodeId |
| `relay_urls` | both | n0 public relays | Relay set; the first is home, the rest are failover |
| `relay_token` | both | none | Bearer token for relays that enforce `access.shared_token` |
| `max_concurrent_streams` | both | transport default (100) | Concurrent tunnel streams per connection; at least 1 |

`max_concurrent_streams` is headroom, not a requirement. Worst-case buffer
memory grows with `max_concurrent_streams × stream_receive_window`, so raise
it only when a measured workload needs more than 100 concurrent channels to
one Serve. When all slots are busy, a new channel waits for a free stream.

## Identity and keys

A node's identity is its NodeId, the public key of `secret_key`.

- `config keygen` writes a new key and prints `New NodeId: <id>`. Replacing an
  existing key changes the NodeId, so it asks first; `--yes` skips the
  question, and without a terminal it fails and keeps the key.
- `config keygen --print` prints a `Secret key:` and its `Node ID:` and writes
  nothing (it cannot be combined with `-c` or `-y`). Use it for a secret store or a `--key-file`.
- When `run` finds no key, it generates one and saves it into the file
  (owner-only), so the NodeId stays the same across restarts. If the file
  cannot be written (for example a read-only mount), `run` fails; generate the
  key in advance instead.
- Quick mode uses a fresh identity on every start unless you pass
  `--key-file PATH`. The file holds only the base64 key, the `Secret key:`
  value printed by `config keygen --print`; keep it owner-only.
- A Serve and an Access running at the same time must use different keys.

The NodeId is public: Access needs it to dial. The secret key is not; keep it
out of shared configs and logs. See the
[security model](how-it-works.md#security-model) for what a NodeId does and
does not protect.

## Relays

With no `relay_urls`, both roles use the n0 public relay network. Relays help
the two nodes find each other and carry encrypted traffic when a direct path
is not possible; they never see plaintext.

```toml
[node]
relay_urls = ["https://relay.example.com", "https://relay2.example.com"]
relay_token = "<token>"           # only for relays that enforce a shared token
```

- Every URL must start with `https://`.
- `relay_token` is sent to every relay in `relay_urls` as
  `Authorization: Bearer <token>`; relays without access control ignore it.
  One token applies to all of them. Without `relay_urls` the token is unused
  and a warning is logged.
- Serve must be registered on every relay its Access peers dial through.
- An Access service can set its own `relay_urls`. That service dials through
  its own list; an empty or missing list falls back to `[node] relay_urls`.
- In quick mode, `--relay-url URL` on both roles sets a single custom relay.

To run your own relay, see [Self-hosted relay](self-hosted-relay.md).

## Validation rules

A file is validated when it loads and before every save. It is rejected when:

- a service `name` is not lowercase letters, digits and `-`, is longer than 63
  bytes, or appears twice in one file;
- `port` is `0`;
- `protocol` is not `tcp` or `udp`;
- an Access `node_id` is not 64 hex or 52 base32 characters;
- a relay URL does not start with `https://`;
- `relay_token` is empty or contains whitespace;
- `max_concurrent_streams` is `0`;
- `secret_key` is not base64 of exactly 32 bytes;
- UDP tuning keys appear on a TCP service or are out of range.

Errors name the file and line but never quote the line, so secrets do not end
up in output.

## How files are written

Every change made by `config`, and the key that `run` saves, is:

- validated first; a failure exits with code `2` and leaves the file
  byte-for-byte unchanged;
- written atomically: a temporary sibling file is fsynced and renamed;
- owner-only (`0600`) on Unix. On Windows an equivalent ACL is not enforced;
- serialized with an advisory lock on `<config>.lock`. A writer waits up to
  10 seconds, then fails with `another process is writing`;
- an in-place edit that keeps comments and formatting.

`config edit` opens a private copy in `$EDITOR`. When the editor exits, the
result is validated and saved. A rejected edit leaves the live file unchanged
and is kept in `<config>.rejected`. If the live file changed while you were
editing, your edit is not applied and is also kept in `<config>.rejected`.

## Legacy settings

`multiplex` on an Access service is ignored, whatever its value, and removed
the next time the application saves the file. Every Access service always
reuses one connection for all its channels; see
[How it works](how-it-works.md#one-connection-many-channels).
