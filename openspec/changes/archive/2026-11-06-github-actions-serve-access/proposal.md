## Why

Pipelines cannot reach private services through iroh without hand-written
scripts, and applications that embed an iroh client have no easy way to test
against a real Serve peer in CI. One-step actions, found on the GitHub
Marketplace and used like a Docker service, remove that friction for any
project. Every role already ships as a release binary, so the actions can
reuse them on Linux, macOS and Windows runners.

## What Changes

- Add an **Access action** `naicoi92/iroh-tunnel@v0` (the repository's root
  `action.yml`, listed on the Marketplace), also reachable as
  `naicoi92/iroh-tunnel/access@v0` with identical behaviour: dials a given Serve NodeId and
  exposes one service on a local listener for the rest of the job. Inputs:
  `name`, `node-id`, `listen`; optional `protocol`, `relay-url`, `relay-token`,
  `version`. Output: `node-id`.
- Add a **Serve action** `naicoi92/iroh-tunnel/serve@v0`: publishes one target
  (runner-mapped port or job-service hostname) for the rest of the job. Inputs:
  `name`, `target`; the same four optional inputs. Output: `node-id`.
- Both actions: fresh random identity per run, no secret-key input; return only
  when the tunnel is usable (fixed 60 s limit); stop the tunnel at job end and
  show its status and log when the job failed. Binaries come from release
  assets with checksum verification. No iroh-tunnel CLI, config or
  wire-protocol change.
- Release stage: after a stable `vX.Y.Z` release, move the major tag (`v0`) to
  it; Marketplace listing is confirmed by the owner in the release UI.
- CI: action changes are type-checked, unit-tested and checked for a stale
  bundle on pull requests, and both actions run end to end on main.
- Documentation: a README section with copy-paste examples and a docs page with
  inputs, outputs, behaviour and the embedded-client contract.

**Non-goals**: two jobs of one run talking to each other (future extension),
several services in one step, tuning inputs and diagnostic outputs, running the
tunnel as a job service container, persistent identities or secret inputs,
long-lived tunnels.

## Capabilities

### New Capabilities
- `github-actions-integration`: the Access and Serve GitHub Actions: inputs and
  outputs, binary provisioning and verification, readiness, identity rules,
  job-end cleanup, diagnostics and Marketplace metadata.

### Modified Capabilities
- `ci-cd-pipeline`: pull-request and main-branch stages also cover the actions
  (path filter, type/unit/bundle checks, end-to-end runs); the release stage
  moves the major action tag; the "tests cover the application only" rule is
  updated to allow the actions' own tests.

## Impact

- **Code**: `access/action.yml` plus its generated root copy, `serve/action.yml`, per-role
  directories with committed JavaScript bundles, shared TypeScript sources; new
  jobs or steps in the existing workflows; no change to the Rust crates.
- **Dependencies**: a TypeScript toolchain for the actions (managed with
  `bun`) and GitHub Actions toolkit packages; none added to the Rust workspace.
- **Public surface**: three `uses:` paths (Access at the root and at
  `/access`, Serve at `/serve`) and their inputs and outputs, versioned
  with the repository tags and pinned as `@v0` or `@vX.Y.Z`; renaming or
  removing one is a breaking change. The CLI, config format and wire protocol
  are unchanged.
- **Security**: identities are ephemeral per run; only public NodeIds leave a
  step; downloads are checksum-verified; no token permissions are needed;
  `relay-token` is masked. NodeId remains identity, not authorization: target
  services keep enforcing their own access control.
- **Release process**: the release job gains write access to move the major
  tag; the owner ticks "Publish this Action to the GitHub Marketplace" in the
  release UI (no API exists for it).
- **Docs**: README section and a new docs page, linked from the docs index.
