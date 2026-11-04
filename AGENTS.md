# Working in this repository

## Orientation

- Domain vocabulary: [GLOSSARY.md](GLOSSARY.md).
- Architecture and design: [docs/design/](docs/design/), starting with `redesign-spec.md` and `redesign-plan.md`. Confirmed decisions, proposals and deferred gates are labelled as such; the issue tracker supersedes older document snapshots.
- Rust changes and reviews: [Rust coding standards](docs/agents/rust-coding-standards.md); cite rule IDs.

## Workflow

Follow the [implementation workflow](docs/design/implementation-workflow.md), beginning with its Pickup step.

- One issue, one branch, one worktree based on `main`, one pull request targeting `main`.
- Record acceptance criteria and map them to end-to-end tests of real behavior before coding; then work test-first.
- Review independently for Standards and for Spec.
- Merge after relevant CI and acceptance pass; rerun affected integration tests afterwards.
- Parallel work needs exclusive file ownership and independent blockers.
- If a required document or decision is missing from the baseline, stop and resolve it before dependent work.
