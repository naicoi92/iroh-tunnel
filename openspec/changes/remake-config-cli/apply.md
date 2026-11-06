# Apply Receipt

> Generated at the end of the apply phase to mark code-implementation
> complete and provide verify with the state it needs.
> Overwritten on each apply iteration; iteration counter grows.

**Change**: `remake-config-cli`
**Iteration**: `1`
**Applied at**: `2026-11-06 19:09`
**Executor**: `subagent-driven-development` (implementer `superspec.implementer`, per-task and whole-branch review `superspec.task-reviewer`)

---

## Workspace

- **Worktree path**: `/Users/naicoi/Projects/iroh/iroh-tunnel.remake-config-cli`
- **Worktree branch**: `remake-config-cli`
- **Feature branch**: `remake-config-cli`. The worktree was created by Worktrunk before planning, so the branch checked out at step 0 is the worktree branch itself. There is no separate feature branch: finalize merges into `main` through a PR.
- **Base branch**: `main` (merge-base `9b8f301`)

---

## Commits

- **Range**: `af6c312..a872fbe`. `af6c312` is the step-0 artifact scaffold commit. The implementation starts at `bae2fea`.
- **Count**: `25`, which includes task-checkbox commits.

---

## Tasks

- **Completed**: `24 of 24` checkboxes in tasks.md flipped to `- [x]`
- **Remaining**: none

---

## Verification evidence (Task 11)

**Quality checks:** all passed.

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | ok |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | ok, 0 warnings |
| `cargo test --workspace --all-features` | 586 passed, 0 failed, 4 ignored |

**Flaky tests.** Two tests flaked once each and passed on every rerun:
- `udp_tunnel::one_peer_cannot_take_the_slots_of_another`, under full-workspace load.
- `configured_multiplex::access_reports_peer_stop_as_tunnel_error`, a network flake.

Both are pre-existing and are not touched by this change.

**Windows:** `cargo check --tests --target x86_64-pc-windows-msvc` cannot run on this Mac. The cross C toolchain fails on `ring` under plain cargo and on `aws-lc-sys` under `cargo xwin`. The Windows test files (`tests/config_lifecycle_windows.rs`) are compile-checked by CI only.

**Smoke test** with the real binary and an isolated `HOME`. All outputs match plan.md Contracts.

| Command | Result |
|---|---|
| `iroh-tunnel serve` | help, exit 2 |
| `serve config set -n web` | `error: creating service 'web' requires --target` and the `config set` help, exit 2 |
| `serve config set -n web -t 3000` | `Created service 'web' in <p>.` |
| `serve config set -n web -t 3001 -p udp` | `Updated service 'web' in <p>:` with `  target: 127.0.0.1:3000 -> 127.0.0.1:3001` and `  protocol: tcp -> udp` |
| the same command again | `Service 'web' unchanged (<p>).` |
| `serve config check` | `OK: <p> is a valid serve config.` |
| `serve config rm -n web` | `Removed service 'web' from <p>.` |
| `serve config keygen --print -c x.toml` | clap conflict and the keygen help, exit 2 |
| quick `serve -n w -t localhost:<echo>` with `access -n w -l <p> -N <id>` | a byte round-trip through the tunnel passed |

**Gate:** `rg -n "config add|config remove|keygen random" README.md docs examples src tests crates` returns no output.

---

## Review summary

- **Per-task reviews:** all 12 tasks were approved.
  - Task 2 needed one fix round to add a test for a successful node_id update.
  - Task 12 needed one fix round on doc wording: the bare-PORT host default on update, and the CHANGELOG enum and help lines.
- **Final whole-branch review** (af6c312..a872fbe) — verdict: Ready with fixes.
  - No Critical findings and no code findings.
  - Important: Windows CI must be green before merge.
  - Important: the two pre-existing flaky tests should be noted in the PR.

**Deferred minors.** These are accepted by the final review. They are style, test-strength or unreachable-path notes:
- Task 2: tests for access create, multi-field order, adding a host key, and a protocol-only change; serve `node_id` behaviour that the CLI cannot reach; a duplicated shape message.
- Task 3: style.
- Task 4: the looser `rejected()` helper; the hostname character set; `match` on a bool.
- Task 5: duplicated Serve/Access dispatch arms.
- Task 6 and Task 9: substring and prefix assertions in help tests.
- Task 7: ancestor non-global args in `takes_value`; extra parser tests.
- Task 8: an mtime check on error paths.
- Task 10: an implicit quick-to-stored link; a hostname test that runs on v4 only when `::1` cannot be bound.

---

## Rulings made during apply

- **Test command:** verification runs `cargo test --workspace --all-features`, not plain `--workspace`. CI uses `--all-features`, and some tests need feature `test-utils`. Cost if wrong: none, because it is a superset.
- **No final fix wave:** the Windows compile check is left to CI, because a local msvc cross-check is impossible. Cost if wrong: one follow-up commit if CI finds a `cfg(windows)` error.

---

## Contract deviations

All are internal; no public contract changed.

- `find_service_index` (host, internal): it takes `(doc, name, path)` so the shape error can name the file. `find_service` was removed because it became dead after `add_service` was deleted.
- `ServiceArgs::ROLE` (CLI, internal): typed as `iroh_tunnel_host::Role`, aliased `StatusRole` in `cli.rs`.
- `run()` dispatch (CLI, internal): split into separate Serve and Access arms, because `RoleCmd<S>` now differs per role. The generic `#[derive(Subcommand)]` worked, so the macro fallback was not needed.
- Example text (CLI, internal): attached per binary inside `build_command` (`it` versus `its`/`ita`) instead of in derive attributes.

---

## Next step

Run `/opsx-verify`. Before merge, make sure Windows CI (`tests/config_lifecycle_windows.rs`) is green.
