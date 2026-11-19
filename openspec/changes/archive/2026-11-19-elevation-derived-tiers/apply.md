# Apply Receipt

> Generated at the end of the apply phase to mark code-implementation
> complete and provide verify with the state it needs.
> Overwritten on each apply iteration; iteration counter grows.

**Change**: `elevation-derived-tiers`
**Iteration**: `1`
**Applied at**: `2026-11-19 09:30`
**Executor**: `subagent-driven-development` (implementer role: `implementer`; task reviewer role: `code-reviewer`; parallel lanes for Tasks 2 and 3)

---

## Workspace

- **Worktree path**: `/Users/naicoi/Projects/iroh/iroh-tunnel.elevation-derived-tiers`
- **Worktree branch**: `elevation-derived-tiers`
- **Feature branch**: `spec/elevation-derived-tiers`
- **Base branch**: `main`

---

## Commits

- **Range**: `ced1765..3c32694` (plus this receipt commit)
- **Count**: `43`

---

## Tasks

- **Completed**: `33 of 33` checkboxes in tasks.md flipped to `- [x]`
- **Remaining**: none

---

## Contract deviations

- `config::store::ConfigLock::acquire`: the Windows confidentiality check runs only for elevated writes into the system-tier config directory, not for every elevated write. A narrower check was needed because elevated CI writes to temp dirs would otherwise exit 3. This matches the spec scope. Internal.
- `IROH_TUNNEL_TEST_TRUSTED_OWNER`: also honoured under `cfg(test)`, not only `test-utils`. The variant is named `TrustedOwners::TestOwner` because clippy rejects enum variant names that share a prefix. Internal.
- System state dir on macOS: `/Library/Application Support/iroh-tunnel/state` instead of `/var/run/iroh-tunnel`. `/private/var/run` is group-writable without a sticky bit, so it fails the trust walk. Locks stay in `/var/run/iroh-tunnel`. Public; the user decided this during Task 4 and it passed the review gate.
- `install` internals: `ensure_not_running_in`; the current-tier `wait_released` was removed in favour of `wait_released_in`; `resolve_config_path_for` is `pub(crate)`. Internal (Task 4).
- Lock files in the machine dir: the system claim moved to an admin-only `<role>.claim.lock`, held exclusively. `<role>.lock` (0644) is now held shared and used only for detection. This stops a non-admin from blocking the service (DoS). Public file format; the user decided this after the final review and it passed the gate.
- Upgrade transition: while an earlier-version holder still holds `<role>.lock` exclusively and has an admin-owned owner record with a live pid, a system claim exits 6, and probe and wait report it as running. Public; the user decided this after the re-review and it passed the gate.
- Windows always-elevated refusal: a user-tier Windows `install` returns exit 3 for the built-in Administrator, or for an Administrators member when `EnableLUA=0`. Found by Windows CI. Public; the user decided this and it passed the gate.

---

## Lanes

| Task | Lane branch | Lane range | Review | Merge commit |
|---|---|---|---|---|
| 1 | sequential | `ced1765..aad786d` | approved (2 fix rounds) | — |
| 2 | `elevation-derived-tiers--task-2` | `931271b..f92161d` | approved (2 fix rounds) | `23d3ab8` |
| 3 | `elevation-derived-tiers--task-3` | `931271b..1b08886` | approved (2 fix rounds) | `2c03e22` (post-merge clippy fix `5ff1ef3`) |
| 4 | sequential | `7a0851d..ca95427` | approved (2 fix rounds) | — |
| 5 | sequential | `7160b96..4a741c7` | approved (1 fix round) | — |
| 6 | sequential | `89e5fb4..48353d7` | approved (1 fix round) | — |
| final review fixes | sequential | `53f5eb6..3c32694` | final review "with fixes" → fix wave + upgrade fix + Windows CI fixes, each re-reviewed: approved | — |

---

## Verification matrix

| Suite | Where | Result |
|---|---|---|
| `cargo fmt --check`, clippy `-D warnings`, rustdoc `-D warnings` | macOS host | clean |
| `cargo test --workspace --all-features --locked` | macOS host | 1205 passed, 0 failed, 4 ignored |
| clippy for Windows (`cargo xwin`) | macOS cross | clean |
| Linux test build (`cargo zigbuild --tests`) | macOS cross | builds |
| `tests/service_system.rs` (BusyBox, real root) | Kiri container `rust:1.91-bookworm` | 22/22, run twice in a row |
| `tests/role_binaries.rs` | same container | 11 passed; `standalone_managed_install_starts_registered_role` skipped (needs systemd) |
| e2e workflow (linux, windows incl. Task Scheduler, Windows Service, elevated host unit tests; macos; action in container) | GitHub Actions run 44482405284 on `3c32694` | green |
| `ci.yml` (PR-only) | not triggered (no PR) | not run |
| macOS launchd system/user lifecycle | not run: the host has live LaunchAgents | not run |
| systemd system lifecycle | not run | not run |
| Windows user-task lifecycle tests | skipped on always-elevated runner (UAC off); refusal test ran | partial |

---

## Next step

Run `/opsx-verify`.

---

## Post-PR iteration 1

- **Trigger**: PR #260 `ci.yml` (first run; PR-only workflow) failed `lint` on linux/macos/windows under the pinned toolchain Rust 1.91. The 1.99 toolchain used locally during apply did not flag the two errors: an unfulfilled `dead_code` expectation in `service/trust_windows/mod.rs` (lib test build), and a duplicated `#[cfg(windows)]` in `service/install.rs`.
- **Fix**: `9bbb6d7 fix(host): satisfy clippy 1.91 lint expectations and drop duplicate cfg`. Checks passed on 1.91: fmt, clippy for macOS and Windows (xwin), rustdoc `-D warnings`, host lib tests 662/0, and the Linux zigbuild test build.
- **Commit range**: `ff2aeff..9bbb6d7`
- **Result**: PR #260 checks green (CI run 44500004016: lint and unit on linux, macos and windows; ci-ok).
- **Contract deviations**: none new.
- **Lesson**: run clippy with `cargo +1.91` (the MSRV toolchain CI pins), not only the local default toolchain.
