## Design Summary

Issue #164: the `refactor-codebase` change (archived 2026-11-05) left living specs that only make sense during that refactor, and Purpose placeholders that make `openspec validate --specs` warn. This change makes every requirement in `workspace-boundaries` and `frontend-independent-management` checkable against the current tree alone, and gives every living spec a real Purpose.

Intent: a reader of `openspec/specs/` should be able to verify each requirement without the archived change directory, a pre-change snapshot or a memory of how the code used to behave.

Goals:
- `openspec validate --specs` reports no Purpose warning. 14 of 16 specs currently have `TBD`, not only the two named in the issue.
- No living requirement in the two specs depends on files under `openspec/changes/`, a one-off baseline, or "before the refactor" behaviour.
- The rewritten dependency rule is enforced by a test.

Non-goals:
- No behaviour change to the product, CLI, config or status formats.
- Making every crate declare its dependencies through `workspace.dependencies`. The host and root crates declare more than 20 crates directly today, and moving them is a separate refactor.
- Rewording other specs beyond their Purpose, and fixing the long-requirement warnings `openspec validate` also reports.

Anti-goals: rewording that loses a concrete, testable guarantee, such as the unknown-field, legacy `multiplex` and hostname-target acceptance cases.

Existing requirements this change alters (all in `openspec/specs/workspace-boundaries/spec.md` unless noted):
- "No new external dependencies": modified.
- "No test is lost by relocation": removed.
- "Rust API migration map": removed.
- "Observable contracts are unchanged": removed.
- "Validated types do not tighten acceptance": modified.
- "Run-scoped drain context": modified to remove the `DRAIN_DEADLINE` history and the "as before the refactor" scenario.
- "Configuration documents carry no filesystem behaviour": modified to remove "as before" and fix an ambiguous `Source:`.
- "Validation gates cover every member" and "Public rustdoc links resolve": only their ambiguous `Source:` lines change.
- `frontend-independent-management`: "Key rotation requires explicit consent", "Insufficient information yields a structured need", "Registered and updated executables are supplied explicitly" and "CLI keeps native passthrough and ordering". Each has one scenario saying "as before (the refactor)", which this change rewrites.

The CLI behaviour those four scenarios point at is already specified elsewhere:
- `config-management-commands` "Key rotation needs confirmation".
- `system-service-management` scope prompt requirements.
- `role-ownership-shutdown` "Repeated stop signal forces exit after the grace" and "Stuck shutdown watchdog".
- `tunnel-configuration` "Legacy multiplex field is ignored and dropped on save".

## Alternatives Considered

### Alternative A: Minimum fix for the issue text
- **Approach**: Write Purpose for the two specs, drop the three named requirements, leave the rest.
- **Pros**: Smallest diff.
- **Cons**: `openspec validate --specs` still has 12 Purpose warnings, so the issue's acceptance is not met. "As before the refactor" scenarios and the "eleven existing specs" requirement stay unverifiable.
- **Why not chosen**: It does not meet the acceptance criteria and leaves the same class of problem in place.

### Alternative B: Durable rules for everything, including a workspace-wide dependency rule
- **Approach**: As C, but replace "No new external dependencies" with "every member declares dependencies through `workspace.dependencies`", and migrate the manifests.
- **Pros**: Uniform dependency management.
- **Cons**: Touches every manifest and turns a spec cleanup into a build refactor, with no product benefit asked for.
- **Why not chosen**: Out of scope; the user chose the narrow, already-true rule.

### Alternative C: Clean the two specs fully, Purpose for all 14, narrow durable rule plus test (chosen)
- **Approach**: Remove or rewrite every refactor-relative requirement and scenario in the two specs, write Purpose for all 14 placeholder specs, and add one test for the narrow engine dependency rule.
- **Pros**: Meets both acceptance criteria. Every kept requirement stays concrete. The new rule is enforced. The only product-tree changes are one test and one CONTRIBUTING paragraph.
- **Cons**: Edits 14 main specs directly (Purpose cannot travel in a delta), so the review packet is larger.

## Agreed Approach

Alternative C, decided in the explore and brainstorm rounds of thread #164.

## Key Decisions

1. Purpose is written for all 14 specs with `TBD`, in this change.
2. "No new external dependencies" becomes a narrow, durable rule: every dependency of `iroh-tunnel-engine` (normal, build and dev, including target-specific tables) is inherited from the root `workspace.dependencies`. This already holds today.
3. A test enforces decision 2.
4. "No test is lost by relocation" and "Rust API migration map" are removed. `CONTRIBUTING.md` keeps its layout description and drops the paragraph that points at the archived migration map.
5. "Observable contracts are unchanged" is removed; the 14 capability specs carry those behaviours themselves.
6. "Validated types do not tighten acceptance" becomes "Validated types accept every input the capability specs accept", keeping the concrete scenarios: unknown fields, legacy `multiplex` keys and hostname targets load, and invalid input fails at load, not earlier.
7. Every "as before", "unchanged … as before the refactor", "replacing `DRAIN_DEADLINE`" and "pre-change" wording in the two specs is restated as a present-tense behaviour, or as a reference to the requirement that specifies it.
8. `Source:` lines stay as provenance. Only ambiguous ones ("this change's design.md", "refactor design") in the two specs are rewritten to name the archived change explicitly.

## Open Questions

None.
