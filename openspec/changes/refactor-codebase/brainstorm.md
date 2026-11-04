## Design Summary

Refactor iroh-tunnel into a small, responsibility-oriented Cargo workspace that is easier to maintain and provides reusable operations for future TUI and Tauri service-management applications. Apply idiomatic Rust and pragmatic DDD to existing behavior rather than rewriting the product or imposing a framework.

The future interfaces will manage independently running Serve and Access services. They may explicitly start or stop a service; closing an interface must not stop that service. This establishes the architectural direction, not a requirement to build either interface in this change. The existing foreground CLI modes remain supported.

The current application-oriented library exposes implementation and testing seams alongside command-line concerns. Examples include clap argument types accepted by configuration/update operations, terminal output inside role startup, TOML models with persistence responsibilities, and runtime observations coupled to status-file schemas. These are stronger reasons for refactoring than file size or a flat source directory alone.

### Goals

- Establish clear responsibility and dependency boundaries between domain decisions, use-case orchestration, tunnel execution, platform effects, and presentation.
- Make configuration, service-management, and status operations reusable without requiring a frontend to construct clap arguments, answer terminal prompts, or parse human-readable output.
- Organize source, tests, documentation, and packaging assets around their owners, avoiding an unstructured collection of top-level implementation files.
- Preserve useful existing domain concepts, including Flow, Session incarnation, Quarantine, Flow cap, and Datagram limit, using the repository glossary rather than inventing competing terminology.
- Address evidenced Rust design and lifecycle weaknesses in the affected code, including secret-bearing debug output, overly broad public APIs, and unclear task ownership.
- Keep every migration step verifiable against existing observable contracts; do not replace the implementation in one unreviewable rewrite.

### Compatibility and Safety

The Rust API may change, including module paths and signatures. Such changes need an explicit migration map for existing callers and tests. Permission to change the Rust API does not permit silent changes to operator-facing or peer-facing behavior.

Preserve the three executable names, command surface, aliases, foreground modes, documented output and exit classification, configuration paths and TOML compatibility, status-file locations and JSON schemas, installer entrypoints, and release artifact identities. Preserve ALPN/wire behavior and serve-first compatibility.

Preserve privilege checks, platform service support and limitations, trusted configuration loading, role/identity ownership, atomic and locked persistence, secret hygiene, forwarding semantics, resource bounds, and shutdown behavior. Future service-management callers must receive actionable outcomes without bypassing these policies. Do not weaken production guarantees to reuse testing seams.

The current contracts remain the compatibility baseline, including documented inferred requirements and UDP deviations. Exposing a conflict does not authorize choosing new behavior. A necessary behavior change must be identified and approved explicitly before implementation; unrelated fixes are not bundled into this refactor.

### Non-Goals and Anti-Goals

- No TUI, Tauri application, embedded-tunnel product API, new daemon, or new control protocol is implemented here.
- No new forwarding features, protocol negotiation, authorization system, or silent correction of documented UDP deviations.
- No automatic tightening of accepted inputs merely because validated types are introduced.
- No speculative repository framework, CQRS, event bus, generic transport framework, or trait for every concrete dependency.
- No assumption that each OpenSpec capability or each Serve/Access role is a separate bounded context.
- No global replacement of the repository's established error strategy, MSRV, edition, or quality gates merely to match generic skill advice.
- No cosmetic relocation that breaks Cargo test discovery, root installer URLs, CI, packaging, or release workflows. A file may remain at the root when it is an external entrypoint or belongs there by convention.

## Alternatives Considered

### Alternative A: Modular Monolith

- **Approach**: Retain one Cargo package and reorganize cohesive modules, visibility, use cases, and adapters within it.
- **Pros**: Smallest build and packaging migration; clear improvements are possible without cross-crate API work.
- **Cons**: Cargo dependencies remain shared, and architectural boundaries rely more on visibility and review than package-level enforcement.
- **Why not chosen**: The user wants a broader structural foundation for several frontends and selected responsibility-based workspace boundaries rather than retaining the current package shape.

### Alternative B: Small Responsibility-Oriented Workspace

