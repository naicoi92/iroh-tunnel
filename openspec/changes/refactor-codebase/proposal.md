## Why

The current single-package layout mixes tunnel execution, configuration persistence, operating-system service management, and CLI presentation. Broad public modules and test seams are not a safe, reusable management boundary. Refactoring now establishes cohesive Rust/DDD boundaries and a navigable repository before TUI and Tauri applications need the same operations, without duplicating policy or tying background-service lifetime to a UI process.

## What Changes

- **Responsibility-oriented workspace:** Replace the single package with a small Cargo workspace whose crate/module boundaries reflect cohesive responsibilities and dependency direction, not one crate per architectural layer, role, or OpenSpec capability. Organize source and tests with their owners, and coordinate documentation, CI, and packaging path changes. Exact crate names, counts, and dependency graph belong in `design.md`.
- **Scoped policy exception:** The approved brainstorm explicitly permits the justified workspace split as an exception to the project context's “no extra crates” constraint. This permits internal workspace packages, not speculative abstractions or new external dependencies. No configuration edit is made by this proposal.
- **Frontend-independent management:** Extract reusable configuration, service-lifecycle, and role-status operations from clap arguments, terminal prompts, and human-readable rendering. Callers supply decisions explicitly and receive actionable data or errors; the CLI retains its existing interaction and output. Separate the existing updater workflow from CLI argument types without broadening its behavior.
- **Independent background services:** Reuse existing install, uninstall, start, stop, restart, and status responsibilities through systemd, BusyBox/SysV, launchd, and Windows Task Scheduler, preserving platform and scope limitations. Releasing a management caller must not stop a registered service. Foreground CLI execution remains supported but is not newly adopted into managed-process control.
- **Domain and lifecycle boundaries:** Separate domain decisions and validated runtime models from persistence formats, transport/platform effects, and presentation. Keep existing glossary concepts and working implementation. Resolve evidenced secret/debug, visibility, and task-ownership weaknesses in affected code while preserving persistence, forwarding, resource, and shutdown contracts.
- **BREAKING — Rust API:** Module paths, visibility, and signatures may change. Document old-to-new API and repository-path mappings and migrate callers/tests; do not retain test-oriented ownership bypasses as production management guarantees.

**Compatibility:** Keep executable names and aliases, CLI commands and documented output/exit codes, config paths and TOML acceptance, status locations and JSON schemas, service scope/privilege/trust policies, installer entrypoints, release identities, and ALPN/wire/serve-first compatibility. Do not tighten accepted inputs as a side effect of newtypes or silently fix known UDP deviations. Necessary observable behavior changes require explicit approval before implementation.

**Non-goals:** No TUI/Tauri implementation, embedded-runtime product API, new daemon/control protocol, arbitrary foreground-process management, forwarding feature, authorization scheme, speculative repository/CQRS/event framework, MSRV/edition change, blanket error-strategy replacement, or new lint gate. This is an incremental refactor, not a replacement implementation.

## Capabilities

### New Capabilities

- `workspace-boundaries`: Enforce cohesive package/module ownership and dependency boundaries between domain decisions, orchestration, effects, and presentation; define the workspace's build/test discovery and migration compatibility obligations.
- `frontend-independent-management`: Expose existing configuration, operating-system service lifecycle, and role-status operations through a non-terminal Rust boundary with explicit caller decisions, actionable outcomes, existing security/ownership policies, and background-service lifetime independent of the caller.

Each new capability requires a delta spec at `specs/<capability>/spec.md`. These are architectural and Rust-caller contracts, not new end-user commands or a remote management API.

### Modified Capabilities

None. Existing requirements are preserved; implementation and source-location changes alone do not modify their behavior. In particular, moving clap-independent operations behind the new management boundary does not remove the CLI's prompts, tables, raw JSON behavior, or backend-native service diagnostics.

## Impact

- **Code and APIs:** Reorganize the current `src/` responsibilities, including role startup/execution, configuration/store/commands, ownership/shutdown, endpoint/forwarding/UDP, status/path reporting, service backends/trust, updater, and the three CLI entrypoints. Narrow public exports and document Rust API migration. Cohesive areas need not be rewritten merely because their paths move.
- **Existing capability regression surface:** Preserve all eleven capabilities: `cli-entrypoints`, `config-management-commands`, `connection-path-status`, `install-release-packaging`, `iroh-endpoint-relay`, `role-ownership-shutdown`, `self-update`, `system-service-management`, `tcp-tunnel-forwarding`, `tunnel-configuration`, and `udp-datagram-forwarding`. They remain verification authorities, not eleven required crate boundaries or delta specs.
- **Build and distribution:** Update Cargo manifests/lockfile integration, binary/test discovery, build metadata, CI, Docker/packaging inputs, documentation, and test fixtures together where paths or package selection change. Preserve externally referenced installer URLs and release artifact names; do not lose tests through relocation.
- **Security and lifecycle:** Redact secret-bearing debug representations, preserve actionable error classification and privileged trust checks, and make ownership/cancellation/join responsibilities explicit. Keep atomic/locked writes and production relay TLS verification; test-only overrides must remain confined to safe local-relay validation.
- **Validation:** Trace tests to preserved specs and the two new capability contracts. Retain focused domain tests and real-binary local-relay CLI/network/lifecycle coverage, verify workspace coverage and MSRV Rust 1.91, and report unavailable platform or release-artifact checks. Adapt existing validation commands only as needed to cover all workspace members; no new quality-gate policy is implied.
- **Dependencies and scope:** Keep the existing stack and supported platforms. Internal package extraction is authorized; any newly required external dependency, broader platform support, control protocol, or observable compatibility change returns for a separate explicit decision.
