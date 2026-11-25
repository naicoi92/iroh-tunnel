# Product

## Purpose

`iroh-tunnel` provides peer-to-peer port forwarding over Iroh for operators and developers who need to reach services without exposing a public IP or configuring router port forwarding. A Serve node runs near the service; an Access node exposes a local listener to clients on another machine. Iroh provides encrypted connections and uses relays for connectivity when needed.

## Product surfaces

- Combined and role-specific command-line binaries configure and run Serve and Access tunnels.
- TOML configuration and service management support persistent operation.
- GitHub Actions start job-scoped Serve or Access tunnels using verified release binaries.
- Release packages, installers, containers and self-update distribute and maintain the binaries.

TCP forwarding is supported; UDP forwarding is experimental. Connection status and diagnostics help operators inspect tunnel health. The capability specifications in this directory describe the detailed contracts.