- **Approach**: Separate responsibilities with genuine dependency boundaries into a small workspace, while keeping cohesive modules together and preserving the existing product.
- **Pros**: More explicit ownership, narrower reusable APIs, and enforceable package dependencies; source and tests can live with the responsibility they verify.
- **Cons**: Requires coordinated Cargo, API, test, CI, documentation, and packaging migration; poorly chosen splits could create excess public surface or unnecessary indirection.
- **Selection rationale**: Chosen by the user. It directly addresses repository organization and multi-frontend reuse in the same change, with incremental implementation inside that change.

### Alternative C: Two Separate Refactor Changes

- **Approach**: Establish module boundaries in one package now, then perform workspace extraction in a later change.
- **Pros**: Smaller scope per change and a chance to validate boundaries before package extraction.
- **Cons**: Defers the desired workspace organization and may require repeated migration of paths, APIs, tests, and documentation.
- **Why not chosen**: The user selected one responsibility-oriented workspace effort rather than two separate migration projects. This does not preclude small, independently verified implementation steps.

## Agreed Approach

Proceed with Alternative B. Improve boundaries first in substance, not just directory names, and migrate incrementally to a small workspace. Retain and adapt the existing implementation and tests wherever they already uphold the product's contracts.

The service-management application is conceptually separate from the tunnel processes it manages. Reusable operations must not depend on the lifecycle or presentation of a particular UI. The future UI's closure is not a service stop request. Management in this change covers the existing install, uninstall, start, stop, restart, and status responsibilities for operating-system service registrations, together with existing configuration and role-status operations. It preserves the current systemd, BusyBox/SysV, launchd, and Windows Task Scheduler support and scope limitations. Foreground CLI execution remains available, but adopting or controlling arbitrary foreground processes is not a management feature introduced here. This change does not invent a remote-management product or a new background-process protocol.

Use DDD to identify the decisions and invariants that belong together, and Rust's ownership, types, visibility, and dependency direction to make those boundaries practical. Preserve the existing UDP decision model, safe persistence, trusted service loading, and supervised drain behavior rather than replacing them for stylistic uniformity.

The exact crate count, names, dependency graph, API signatures, error/result representations, state lifetimes, observation mechanisms, and file-move sequence belong in `design.md` and subsequent artifacts. This document intentionally does not choose them.

## Key Decisions

### Confirmed by the User

1. Future TUI and Tauri applications manage background Serve/Access services. Closing the interface leaves those services running; stopping is an explicit management operation.
2. Rust API compatibility may be broken with a documented migration, while existing product and security contracts remain protected.
3. A small responsibility-oriented workspace is the selected direction. This is a scoped exception to the current project context's prohibition on extra crates, not permission for arbitrary crate proliferation. The proposal must state and justify the exception; no configuration change is implied by writing this artifact.

### Proposed Engineering Guardrails

These calls support the selected direction and remain subject to this document's review:

- Evaluate boundaries by independent responsibility, invariants, and actual callers, not by equal file sizes or a predetermined architectural diagram.
- Treat GUI/TUI service management as the reuse target; do not design a general embedded runtime solely for hypothetical callers.
- Make the migration account for all existing runtime, configuration, management, update, observation, and distribution responsibilities. A cohesive existing area may remain largely unchanged when no defect justifies deeper refactoring.
- Preserve current Linux, macOS, and Windows behavior, including unsupported operations and scope limitations; do not silently substitute a different privilege level or service scope.
- Derive regression coverage from the existing specifications and real CLI/network/service flows. Keep local-relay process tests alongside focused domain tests, and report unavailable platform or artifact coverage honestly.
- Retain MSRV Rust 1.91 and the repository's existing validation gates. Do not enable test-only TLS overrides in production or use those binaries against public relays.
- Record Rust API migration, repository path changes, and affected CI/release/test entrypoints before the corresponding implementation steps. No external dependency additions are authorized by the workspace choice.

## Open Questions

No product-level decision remains unanswered for this brainstorming artifact. The following are design work, not decisions already made by the user:

- Which responsibilities justify separate crates, and which remain cohesive modules within them?
- What narrow reusable management inputs and outcomes preserve existing service scope, privilege, ownership, and status semantics without exposing terminal presentation?
- Which process-global lifecycle policies must remain at executable boundaries, and which state belongs to individual operations or running roles?
- How will the migration preserve test discovery, package/build entrypoints, release identities, and compatibility throughout the transition?

Any design discovery that requires a new service-control protocol, broader platform support, new dependencies, or a change to an observable compatibility contract must return for an explicit scope decision rather than being assumed from this artifact.
