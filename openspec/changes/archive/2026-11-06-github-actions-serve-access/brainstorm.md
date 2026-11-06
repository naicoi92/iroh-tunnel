## Design Summary

**Intent.** Give CI/CD pipelines a real, job-scoped iroh-tunnel endpoint in
one step. The priority is **Access**: a pipeline reaches a private service
(staging database, internal API) that an existing Serve peer publishes outside
the runner, without opening ports or a VPN. The second use is **Serve**:
applications that embed an iroh client need a live Serve peer in CI, in front
of a test service such as PostgreSQL, the way a workflow uses a Docker
`postgres` service.

**What ships (v1).** Two small GitHub Actions in this repository, one per role,
each with its own directory and only the inputs its role needs:

| Action | `uses:` | Marketplace |
|---|---|---|
| Access | `naicoi92/iroh-tunnel@v0`, alias `naicoi92/iroh-tunnel/access@v0` | listed (root `action.yml`) |
| Serve | `naicoi92/iroh-tunnel/serve@v0` | usable, linked from the Access page |

- **Access** dials a given Serve NodeId and exposes one service on a local
  listener, so later steps connect to `127.0.0.1:<port>`.
- **Serve** publishes one local target (`127.0.0.1:<port>` for a job service
  mapped to the runner, or a hostname such as `postgres:5432` for a job running
  in `container:`) and outputs its NodeId so later steps can dial it.

The tunnel keeps running for the rest of the job and is stopped, with its
status and log shown on failure, when the job ends.

**Simple to use.** One step = one service. Each action takes only its own
inputs, one plain value each (`name`, `node-id`, `listen` / `name`, `target`),
plus the same four optional ones (`protocol`, `relay-url`, `relay-token`,
`version`). Waiting, timeouts and log display are fixed behaviour, not inputs.

```
 job CI (one runner)                         outside the runner
 +---------------------------------------+   +------------------------+
 | - uses: naicoi92/iroh-tunnel@v0       |   | Serve (existing,       |
 |     name: db, node-id: <id>,          |==>| stable NodeId) in      |
 |     listen: 127.0.0.1:15432           |   | front of staging DB    |
 | - run: migrations / tests on :15432   |   +------------------------+
 | - (job end) tunnel stopped            |
 +---------------------------------------+
```

**Goals.**
- A pipeline gets a working Access or Serve endpoint in one step, on Linux,
  macOS and Windows runners, including jobs that run in a `container:`.
- Another project can adopt it by copying a README example; every input is
  obvious from its name and belongs to the role it is written for.
- Every run has its own fresh identity; concurrent jobs and runs never collide.
- Only public values (NodeIds) are passed between steps.
- The step returns only when the tunnel is really usable, so tests do not race.
- Failures are diagnosable from the job log without re-running.
- Access is listed on the GitHub Marketplace; versions follow iroh-tunnel
  releases.

**Non-goals (v1).**
- Two jobs in the same run talking to each other; recorded as a future
  extension.
- Several services in one step; use one step per service.
- Tuning inputs (readiness level, timeout, log display) and diagnostic outputs
  (file paths); they can be added later without breaking anyone.
- Long-lived tunnels, persistent identities or secret-key inputs.
- Running the tunnel itself as a job service container (`jobs.<id>.services`):
  it starts before checkout, quick mode takes numeric targets only, and it is
  Linux-only; the in-job step covers the same need.

**Anti-goals.** It is a failure even if it "works" when:
- a fixed secret key is required, or two concurrent jobs can end up with the
  same NodeId;
- a secret appears in logs or outputs;
- a started tunnel outlives its job or is left running after a failed step;
- the actions need a change to the iroh-tunnel CLI or wire protocol;
- using them requires learning a custom syntax or inputs of the other role.

**Constraints.**
- No public CLI change: the actions rely on existing behaviour (the role prints
  `NodeId: <id>`; the status file carries `node_id` and Access
  `transports`; config `host` accepts hostnames). Verified against the current
  build.
- Binaries come from the published release assets with checksum verification.
- Marketplace rules: public repository, one listed action from the root
  `action.yml`, unique name (`iroh-tunnel` is free), branding metadata; the
  listing is confirmed by the owner in the release UI with 2FA.
- Existing CI test and release gates still hold; the actions are exercised in
  CI.
