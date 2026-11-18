# Apply Receipt

> Generated at the end of the apply phase to mark code-implementation
> complete and provide verify with the state it needs.
> Overwritten on each apply iteration; iteration counter grows.

**Change**: `config-relay-flags`
**Iteration**: `1`
**Applied at**: `2026-11-18 12:19` (UTC)
**Executor**: `subagent-driven-development` (implementer agent per task; `code-reviewer` agent as task reviewer and final whole-branch reviewer)

---

## Workspace

- **Worktree path**: `/Users/naicoi/Projects/iroh/iroh-tunnel.config-relay-flags`
- **Worktree branch**: `config-relay-flags`
- **Feature branch**: `config-set-relay-flag`
- **Base branch**: `main`

---

## Commits

- **Range**: `690fe4c..96e9865` (plus this receipt commit)
- **Count**: `14`

---

## Tasks

- **Completed**: `19 of 19` checkboxes in tasks.md flipped to `- [x]`
- **Remaining**: none

---

## Contract deviations

- `ServiceArgs::SetExtra` (public): the bound gained `Send + 'static`, because `src/app.rs` moves `ConfigAction` into `tokio::task::spawn_blocking`. User-approved during apply.
- `<role> config node` / `--relay-token` (public): accepts hyphen-prefixed values (`allow_hyphen_values`). Without it, clap echoed a token starting with `-` in its unexpected-argument error. A value equal to a `config node` flag name (`-c`, `--config`, `-r`, `--relay-url`, `--relay-token-file`, `--clear-relays`) is rejected with the usage error `--relay-token is missing its value`, which does not echo the value. User-approved during apply. The final review then added `-h` and `--help` to the rejected list, because `--relay-token --help` stored `--help` as the token.

---

## Lanes

| Task | Lane branch | Lane range | Review | Merge commit |
|---|---|---|---|---|
| Task 1 | `sequential` | `690fe4c..0419378` | approved after 1 fix round (NodePlan Debug leak) | n/a (sequential) |
| Task 2 | `sequential` | `1bc24df..3fce77d` | approved after 1 fix round ([node] preservation test) | n/a (sequential) |
| Task 3 | `sequential` | `a3a669a..204c733` | approved after 1 fix round (hyphen token echo) | n/a (sequential) |
| Task 4 | `sequential` | `91d4745..a76777f` | approved after 1 fix round (scenario THEN assertions) | n/a (sequential) |
| Final review | `sequential` | `6e52078..96e9865` | with fixes → re-review approved (I1–I3, M3–M5 fixed; M1, M2 parked) | n/a |

Parked from the final review:
- **M1:** clearing an explicit `relay_urls = []` renders as `(default) -> (default)` / `(node) -> (node)`. The spec defines only `[a, b]`, `(default)` and `(node)`, so changing this needs a spec decision. The file itself is still edited correctly.
- **M2:** `render::node_list` duplicates the host-internal `relay_list::display_list`. Reusing it would mean widening the host public API for a 5-line helper.

Environment notes:
- `tests/homebrew_casks` fails locally because the system Ruby is 2.6 (it needs ≥ 3.1). This change does not touch that file.
- Clippy reports pre-existing unused imports in `crates/host/src/service/{install,verb}.rs`; they are out of scope.
- The `config node` case added to `tests/config_lifecycle_windows.rs` was not run, because no Windows toolchain is available.

---

## Next step

Run `/opsx-verify`.
