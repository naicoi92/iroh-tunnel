# Retrospective: windows-system-service

## Wins
- The real-SCM E2E on the elevated runner (23 tests) found five product defects that unit tests had missed:
  - `is_elevated` and deny-only Administrators;
  - an unparseable pin, so every reinstall looked foreign;
  - a binary refusal that did not name the binary;
  - the config read before the trust check;
  - no log on the first start.
- The Task 1 spike measurements held. No design assumption was contradicted.

## Misses
- Several E2E failures were fixture artefacts, each costing one CI round:
  - TEMP on `D:` owned by NETWORK SERVICE;
  - `target\debug` on `PATH`;
  - icacls materialising inherited ACEs;
  - an over-asserted exit code.
- One review fix was needed for the CI wiring: a multi-line pwsh step masked a failing cargo run.

## Plan deviations
- See apply.md "Contract deviations". The public ones are:
  - the elevated-only `install.ps1` protection (owner decision);
  - the `update` hint naming both roles;
  - `is_elevated` adding a membership check;
  - the Description `# ` prefix.
- The new internal helpers `require_present` and `ensure_machine_dir` came from the final review.

## Skill/workflow compliance
- Subagent-driven development ran through Task 13B. After an implementer run aborted, and at the owner's request, the controller implemented the rest; every part was still reviewed by a fresh seat and proved on Windows CI.

## Surprises
- A plain `read_to_string` existence check in `run` opened the user's config with service rights before the trust check. This predates the branch on Unix as well.

## Promote candidates
- For Windows E2E fixtures, use `%LOCALAPPDATA%`, never TEMP, and pin `PATH`.
- Split each cargo invocation in a pwsh CI step into its own step.
