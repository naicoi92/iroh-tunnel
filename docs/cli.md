# CLI reference

Run any command with `--help` for its full options.

```
iroh-tunnel [-v...|-q] [--color auto|always|never] <serve|access|update>
iroh-tunnel-serve  (its) [OPTIONS] [COMMAND]    # same as `iroh-tunnel serve`
iroh-tunnel-access (ita) [OPTIONS] [COMMAND]    # same as `iroh-tunnel access`
```

- [Quick mode](#quick-mode)
- [Role commands](#role-commands)
- [`config`](#config)
- [`service`](#service)
- [`update`](#update)
- [Global flags](#global-flags)
- [Environment variables](#environment-variables)

## Quick mode

A role with flags and no command runs one service with no config file:

```sh
iroh-tunnel serve  -n NAME -t [HOST:]PORT [-p tcp|udp] [-r URL] [-k KEY_FILE]
iroh-tunnel access -n NAME -l [HOST:]PORT -N NODE_ID [-p tcp|udp] [-r URL] [-k KEY_FILE]
```

| Flag | Role | Meaning |
|---|---|---|
| `-n`, `--name` | both | Service name; must match on both sides |
| `-t`, `--target` | Serve | Target `[HOST:]PORT`; a bare port means `127.0.0.1`; `HOST` may be an IP, a hostname or bracketed IPv6 (`'[::1]:5432'`) |
| `-l`, `--listen` | Access | Local listen `[HOST:]PORT` (same forms as `--target`) |
| `-N`, `--node-id` | Access | Serve NodeId |
| `-p`, `--protocol` | both | `tcp` (default) or `udp` ([experimental](udp.md)) |
| `-r`, `--relay-url` | both | Use this relay instead of the n0 public relays |
| `-k`, `--key-file` | both | File holding a base64 secret key, for a stable NodeId |

Without `--key-file`, each start uses a fresh identity, and nothing is written
to disk. Quick flags cannot be mixed with a command such as `run` or `config`.

## Role commands

| Command | Purpose |
|---|---|
| `<role> run [-c PATH]` | Run the services in the config file in the foreground |
| `<role> config ...` | Manage the config file |
| `<role> status [--json]` | Show the running role's status; see [Monitoring](monitoring.md) |
| `<role> service ...` | Manage a background service (all-in-one executable only) |
| `its update`, `ita update` | Update the standalone executable |

## `config`

All subcommands accept `-c`, `--config PATH` except `keygen --print`.

| Subcommand | Purpose |
|---|---|
| `keygen [-y]` | Generate or replace the node's `secret_key`; asks before replacing |
| `keygen --print` | Print a new secret key and its NodeId; writes nothing; conflicts with `-c` and `-y` |
| `set -n N -t [HOST:]PORT [-p P]` (Serve) | Create or update a service; prints what changed |
| `set -n N -l [HOST:]PORT -N ID [-p P]` (Access) | Create or update a service; creating needs `-l` and `-N` |
| `rm -n N` | Remove a service |
| `check` | Validate the file without changing it; prints `OK: <path> is a valid <role> config.` |
| `list` | List services |
| `show [--reveal]` | Print the file; `secret_key` and `relay_token` are hidden unless `--reveal` |
| `edit` | Edit in `$EDITOR`; the result is validated before it is saved |
| `path` | Print the config file path |

See [Configuration](configuration.md).

### Short options

| Short | Long | Meaning |
|---|---|---|
| `-c` | `--config` | Config file |
| `-n` | `--name` | Service name |
| `-t` | `--target` | Serve target `[HOST:]PORT` |
| `-l` | `--listen` | Access listen `[HOST:]PORT` |
| `-N` | `--node-id` | Serve NodeId |
| `-p` | `--protocol` | `tcp` or `udp` |
| `-r` | `--relay-url` | Relay URL (quick mode) |
| `-k` | `--key-file` | Secret key file (quick mode) |
| `-y` | `--yes` | Skip confirmation |
| | `--reveal`, `--print`, `--system`, `--user` | Long-only |

### Usage errors

Unknown, missing or invalid arguments (including a missing address when
creating a service) print `error: ...`, a blank line and the help of the failing
subcommand to stderr, and exit with code `2`. A bare `iroh-tunnel`, `its`,
`ita`, role, `config` or `service` prints help and exits `2`. Runtime and
config-file errors print only the error.

## `service`

| Subcommand | Purpose |
|---|---|
| `install [-c PATH] [--user\|--system]` | Register, enable and start the role |
| `start`, `stop`, `restart`, `status` `[--user\|--system]` | Control the registered service |
| `uninstall [--user\|--system]` | Stop and remove it |

See [Running as a service](services.md).

## `update`

```sh
iroh-tunnel update [--to VERSION] [-i] [-y]
its update ...
ita update ...
```

| Flag | Meaning |
|---|---|
| `--to VERSION` | Install this release (`0.6.0` or `v0.6.0`); allows downgrades |
| `-i`, `--insecure` | Skip signature and checksum verification, with a warning |
| `-y`, `--yes` | Replace a package- or Homebrew-owned executable anyway |

See [Updating](install.md#updating).

## Global flags

Global flags go before the role: `iroh-tunnel -vv serve run`, not
`iroh-tunnel serve run -vv`.

| Flag | Meaning |
|---|---|
| `-v`, `-vv`, `-vvv` | Log at info, debug, trace (default info) |
| `-q`, `--quiet` | Errors only |
| `--color auto\|always\|never` | Colored output |
| `-V`, `--version` | Print the version |

## Environment variables

| Variable | Effect |
|---|---|
| `RUST_LOG` | Log filter; overrides `-v` and `-q` |
| `IROH_TUNNEL_STATE_DIR` | Directory for the status files |
| `IROH_TUNNEL_BASE_URL` | Release host for `update` and the installers |
| `IROH_TUNNEL_CONTAINER=1` | Mark the executable as container-owned; `update` will not replace it |
| `EDITOR` | Editor used by `config edit` |
