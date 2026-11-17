# Keep Failed Edit Text Implementation Plan

> Executed via `/opsx-apply`, which chooses the executor.
> Spec reference: this change's `specs/` and `design.md`.

**Goal:** A failed `config edit` commit never loses the edited text: it lands in `<config>.rejected`, or stays in the named scratch copy when the sidecar cannot be written.

**Architecture:** `EditAttempt::commit` reads the scratch copy, runs a fallible `apply`, and on error routes the text through the existing `keep_rejected` before wrapping the error with the recovery path. A `keep_scratch` flag on `EditAttempt` stops `Drop` from removing the scratch copy only when the text lives nowhere else.

**Tech Stack:** Rust 2021, anyhow, tempfile, real-binary tests in `tests/config_lifecycle.rs`.

---

## Contracts

| Name | new / modified | Shape | public |
|---|---|---|---|
| `EditAttempt` field `keep_scratch` | new | `pub(super) keep_scratch: bool` | no |
| `EditAttempt::commit` error text | modified | `Err` context: `your edit was not saved; your text is in <sidecar>; delete it once recovered, it may contain secrets` or `...; it is still in <scratch>; ...` | no (message only; signature `fn commit(self) -> Result<EditOutcome>` unchanged) |
| `keep_rejected` | modified | `fn keep_rejected(path: &Path, scratch: &Path, edited: &str) -> (Option<PathBuf>, String)` | no |

Contracts are relative to `main` (`82d7d92`); HEAD `3f5b3b3` already carries a partial version.

## Scenario coverage

| Scenario | Test |
|---|---|
| Dropped attempt | `edit.rs::dropped_edit_attempt_removes_scratch` (existing) |
| Commit error keeps the text in the sidecar | `config_lifecycle.rs::edit_kept_when_the_writer_lock_times_out` |
| Sidecar unavailable keeps the scratch copy | `edit.rs::commit_error_keeps_scratch_when_sidecar_cannot_be_written` (new), `edit.rs::rejected_edit_keeps_scratch_when_sidecar_cannot_be_written` (existing, gains reminder assertion) |
| Unreadable scratch copy is kept | `edit.rs::unreadable_scratch_is_kept_and_named`, `edit.rs::removed_scratch_is_reported_gone` |
| Lock held during commit | `config_lifecycle.rs::edit_kept_when_the_writer_lock_times_out` |
| Config directory not writable | `config_lifecycle.rs::edit_kept_when_writing_the_config_fails` |
| Saved or unchanged edit leaves no scratch copy | `config_lifecycle.rs::saved_and_unchanged_edits_leave_no_scratch` |

## Task 1 — implements 1

**Covers:** 1.1, 1.2, 1.3, 1.4, 1.4b, 1.5, 1.6
**Difficulty:** Normal
**Blocked by:** none
**Files:** modify `crates/host/src/config/edit.rs`, `crates/host/src/config/types.rs`, `tests/config_lifecycle.rs`

- [ ] **Step 1 (red):** In `tests/config_lifecycle.rs`, change `edit_kept_when_the_writer_lock_times_out` to assert exit 2, stderr names `serve.toml.rejected` and contains `delete it once recovered`, the sidecar holds `9090` with mode 0600, and `edit_scratches(&world)` is empty.
- [ ] **Step 2 (guard):** In `edit_kept_when_writing_the_config_fails`, return early when running as root (`libc::geteuid() == 0`, as in `tests/service_user.rs`), and restore `0o755` through a drop guard created before the command runs. Keep the assertion "stderr names an existing file containing `9090`"; in a read-only directory the sidecar write also fails, so that file is the scratch copy.
- [ ] **Step 3 (red):** In `edit.rs` tests add `commit_error_keeps_scratch_when_sidecar_cannot_be_written`: prepare, write text different from the original to the scratch copy, replace the live file with a directory (so `read_optional` fails under the lock) and put a directory at `serve.toml.rejected`. Expect `Err` whose `{:#}` names the scratch copy, says the text could not be kept in `serve.toml.rejected`, and contains `delete it once recovered`; the scratch copy still holds the text. Add the reminder assertion to `rejected_edit_keeps_scratch_when_sidecar_cannot_be_written` and to `edit_commit_rejects_invalid_text_and_names_sidecar`.
- [ ] **Step 3b:** Add `unreadable_scratch_is_kept_and_named` (Unix, skip as root; regression guard): chmod the scratch copy 000, commit, expect `Err` naming it and the file still present. Add `removed_scratch_is_reported_gone` (red): remove the scratch after prepare, commit, expect `Err` that does not contain `still in`.
- [ ] **Step 4:** Run `cargo test --all-features --locked --test config_lifecycle edit_` and `cargo test -p iroh-tunnel-host --all-features --locked config::edit`. Expected red: Step 1, Step 3 and `removed_scratch_is_reported_gone`; the rest stay green.
- [ ] **Step 5 (green):** Implement in `edit.rs`: `commit` reads the scratch copy (on failure: if it exists, keep and name it; otherwise report it gone), calls `apply(&edited)`, and on `Err` calls `keep_rejected` then adds context naming the sidecar, or the kept scratch copy when the sidecar failed; set `keep_scratch` when the sidecar write failed (for `Err` and for `Rejected { sidecar: None }`) and restrict a kept scratch copy to 0600 on Unix. `keep_rejected` messages carry the delete-once-recovered reminder.
- [ ] **Step 6:** Run the targeted tests until green, then `cargo fmt`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, `RUBY=/opt/homebrew/Library/Homebrew/vendor/portable-ruby/current/bin/ruby cargo test --all-features --locked`.
- [ ] **Step 7:** Commit `fix(config): keep failed edit text in the rejected sidecar`.
