## Why

The `refactor-codebase` change left `workspace-boundaries` and `frontend-independent-management` with requirements that only hold relative to a pre-refactor snapshot or to files now in the change archive. 14 of 16 living specs still have a `TBD` Purpose, so `openspec validate --specs` warns. Issue #164 asks for durable specs that can be checked against the current tree alone.

## What Changes

**Engine dependency rule**
- From: "No new external dependencies", checked against a pre-change `Cargo.lock`.
- To: new requirement "Engine dependencies are inherited from the workspace": every dependency of `iroh-tunnel-engine` is inherited from the root `workspace.dependencies`, enforced by a test in `tests/workspace_layout.rs`.
- Reason: the old check needs a baseline that no longer exists; the new rule already holds and stays checkable.
- Impact: non-breaking; the engine manifest already complies.

**Refactor-only requirements removed**
- "No test is lost by relocation", "Rust API migration map" and "Observable contracts are unchanged" are removed from `workspace-boundaries`. The capability specs already specify the behaviours that "Observable contracts" protected.
- The `CONTRIBUTING.md` paragraph that links the archived migration map is removed; the layout description stays.

**Validated types**
- From: "Validated types do not tighten acceptance" (relative to previously accepted input).
- To: new requirement "Validated types accept every specified input", with the same concrete inputs: unknown fields, legacy `multiplex` keys and hostname targets.
- Impact: non-breaking.

**Present-tense wording**
- Requirement text and scenarios that measure against a baseline are restated as present behaviour, or as references to the capability requirement that specifies it. This covers "as before (the refactor)", "pre-change", "replacing `DRAIN_DEADLINE`", "existing …" and "remain unchanged".
- Ambiguous `Source:` lines ("this change's design.md", "refactor design") name the archived change explicitly.

**Purpose**
- A real `## Purpose` is written directly in the main spec of all 14 capabilities that have `TBD`. A delta cannot carry a Purpose for an existing capability.

Non-goals: no product behaviour change; no migration of host or root dependencies into `workspace.dependencies`; no fix for the long-requirement warnings.

## Capabilities

### New Capabilities

None.

### Modified Capabilities
- `workspace-boundaries`:
  - added: "Engine dependencies are inherited from the workspace", "Validated types accept every specified input";
  - modified: "Run-scoped drain context", "Runtime tasks have explicit owners", "Quarantine sinks are owned for the role lifetime", "Status file removal requires writer quiescence", "Status publishing ordering and change detection are preserved", "Secret-bearing values redact their Debug output", "Exit classification survives package boundaries", "Validation gates cover every member", "Binary and version identities are preserved", "Distribution entrypoints follow moved paths", "Public internal APIs are justified and documented", "Configuration documents carry no filesystem behaviour", "Public rustdoc links resolve";
  - removed: "No new external dependencies", "Validated types do not tighten acceptance", "No test is lost by relocation", "Rust API migration map", "Observable contracts are unchanged";
  - Purpose is written.
- `frontend-independent-management`: requirements that compare with "before the refactor" or with "existing" behaviour are restated as present behaviour or as references to the owning capability:
  - "Key rotation requires explicit consent";
  - "Insufficient information yields a structured need";
  - "Registered and updated executables are supplied explicitly";
  - "CLI keeps native passthrough and ordering";
  - "Service operations enforce existing policy";
  - "Optional status facts do not cause failure";
  - "Raw status JSON stays byte-identical";
  - "Updater is a host operation with explicit inputs".

  Purpose is written.

The other 12 capabilities with a `TBD` Purpose get a Purpose only; none of their requirements change: `cli-entrypoints`, `config-management-commands`, `connection-path-status`, `install-release-packaging`, `iroh-endpoint-relay`, `role-ownership-shutdown`, `self-update`, `system-service-management`, `tcp-tunnel-forwarding`, `tunnel-configuration`, `udp-datagram-forwarding` and `windows-service-trust`.

## Impact

- `openspec/specs/*/spec.md`: 14 Purpose sections; requirement edits in two specs through deltas.
- `tests/workspace_layout.rs`: one new test. No production code changes.
- `CONTRIBUTING.md`: one paragraph removed.
- Compatibility: no CLI, config, status, wire or Rust API change. Security: none.