- Repository content is in English.

**Left to the builder.** How readiness is detected, the log/status
presentation, the shared code layout, the CI test matrix, and the README
layout.

## Alternatives Considered

### Alternative A: Composite actions (bash + PowerShell)
- **Approach**: Shell steps download the binary, start it in the background,
  poll readiness and print outputs.
- **Pros**: No dependencies or build step; easy to read.
- **Cons**: No post-step, so the tunnel cannot be stopped cleanly or its log
  shown when a later step fails; two shell implementations for Windows.
- **Why not chosen**: Cleanup and diagnosis at job end are core goals.

### Alternative B: A separate repository per action
- **Approach**: `iroh-tunnel-access` and `iroh-tunnel-serve` action
  repositories, each with a root `action.yml`.
- **Pros**: Both listed on the Marketplace; action-only READMEs.
- **Cons**: Versions synced with binary releases across repositories; CI cannot
  test the actions against the commit's own build; extra release processes.
- **Why not chosen**: Keeping actions, binaries, versions and tests in one
  place matters more than a second listing.

### Alternative C: Docker (GHCR image as a job service or everywhere)
- **Approach**: Run the published image under `jobs.<id>.services` with
  `command:`, reading the NodeId from the container log, or for every mode.
- **Pros**: Declarative, next to the test service; image already published.
- **Cons**: Linux only; starts before checkout; quick mode rejects hostnames;
  NodeId scraped from logs; no job-end diagnostics.
- **Why not chosen**: In-job steps cover the same needs on all three OSes.

### Alternative D: One action with a `role` input
- **Approach**: A single root action; `role: serve | access` selects which
  other inputs apply.
- **Pros**: One Marketplace page covers both roles.
- **Cons**: Users see inputs of both roles and must learn which apply; errors
  for "wrong input for this role" become necessary.
- **Why not chosen**: Rejected in review as too complex to read; one action
  per role keeps each input list short.

### Alternative E (two jobs): run-scoped deterministic key or artifact rendezvous
- **Approach**: Derive a per-run key from a shared secret, or exchange the
  NodeId and a "done" signal through run artifacts.
- **Pros**: Enables cross-job, cross-OS tests.
- **Cons**: Needs a long-lived secret (anti-goal) or roughly doubles the scope.
- **Why not chosen**: Deferred; v1 targets single-job use.

## Agreed Approach

Two JavaScript/TypeScript actions on the node24 runtime with a post-step, one
per role, in this repository: Access is the root action
`naicoi92/iroh-tunnel@v0` (also reachable as `naicoi92/iroh-tunnel/access@v0`)
and is listed on the Marketplace; Serve is `naicoi92/iroh-tunnel/serve@v0`. Each has its own directory and only its own
inputs, fixed waiting and logging behaviour, and runs release binaries on all
three runner OSes. It won over A because the post-step gives clean shutdown
and diagnostics from one codebase, over B because versions and CI tests stay
with the binaries, over C because of cross-OS support and hostname targets
without a CLI change, and over D because each role's input list stays short.

## Key Decisions

- Priority: Access (CI reaches a private service through an existing Serve);
  Serve for apps with an embedded iroh client is the second use.
- Access is the root action `naicoi92/iroh-tunnel@v0`, listed on the
  Marketplace, with `naicoi92/iroh-tunnel/access@v0` as an equivalent path, so
  both roles can be written symmetrically (`.../access`, `.../serve`); Serve is
  `naicoi92/iroh-tunnel/serve@v0`; each role has its own directory.
- One service per step; only the role's own inputs; no list syntax, no `role`.
- Minimal surface: readiness, timeout and log display are fixed behaviour;
  outputs are only the NodeIds.
- Implementation style: JavaScript/TypeScript, node24, with a post-step.
- Binary source: release assets with checksum verification.
- No service-container mode; Serve targets may be runner-mapped ports or
  hostnames.
- Identity: a fresh random identity per run; no secret-key input; no CLI change.
- Two jobs in one run: out of scope for v1, documented as a future extension.
- A README section with copy-paste examples is part of the deliverable.
- Spec impact: a new capability for the actions, and CI pipeline requirements
  that test them and move their major tag on release.

## Open Questions

None blocking. A later version may add tuning inputs or a combined
"connection hint" output if users ask for them.
