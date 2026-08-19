# Security Policy

## Supported versions

| Version                  | Supported |
|--------------------------|-----------|
| latest tagged release    | ✅ |
| older `0.x` tags         | ❌ |

iroh-tunnel is pre-1.0. Only the newest release receives security fixes;
please verify against the [latest release](https://github.com/naicoi92/iroh-tunnel/releases)
before reporting.

## Reporting a vulnerability

Use GitHub's **private vulnerability reporting**: open the
[Security tab](https://github.com/naicoi92/iroh-tunnel/security) and click
**Report a vulnerability**, or go directly to
<https://github.com/naicoi92/iroh-tunnel/security/advisories/new>.

Please **do not** open public issues, pull requests, or discussions for
security problems.

Include what you can:

- affected version (`iroh-tunnel --version`) and role (`serve` / `access`)
- OS / platform and how you installed (binary, `.deb`/`.apk`, Docker, Homebrew)
- logs at `-vv`, and a reproduction or PoC if you have one

## Scope

- **In scope:** the code in this repository — the tunnel binary, its config
  handling, and the service-management backends.
- **Out of scope:**
  - The Iroh network, relay infrastructure, and the `iroh` crate itself —
    report those to the [iroh](https://github.com/n0-computer/iroh) project.
  - Vulnerabilities in third-party dependencies — report upstream, but feel
    free to notify us privately as well so we can bump the dependency.
  - Issues in tools you tunnel through iroh-tunnel (the local service being
    exposed). iroh-tunnel forwards traffic; it does not secure the service
    behind it.

**Trust model:** the `access` side pins the serve `node_id` (an Iroh TLS
identity), so it always reaches the intended serve node. The `serve` side has
**no peer allowlist**: any peer that learns the `node_id` and the service
name can connect and reach the exposed local service. Treat the `node_id` as
a bearer capability.

## Handling

Best effort — there are no guaranteed response or fix timelines. Reports are
triaged as capacity allows; fixes are coordinated with the reporter and ship
in the next patch release when practical. Credit is given on request.
