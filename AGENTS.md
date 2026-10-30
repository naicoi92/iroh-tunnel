# Redesign implementation

Before implementing or dispatching a redesign ticket, start from a commit containing `GLOSSARY.md` and `docs/design/`. Read `docs/design/redesign-spec.md`, `docs/design/redesign-plan.md`, the relevant decision records and the current GitHub issue, comments, and native blockers. Live tracker decisions supersede older document snapshots; distinguish confirmed decisions from proposals and deferred gates.

For Rust changes and Standards reviews, read [Rust coding standards](docs/agents/rust-coding-standards.md) and cite applicable rule IDs.

Each implementation worktree must contain the required tracked documentation at its baseline. If a required document is absent or a decision required by the task is unresolved, stop before dependent implementation. Do not rely on untracked files in another checkout. Record acceptance criteria and map them to end-to-end tests of real functionality before coding.

Each implementation issue has its own branch and worktree based on `main`, and its own pull request targeting `main`. Follow the [implementation workflow](docs/design/implementation-workflow.md), starting with its Pickup step before proposing or starting an issue, then TDD, independent Standards and Spec reviews, and the PR evidence template. External bot reviews are temporarily suspended by the repository owner; retain independent Standards and Spec reviews. Merge only after relevant CI and acceptance; rerun affected combined integration tests after merging. Parallel tasks require exclusive file ownership and independent blockers. Do not treat individual PR evidence as full redesign release acceptance.
