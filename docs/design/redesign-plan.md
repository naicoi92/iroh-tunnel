---
title: Iroh tunnel redesign plan
summary: Target architecture, confirmed decisions, open gates, phased implementation and acceptance evidence for the redesign.
---

## Purpose and status

The [canonical tracker spec](https://github.com/naicoi92/iroh-tunnel/issues/68) records the product scope and open decision gates. This plan brings the approved redesign direction into one implementation roadmap. It is for maintainers preparing design decisions, implementation and release. Read the [capability specifications](redesign-spec.md) for detailed rules, scenarios and acceptance criteria, and the [decision record](project-redesign.md) for interview decisions.

**Status: planning and review.** The capability map and selected decisions are approved; proposed details are not blanket implementation approval. This document does not imply a working UDP implementation, completed migration, production readiness or a committed release schedule. No implementation was made during this design effort; the Iroh dependency upgrade is recorded in the [#101 evidence](issue-101-evidence.md).

**Non-goals:** application-protocol inspection or modification; source-IP transparency without an explicit feasible design; treating NodeId as authorization; converting the modular monolith into networked microservices; assuming a database; promising reliable UDP delivery.

## Decision status

| Area | Confirmed | Proposed / open before dependent work |
|---|---|---|
| Structure | DDD-oriented modular monolith; eight capability boundaries; three independently runnable binaries. | Crate layout, exact Rust interfaces and data cardinalities. |
| Protocol | TCP and best-effort UDP to fixed targets; multiplex always enabled; seek TCP compatibility with 0.4 and test both directions without adding version negotiation. | On 0.4 compatibility failure, user decides trade-off; retry/drain numeric defaults and flow-control policies remain open. |
| UDP mapping | Opaque payload, client-to-flow association at Access, dedicated Serve-side UDP socket per active flow. | Transport, payload size, fragmentation, timeout, admission, reply source filtering and tuple-reuse risks. |
| Configuration | Platform paths, explicit overrides, safe macOS migration, ephemeral quick-run identity. | Initial setup, secret disclosure, rotation and concurrent write details. |
| Operations | Role/scope exclusivity, service privilege behavior, pinned paths and independent updates. | Locking/stale state, privileged config trust, Windows user background, rollback and trust rotation. |
| Product scope | Windows plus currently released platforms, installers, separate packages, three images and verified updates. | Detailed platform matrix, exact CLI/exit codes and release recovery policy. |

**Dependency version:** target Iroh stable `1.4.1` was selected during design and adopted by [#101](issue-101-evidence.md); recheck the latest stable before final release evidence. No benchmark has established UDP transport performance.

## Capability boundaries

These are responsibility boundaries, not a mandate to create eight crates or eight processes. Dependencies are capability contracts, not necessarily build dependencies. See the [approved capability map](capability-map.md).

| Capability | Responsibility |
|---|---|
| `tunnel-contract` | Service identity, peer references, compatibility and shared tunnel rules. |
| `configuration` | Role configuration, identity, validation, migration and secret handling. |
| `runtime-ownership` | Role/scope ownership, lifecycle, status and cleanup. |
| `tcp-forwarding` | Multiplexed per-client TCP streams to fixed targets. |
| `udp-forwarding` | Opaque datagrams, per-client flow association and fixed-target forwarding. |
| `service-management` | User/system background service operations and platform adapters. |
| `distribution` | Independent binaries, packages, images, installers and verified updates. |
| `cli-experience` | Consistent role commands, help, options and diagnostics. |

## Executables and CLI

- `iroh-tunnel` (`it`) supports Serve and Access; `iroh-tunnel-serve` (`its`) and `iroh-tunnel-access` (`ita`) each support their own role. No executable requires another at runtime.
- Quick flags run directly; configured execution uses role `run --config`. Quick execution uses an ephemeral identity unless a key is provided. TCP is the default.
- Consistent short options and complete help are required; exact arguments and exit codes still require review. Breaking CLI and config changes are allowed. Seek TCP interoperability with 0.4, verify both directions, and present failures for user decision; no new version-negotiation framework is in scope.
- Multiplex is always active. Legacy `multiplex` is ignored silently and removed on config save.

## Forwarding contract

### TCP

Access accepts a local TCP client, obtains or reuses a compatible Iroh connection and opens an independent bidirectional stream. Serve connects that stream to the configured target. Multiple client streams share the service connection. Target failure isolates the affected channel; peer loss closes affected channels without replay. New clients retry with backoff under a bounded default deadline. Shutdown stops new clients and drains active channels for a bounded period; numeric defaults remain open. See TCP-01–02 and AC-TCP in the [specification](redesign-spec.md#tcp-forwarding--multiplexed-tcp).

### UDP

Access associates each listener and client endpoint with a runtime UDP flow. Serve uses a dedicated upstream UDP socket per active flow. Responses are routed back through the corresponding flow. The socket keeps its local endpoint for its lifetime, not necessarily after expiry or reconnect. Flows may share an Iroh connection; no dedicated task or Iroh connection per flow is implied.

The tunnel does not interpret or change application payloads and must preserve datagram boundaries for delivered messages. Framing or fragmentation inside the tunnel may be considered without altering that payload. UDP remains best-effort: loss, reordering and duplication are possible. A persisted `AccessMapping` identifies a configured service; it is not the runtime client-to-flow association.

The target generally sees the Serve-side endpoint, potentially modified by NAT, rather than the original client IP. A connected upstream socket filters replies from other endpoints; compatibility policy remains open. A late upstream response after UDP tuple reuse cannot be distinguished solely by a tunnel flow ID. Timeouts, cleanup, source filtering and reuse risk mitigation require explicit policy and tests; do not promise absolute isolation across every reuse. See UDP-01–03 and AC-UDP in the [specification](redesign-spec.md#udp-forwarding--datagram-sessions), and the [UDP decision map](https://github.com/naicoi92/iroh-tunnel/issues/69).

## Configuration, ownership and security

- macOS/Linux defaults: `~/.config/iroh-tunnel/` and `~/.local/state/iroh-tunnel/`, ignoring XDG directory variables; application-specific overrides remain. Windows uses the resolved `LocalAppData/iroh-tunnel/config` and `state` directories.
- On macOS use new config when present. Otherwise safely copy legacy config without overwriting, preserve the original and secret permissions, and never generate a substitute identity after copy failure. Verify actual legacy paths.
- For each role only one process runs at a time: a start is refused when a process of the role is detected, regardless of executable name; Serve and Access are distinct roles. A system service runs as root/Administrator and its installer owns it; the operator stops it before using a user service or `run`. Independent local endpoints must not use the same identity concurrently across roles or scopes; reject the later process. No cross-host intervention is planned. Per RO-03 (#77) the minimum is refusing a later start while an instance runs, keeping the 0.4.2 stop behavior; lock mechanics and crash cleanup are implementation details of #87.
- NodeId is public identity, not authorization. Services protect themselves. Secret permissions and disclosure policy require review.

## Background service and distribution

A non-elevated user chooses user or system service scope. A system choice without privilege stops with instructions to rerun under `sudo` or an elevated Windows Administrator terminal; do not auto-elevate or silently fall back. Root/elevated Administrator defaults to system scope without the scope prompt; safety confirmations still apply. Pin the installing user's resolved absolute config/state paths even for system services. Confirm retargeting another user's existing service and preserve both configs. Missing access fails with guidance.

Users may edit managed configuration, and authorized administrators may restart a system service with sudo to apply changes. Do not make config immutable merely to block authorized edits, and do not silently change ownership, permissions or runtime account. A privileged service reading user-editable configuration remains a security boundary against unauthorized writers: file and parent-directory ownership, replacement, symlink and TOCTOU handling, and service-account permissions need an approved threat model before implementing that privileged path. Windows user-background mechanism also remains open.

Installers for shell and PowerShell default to all three binaries and system-wide installation, with confirmation before PATH changes or conflicting replacements. Publish separate packages and all-in-one, Serve and Access images. Update each executable independently; package managers own their package updates and containers update by image redeploy. Standalone updates verify checksum and keyless signature pinned to CI issuer/identity; a deliberate unsafe bypass warns. Failed verification without bypass retains the old binary. Never automatically stop a running tunnel; guide the operator. Exact checksum-bypass behavior, rollback, trust rotation and Windows replacement remain open.

## Implementation phases and gates

Phases describe dependencies, not dates or approval of a particular release slice. The user deferred UDP design, benchmarking and implementation until after independent non-UDP work; UDP remains in scope and remains a full-redesign release gate. Do not add artificial dependencies between UDP and unrelated work. Unresolved decisions block only the work that depends on them.

| Phase | Deliverable | Entry and exit gates |
|---|---|---|
| 0. Contract and evidence | Review capability rules and data model; characterize 0.4 interoperability; benchmark UDP and investigate OS/security constraints. | Exit when decisions needed by each subsequent slice are approved; record direct/relay results, threat model and platform constraints. |
| 1. Shared foundation | DDD boundaries, common configuration/identity, existing protocol compatibility and role runtime seams. | Config mutation/migration, secret handling, ownership locking and stale-state policies must be specified before their implementations; relevant CFG, TC and RO criteria pass. |
| 2. Binaries and TCP | Three independent entrypoints, CLI, always-on multiplexed TCP. | CLI contract reviewed; independent process E2E and concurrent TCP criteria pass; both 0.4 compatibility directions are tested and failures presented for user decision; retry/shutdown policy agreed where implemented. |
| 3. UDP | Approved per-flow mapping plus selected Iroh transport, bounded resources, lifecycle and diagnostics. | Benchmark direct/relay; decide payload/fragmentation, expiry/reconnect, source filtering, tuple reuse and exhaustion policies; pass UDP E2E including adverse cases. |
| 4. Services and installation | OS adapters, service lifecycle and installers. | Privileged trust policy and account/path handling approved; Windows user-background mechanism chosen; real-platform privilege, retarget and install tests pass. |
| 5. Updates and release | Independently updated artifacts, separate packages, three images, signing and CI. | Verify package ownership, identity pinning, checksum/bypass behavior, rollback/trust rotation, Windows replacement and cross-platform release E2E. |

A phase may be partitioned into smaller changes once its relevant rules and safety gates are settled. Do not treat the proposed story-map cut as an approved release scope.

## Acceptance and verification matrix

Criteria IDs refer to the [capability specifications](redesign-spec.md); proposed criteria require approval before becoming release contracts.

| Coverage | Trace | Evidence required |
|---|---|---|
| Compatible peers and role binaries | AC-TC-01–03, AC-CLI-01–04 | Exercise all three binaries independently; test TCP with 0.4 in both directions and present failures for user decision; help/defaults match. |
| Config, identity and migration | AC-CFG-01–04; proposed AC-CFG-05–07 | Run new/legacy config, failed copy, secrets and concurrent mutation cases; verify identity is preserved. |
| Runtime ownership | AC-RO-01–03; proposed AC-RO-04–06 | Concurrent `it`/role binary starts, distinct roles, crash/stale status and ownership cleanup using approved policy. |
| TCP | AC-TCP-01–03; proposed AC-TCP-04–06 | Real clients share a connection, transfer independently; exercise target and peer failures, no silent replay. |
| UDP | AC-UDP-01–04; proposed AC-UDP-05–06 | Concurrent clients and interleaved replies, exact payload/boundary checks, empty and oversized datagrams, expiry/reconnect, late replies and resource exhaustion. Document remaining tuple-reuse risk. |
| Network paths and scale | AC-UDP-04 | Measure direct and forced relay with selected path verified, loss/congestion, varying packet sizes and flow counts; report latency, loss, throughput and resource use without invented targets. |
| Service operations | AC-SM-01–04; proposed service scenarios | Exercise user/system scope, privilege failures, pinned paths, retarget confirmation, trust policy, start/stop/status and actual Windows user-background behavior. |
| Distribution and update | AC-DIST-01–05 and proposed update scenarios | Install/update each binary, package and image ownership, signature/checksum failures and bypass, running process and platform-specific replacement; preserve old binary on failed verification absent explicit bypass. |
| Platform release | Cross-capability gates | Preserve released platforms; execute actual Windows x64 and ARM64 checks where runners exist, disclose unavailable coverage, validate installers and services end-to-end. |

Unit tests alone do not establish end-to-end behavior. If a platform or network path is unavailable, report the missing coverage and closest alternative; do not claim it passed.

## Design artifacts

- [Detailed design entrypoint](detailed-design.html): coverage, data models, flow and planned-work views.
- [Executable architecture delta](executable-delta.html): target versus current topology.
- [Scenario sequences](scenario-sequences.html) and [operational decisions](operational-decisions.html).
- [Dependency graph, UML and story map](structural-views.html): UML and release slices are proposals.
- [Target architecture dossier](target-diagrams.html) and [capability logic](capability-logic.html).
- [UDP transport research](../research/udp-transport.md) and [wayfinding map](https://github.com/naicoi92/iroh-tunnel/issues/69).

Diagrams depict target design unless explicitly labelled current. Automated geometry and accessibility checks do not establish business correctness. Full perceptual visual review of final diagrams has not been completed.

## Issue #81 quick TCP CLI slice

The all-in-one binary accepts `serve -n NAME -t IP:PORT` and
`access -n NAME -l IP:PORT -N NODE_ID`. Long forms are `--name`,
`--target`, `--listen`, and `--node-id`; `-p` / `--protocol` accepts `tcp`
(default), and `--relay-url` selects a relay shared by both roles. Socket
addresses are numeric IPv4 or IPv6 addresses with ports. Quick roles generate
a fresh ephemeral identity and do not write configuration or keys. Configured
`run` remains separate. UDP quick forwarding is not implemented here.

Issue #81 / TC-01 and stories 6, 10, 14, 15 and 18 map to
`tests/quick_run.rs` real-process TCP round trips: one uses long options and
omits protocol, another uses all role-relevant short options and explicit TCP.
Story 12 maps to independent Serve and Access restarts and config absence;
invalid-input acceptance maps to rejection by each role without config/key
writes. Tests use a local relay and actual TCP client and target. The existing
`test-utils` feature disables TLS verification for the entire test binary:
use it only with isolated local fixtures, no secrets or public relays, and do
not distribute that binary. Production builds must omit `test-utils`.

Issue #82 remains responsible for consistent short arguments and help/defaults
across all three independently runnable binaries. This slice does not establish
0.4 interoperability, which remains issue #85, or TCP lifecycle and multiplex
acceptance, which remain issue #86.
