# iroh-tunnel

P2P port-forwarding tunnel (TCP/UDP) over [Iroh](https://iroh.computer).
Expose a local service to the internet via an Iroh `node_id` — no public IP,
port forwarding, or relay server required.

> **Status:** Phase 0 — foundation skeleton (CLI builds, commands dispatch to
> placeholders). Real serve/access logic lands in later tasks.

## Build

```sh
cargo build
cargo run -- --help
```

## Usage (CLI shape)

```
iroh-tunnel <ROLE> <COMMAND>

Roles:   serve | access
Commands:
  run       Run in the foreground
  config    Manage config (keygen | add | remove | list | show | edit | path)
  service   Manage systemd/launchd service (install | start | stop | ...)
```

Exit codes: `0` success · `1` general · `2` config · `3` permission · `4` iroh · `5` service.
