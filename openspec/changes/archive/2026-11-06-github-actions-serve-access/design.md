## Context

iroh-tunnel ships three executables (`iroh-tunnel`, `iroh-tunnel-serve`,
`iroh-tunnel-access`) as release archives for eight targets, with
`checksums.txt` (Linux, Windows) and `checksums-darwin.txt` (macOS) signed
keyless, plus GHCR images. Tags are `vX.Y.Z[-pre]`, created on `main` after the
test gates pass. Nothing in the repository helps a GitHub Actions job start a
role: `.github/actions/` holds only the internal `setup-rust` composite, and the
`ci-cd-pipeline` spec fixes exactly four workflows and limits workflow checks
to static lint.

Behaviour of the binaries this design relies on (verified against the current
build; no change needed):

- `<role> run -c <file>` with a config that has no `secret_key` generates a
  fresh key, saves it into that file and prints `NodeId: <id>` on stdout. Two
  such processes with different files get different NodeIds.
- Config `host` accepts hostnames (`postgres`, `localhost`); quick mode `-t`
  accepts numeric sockets only.
- With `IROH_TUNNEL_STATE_DIR=<dir>` the role writes `<role>-status.json` there
  (atomic, refreshed about every 5 s). Serve: `node_id`, `home_relay` (always
  `null` today, issue #177),
  `services[]`, `connections[]`. Access: `node_id`, `services[]` with `peer`
  and `transports` (empty until the first local client triggers a dial).
- A per-user ownership lock under the user's default state directory allows
  one process per role per user; Serve and Access coexist. The directory comes
  from `HOME` on Unix and from the LocalAppData known folder on Windows (not
  overridable by an environment variable). SIGINT/SIGTERM start a graceful
  drain of at most 3 s.
- TCP tunnel streams are raw byte pipes: one bidirectional QUIC stream per TCP
  connection on ALPN `iroh-tunnel/<name>`, no framing.
- Release archives are named `<exe>-<version>-<os>-<arch>[-musl].tar.gz`, or
  `.zip` on Windows; macOS ships arm64 only.

GitHub Marketplace lists one action per public repository, from `action.yml`
at the root, with a unique name and `branding`. Actions in sub-directories are
usable (`owner/repo/dir@ref`) but not listed. Listing a release is a manual
step in the release UI with 2FA; there is no API for it.

Stakeholders: pipelines reaching private services through Access (priority),
application developers embedding an iroh client who need a Serve in CI, and
this repository's CI and release process.

## Goals / Non-Goals

**Goals:**
- One step starts Access or Serve for one service, returns when the tunnel is
  usable and outputs the NodeId, on Linux, macOS (arm64) and Windows runners
  and in jobs that run in `container:`.
- Each action exposes only its role's inputs, one plain value each.
- A fresh identity per step run; only public values leave the step.
- The process never outlives the job; failures leave a readable status and
  log in the job output.
- `naicoi92/iroh-tunnel@v0` (Access, also `naicoi92/iroh-tunnel/access@v0`) is
  listed on the Marketplace and follows the latest 0.x release, as does
  `naicoi92/iroh-tunnel/serve@v0`.
- This repository's CI tests both actions against the commit's own build.

**Non-Goals:**
- Two jobs of one run talking to each other; several services in one step.
- Tuning inputs (readiness level, timeout, log display) and diagnostic outputs.
- Tunnel as a job service container, persistent keys, any change to the Rust
  crates, CLI, config format or wire protocol.
- Sigstore verification inside the actions (checksum only, as agreed).
- Automating the Marketplace listing (not possible).

## Decisions

### D1. Bounded context and module seams
The actions are a separate "CI provisioning" context that consumes the
published CLI as an external system; they share no code with the Rust crates.
Both roles share one core; each role contributes only its request type and its
readiness rule.

```
 entry points: access/main, access/post, serve/main, serve/post
        |
 use cases: start (validate -> install -> launch -> await ready -> outputs)
            stop  (read state -> report status -> terminate -> show log -> clean)
        |
 domain: AccessRequest (name, nodeId, listen, protocol, relays)
         ServeRequest  (name, target, protocol, relays)
         Readiness rule per role; ReleaseTarget (runner os/arch -> asset names)
        |
 adapters: ActionsIO (inputs, outputs, state, masking, log groups)
           ReleaseSource (download, checksum, extract, tool cache)
           ProcessLauncher (spawn detached, signal, kill)
           StatusFile (read status JSON)   TcpProbe (one connect/close)
```

Domain invariants, all checked before any download or spawn:
- `name` follows the CLI rule (`[a-z0-9-]`, 1..63 bytes).
- `protocol` is `tcp` or `udp`.
- `listen` and `target` are `host:port` (IPv4, bracketed IPv6, or a hostname
  for `target`) with a port in 1..65535; `node-id` is a NodeId in either
  accepted encoding (64 hex or 52 base32).
- `relay-url` entries are `https` URLs, as the CLI requires.
- `relay-token` is trimmed and must be one non-empty line.
- `version` is `X.Y.Z` or `vX.Y.Z` (optionally `-pre`), or `latest`.

Only adapters touch the network, file system or processes, so use cases are
unit-tested with fakes. No plugin system or generic role framework.

### D2. Layout: one directory per role, root as the Access alias
```
 access/action.yml    Access metadata (source of truth)
 access/dist/         Access bundle: main.js, post.js
 action.yml           Access alias at the root (listed on the Marketplace)
 serve/action.yml     Serve metadata
 serve/dist/          Serve bundle: main.js, post.js
 action-src/          TypeScript sources, tests, package.json, bun.lock
```
Access is reachable as both `naicoi92/iroh-tunnel@v0` and
`naicoi92/iroh-tunnel/access@v0`; they are the same action with the same
inputs, outputs and behaviour. `access/action.yml` is the source of truth; the
root `action.yml` is generated from it by the build and committed, because the
Marketplace needs metadata at the root. The two files differ only in the `runs`
paths: `access/action.yml` uses `dist/main.js` and `dist/post.js` (relative to
its own directory), the root uses `access/dist/main.js` and
`access/dist/post.js`, so both run the same bundle and no code is duplicated.
Both carry `name: iroh-tunnel`, description, `branding` and
`runs: { using: node24, main, post, post-if: always() }`; `serve/action.yml`
has the same shape pointing at its own `dist/`. A composite wrapper at the root
was rejected: it has no post-step of its own and would hide the inner step's
outputs behind an extra mapping. Sources
live in one package (`action-src/`) because the roles share the core; bundles
are built with `bun build` and committed, since the runner runs the checked-out
files without installing dependencies. Dependencies: `@actions/core`,
`@actions/tool-cache`, `@actions/exec`, pinned in `action-src/bun.lock`.

### D3. Always run from a generated config file
Each step writes `<RUNNER_TEMP>/iroh-tunnel-<random>/<role>.toml` (0600 on
Unix) with one `[[services]]` entry, optional `relay_urls` and `relay_token`,
and starts `iroh-tunnel-access run -c <file>` or `iroh-tunnel-serve run -c
<file>`. Config mode accepts hostname targets (needed for `container:` jobs,
`postgres:5432`), which quick mode does not. The file has no `secret_key`: the
binary generates one and saves it into this job-private file, which the
post-step deletes. The actions never read, print or output the key.

### D4. Inputs and outputs (full shape in plan.md `Contracts`)
| Input | Access | Serve | Notes |
|---|---|---|---|
| `name` | required | required | service name, the ALPN suffix |
| `node-id` | required | — | Serve NodeId to dial |
| `listen` | required | — | local `host:port` |
| `target` | — | required | `host:port`, IP or hostname |
| `protocol` | optional | optional | `tcp` (default) or `udp` |
| `relay-url` | optional | optional | one URL per line; default n0 public relays |
| `relay-token` | optional | optional | masked |
| `version` | optional | optional | see D6 |

Outputs: Access `node-id` (its own); Serve `node-id` (what clients dial) and
nothing else (no `home-relay`, see D7 and issue #177). Reasons: inputs mirror the CLI flags (`-n`, `-N`, `-l`, `-t`,
`-p`, `--relay-url`), need no parsing beyond trimming, and give a precise error
per input; every input belongs to the role of the action it is written on.
Anything else is fixed behaviour (D7, D8) and can become an optional input
later without breaking callers. Each metadata file also declares one internal
input, `job-status`, defaulting to `${{ job.status }}` and described as
"Internal; do not set"; the runner re-evaluates it for the post step, which is
how the post step knows whether the job failed (D8). Consumer docs omit it.

### D5. Isolated state per step
The child process runs with `IROH_TUNNEL_STATE_DIR` and, on Unix, `HOME` set to
directories inside the step's temp directory. The status file lands where the
action reads it, and each step has its own ownership lock, so several Serve or
Access steps in one job (one per service) and concurrent jobs on one
self-hosted Unix runner do not refuse each other. On Windows the lock follows
the LocalAppData known folder, so **one Access step and one Serve step per
job** is the limit there; a second same-role step fails with a message naming
the limit (detected from the CLI's conflict exit code 6). On every OS, exit
code 6 is reported as "role already running for this runner user", naming a
system service or another step as possible causes. See Risks.

### D6. Binary provisioning and versioning
`version` accepts `X.Y.Z`, `vX.Y.Z` (optionally with a pre-release suffix,
since release tags may carry one) or `latest`, which means the latest stable
release, as in `install.sh`. Default: the release recorded
in the bundle at build time, which is the version of the commit the ref points
to, so `@v0.6.0` installs 0.6.0 and `@v0` installs the release `v0` was moved
to. The platform maps to a release target: Linux amd64/arm64, using the musl
build when the step runs on musl libc (for example an Alpine `container:`) and
the glibc build otherwise; macOS arm64; Windows amd64/arm64. Others (macOS
x86_64, Linux riscv64) fail with the supported list. The
action downloads the role archive and the matching checksum file from
`https://github.com/naicoi92/iroh-tunnel/releases/download/v<ver>/`, verifies
SHA-256 before extracting, and caches the executable with the tool cache. The
first release containing the actions is the minimum accepted `version`,
recorded in the bundle as a constant. For
this repository's CI only, the undocumented environment variable
`IROH_TUNNEL_ACTION_BIN_DIR` skips the download and uses executables built by
the job.

### D7. Fixed readiness
Polled from the status file every 500 ms, for at most 60 s:

| Action | Ready when |
|---|---|
| Access, TCP | the listener is bound, the action has opened and closed one connection to `listen` (loopback of the same family for `0.0.0.0` / `[::]`) to trigger the dial, and the service's `transports` is non-empty |
| Access, UDP | the listener is bound (a UDP dial cannot be triggered safely); logged as such |
| Serve | `node_id` is present in the status file. `home_relay` is not used: it is captured once at bind and stays `null` (verified with the 0.5.0 release; issue #177), so clients may need a few seconds of retries before the node is reachable through a relay |

"Listener bound" is read from the tunnel log, because the status file has no
listener field: the line `listening for local clients listen_addr=<addr>` (or
the UDP variant) marks it bound. Access does not exit when its listener fails
to bind; it logs `ERROR failed to bind <addr>` and keeps running (verified),
so the action watches the log for that line and fails at once naming
`listen`. If the process exits first, the step fails at once with its exit
code, the CLI exit class (2 config, 4 iroh, 6 conflict, ...) and the log tail.
On timeout it fails with the last status and the log tail. The process gets
`NO_COLOR=1` so these lines are plain text (verified; the standalone
executables reject `--color` together with `run`).

### D8. Process lifecycle and diagnostics
`main` spawns the executable detached, stdout/stderr appended to a log file in
the temp directory, saves pid and paths with `core.saveState`, waits for
readiness, sets outputs and returns; the tunnel keeps running for later steps.
`post` always runs: prints the final status in a collapsed group, sends
SIGTERM (Unix) and waits up to 5 s before SIGKILL, or terminates the process on
Windows (no console to deliver Ctrl+C to), prints the log only when the job
failed (internal `job-status` is `failure` or `cancelled`, or the main step did
not succeed), deletes the temp directory with the key, and fails if the process is
still alive. Log lines are passed through unchanged; the CLI never logs the
key.

### D9. Release and Marketplace
The release stage gains a final job after the GitHub Release is published: for
a stable tag `vX.Y.Z` that is the highest stable version of major `X`, it
force-moves `vX` to the same commit; pre-release tags and late lower patches do
not move it. A failed move fails the run and logs the manual command; the
published release is not touched. The job needs `contents: write`. One tag serves both actions,
since they live in the same repository. The Marketplace listing is a manual
owner step: on the first release, accept the Marketplace agreement and tick
"Publish this Action to the GitHub Marketplace"; for later versions to appear
on the listing, edit each release and tick it again. The CONTRIBUTING release
checklist records this. Callers pinning `@v0`, a version or a SHA are not
affected by whether a version is listed.

### D10. CI coverage within the existing four workflows
No new workflow file. `ci.yml` gains an `action` path filter (`action-src/**`,
`action.yml`, `access/**`, `serve/**`, `.github/workflows/ci.yml`, and
`Cargo.toml` because the bundles embed its version) and an `action` job:
`bun install --frozen-lockfile`, typecheck, unit tests, and a check that both
committed bundles and the generated root `action.yml` match a fresh build. `e2e.yml` builds the role executables
and, on Linux, macOS and Windows, runs the actions from the checkout with
`IROH_TUNNEL_ACTION_BIN_DIR` and the n0 public relays: `uses: ./serve` in front
of a local TCP echo target, `uses: ./` dialing its `node-id`, and a round trip
through the tunnel. Linux and macOS add a second Access step through
`uses: ./access`; Windows runs that step with `continue-on-error` and the CI-only
`IROH_TUNNEL_ACTION_ERROR_FILE` (the main step writes its failure message
there), and asserts the file holds the one-step-per-role message; the post-steps' own liveness check covers cleanup.
Linux also runs a `container:` job with a hostname Serve target.

### D11. Documentation for consumers
The README gets a short "GitHub Actions" section with the most common example
of each role and a link. A new `docs/github-actions.md` holds: copy-paste
examples (Access to a private service; Serve in front of a postgres job
service; Serve in a `container:` job; two services as two steps; own relay
with a token; Serve and Access in one job), the input and output tables, the
fixed behaviour (readiness, including the one connection Access opens and
closes to the remote service; 60 s limit; cleanup; logs), version pinning, the
supported platforms, the Windows limit, and the embedded-client contract (dial `node-id` with ALPN
`iroh-tunnel/<name>`, one bidirectional stream per TCP connection carrying raw
bytes; use the same relays as the Serve step).

## Risks / Trade-offs

- [Public n0 relays slow or rate-limited on shared runners] → `relay-url` for
  self-hosted relays; a clear timeout message; repo e2e can use its existing
  self-hosted relay container on Linux.
- [Fixed 60 s readiness limit too short for some networks] → message suggests
  a closer relay; an optional timeout input can be added later without
  breaking callers.
- [Windows: one step per role per job] → documented; fails with a message
  naming the limit; lifting it needs a CLI-side lock directory override, out of
  scope.
- [Committed bundles drift from sources] → CI fails when a fresh build differs.
- [Windows stop is a hard terminate] → acceptable at job end; status captured
  before termination.
- [Release host unavailable] → tool cache reuse on a runner; explicit error
  naming the URL.
- [Access readiness opens one connection to the remote service] → documented;
  it carries no data and closes at once.
- [Serve is not listed on the Marketplace] → linked prominently from the
  Access listing and README.
- [Root and `access/` metadata drift apart] → the root file is generated, and
  CI fails when it differs from a fresh build.
- [Marketplace versions require a manual tick per release] → release checklist;
  pinned callers unaffected.
- [Moving `v0` is a force tag update] → done only by the release job on stable
  tags, after the release is published.

## Migration Plan

Additive. Callers opt in by adding a step; nothing changes for users of the
binaries. Rollback: pin an earlier exact tag, or move `v0` back to the previous
release commit. The first release containing the actions becomes the minimum
`version` and the first Marketplace listing.

## Open Questions

None blocking.
