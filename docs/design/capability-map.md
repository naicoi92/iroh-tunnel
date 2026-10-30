# Redesign capability map

**Status: approved by the user.** This is a map of independently testable capabilities, not a crate layout, implementation plan, or approval of unresolved business rules. Dependencies describe capability contracts, not necessarily build-time Rust dependencies. The [interview decisions](project-redesign.md) remain the source for confirmed requirements.

## Capabilities

| ID | Responsibility | Depends on |
|---|---|---|
| `tunnel-contract` | Define service names, peer identity references, protocol compatibility and shared tunnel rules. | — |
| `configuration` | Role config, identity lifecycle, validation, overrides, legacy migration and secret handling. | `tunnel-contract` |
| `runtime-ownership` | Role ownership, process lifecycle, status snapshots and cleanup. | `tunnel-contract` |
| `tcp-forwarding` | Serve and Access TCP forwarding, connection reuse and per-client stream isolation. | `tunnel-contract`, `runtime-ownership` |
| `udp-forwarding` | Fixed-target UDP forwarding, concurrent client flows and best-effort response routing. | `tunnel-contract`, `runtime-ownership` |
| `service-management` | Install and manage user/system background services with pinned paths, scope and privilege checks. | `configuration`, `runtime-ownership` |
| `distribution` | Independent binaries, packages and images; signed release, installers and update ownership. | — |
| `cli-experience` | Consistent standalone/all-in-one entrypoints, commands, help and short arguments. | `configuration`, `runtime-ownership`, `tcp-forwarding`, `udp-forwarding`, `service-management`, `distribution` |

A configured role composes `configuration` with the selected forwarding capability at the application boundary; quick execution can provide equivalent validated inputs without persisting a config. Shared domain types and exact crate boundaries remain design decisions.

## Suggested design order

1. `tunnel-contract` and `distribution` can be specified independently.
2. `configuration` and `runtime-ownership` establish identity, state and process boundaries.
3. `tcp-forwarding` and `udp-forwarding` specify protocol-specific behavior; UDP transport selection remains gated on benchmarks.
4. `service-management` composes the config and runtime contracts.
5. `cli-experience` presents the agreed capability contracts consistently.

This is an order for reviewing contracts, not a claim that implementation must proceed strictly serially.

## Diagram scope after map review

| View | Question |
|---|---|
| Architecture delta | Which current components and behaviors change in the target, and which are retained? |
| Logical ER / data model | What persisted config/identity and runtime process/connection/channel/flow/status concepts exist, and who owns them? No database is assumed. |
| Flowcharts | How do config resolution and migration, ownership, service scope/privileges, installation and update verification branch on success and failure? |
| Sequences | How do TCP, multi-client UDP, configured startup, service installation and update interact across actors? |
| State machines | How do role and connection lifecycles progress? Retry and shutdown details remain proposed until decided. |
| Kanban | Which design and implementation gates remain? Cards are planned work, not evidence of completion. |

Each capability specification should trace confirmed rules and acceptance criteria to diagram views, and mark proposed or deferred decisions explicitly. In particular, UDP transport/fragmentation/limits, Windows user-background, runtime locking, peer retry/shutdown, release rollback/trust rotation, and privileged service access to user-editable config remain unresolved.
