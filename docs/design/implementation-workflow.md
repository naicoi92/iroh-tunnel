# Redesign implementation workflow

Each implementation issue owns one branch, worktree and pull request targeting `main`. Refresh the issue, comments and native blockers before pickup. Start from a `main` commit containing the relevant tracked specification and acceptance records; unresolved decisions block only work that depends on them.

## Skill sources

The workflow follows Matt Pocock's [implement](https://github.com/mattpocock/skills/blob/4588b32ecab9ecc9fc8cc6b6c5e7d675b6004b0d/skills/engineering/implement/SKILL.md), [TDD](https://github.com/mattpocock/skills/blob/4588b32ecab9ecc9fc8cc6b6c5e7d675b6004b0d/skills/engineering/tdd/SKILL.md), [code review](https://github.com/mattpocock/skills/blob/4588b32ecab9ecc9fc8cc6b6c5e7d675b6004b0d/skills/engineering/code-review/SKILL.md), and [PR](https://github.com/mattpocock/skills/blob/4588b32ecab9ecc9fc8cc6b6c5e7d675b6004b0d/skills/engineering/pr/SKILL.md) skills. Read their `SKILL.md` files and referenced guidance before the corresponding stage; install or load the skill package in the agent harness as appropriate. The steps and template below remain available to contributors without that harness.

## Implement and review

1. Read [the specification](redesign-spec.md), [traceability](redesign-traceability.md), [acceptance procedures](redesign-acceptance.md), the issue and relevant decisions. Record the acceptance criteria and agreed real-process or platform test seam.
2. Follow Matt Pocock's `implement` and `tdd` skills: one observable behavior per red → green slice, then the appropriate full test suite. Preserve actual red/green evidence and disclose unavailable environments.
3. Run the `code-review` skill against a pinned base and head with independent **Standards** and **Spec** reviews. Address valid findings and rerun affected tests. Apply CodeRabbit review and disposition its findings without replacing either independent axis.
4. Open the task's pull request against `main` using the `pr` skill. Include the template below. Require relevant CI and review on the exact final head before merge. Revalidate affected combined behavior after merge.

## Pull request evidence template

```markdown
## Summary

<Smallest diagram, tree or diff showing the change and issue reference.>

## Evidence

- **Before:** <Observed failing behavior or red test, command and result.>
- **After:** <Exact head, commands, results, acceptance scenarios and platform.>
- <Review findings and disposition; unavailable environments and test limits.>

## Merge Danger

**Door:** <one-way or two-way, with reason>

**Blast Radius:** <affected surface>
```

An individual task merge is not full-redesign acceptance. Final release gate #97 requires final-artifact evidence across all required capabilities, including UDP and supported platforms.
