# Redesign implementation

Before implementing or dispatching a redesign ticket, start from a commit containing `GLOSSARY.md` and `docs/design/`. Read `docs/design/redesign-spec.md`, `docs/design/redesign-plan.md`, the relevant decision records and the current GitHub issue, comments, and native blockers. Live tracker decisions supersede older document snapshots; distinguish confirmed decisions from proposals and deferred gates.

Each implementation worktree must contain the required tracked documentation at its baseline. If a required document is absent or a decision is unresolved, stop before implementation. Do not rely on untracked files in another checkout. Record acceptance criteria and map them to end-to-end tests of real functionality before coding.
