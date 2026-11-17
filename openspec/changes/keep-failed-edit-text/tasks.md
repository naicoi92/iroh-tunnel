## 1. Keep failed edit text in the sidecar

Blocked by: none

Files: `crates/host/src/config/edit.rs`, `crates/host/src/config/types.rs`, `tests/config_lifecycle.rs` (single writer; Unix real-binary tests, Windows covered by host unit tests).

- [ ] 1.1 Write failing real-binary tests: lock held past the timeout keeps the edit in `serve.toml.rejected` (exit 2, mode 0600, no scratch left) — scenario "Lock held during commit" / "Commit error keeps the text in the sidecar"
- [ ] 1.2 Write a failing real-binary test: config directory made read-only during the edit leaves a named file holding the edit; skip as root; restore the mode with a drop guard — scenario "Config directory not writable"
- [ ] 1.3 Keep the test that saved and unchanged edits leave no scratch copy — scenario "Saved or unchanged edit leaves no scratch copy"
- [ ] 1.4 Write a failing host unit test: commit error with an unwritable sidecar keeps the scratch copy and names it; and an invalid edit with an unwritable sidecar does the same — scenario "Sidecar unavailable keeps the scratch copy"
- [ ] 1.4b Write host unit tests: an unreadable scratch copy is kept and named (regression guard), and a scratch copy removed by the editor is reported gone without "still in" (failing first) — scenario "Unreadable scratch copy is kept"
- [ ] 1.5 Implement: `commit` writes the text to the sidecar on any error after reading it, keeps the scratch copy only when the sidecar write fails, says "still in" only when the file exists, and reminds the user to delete kept files; the dropped-attempt unit test still passes — scenario "Dropped attempt"
- [ ] 1.6 Run cargo fmt, clippy `-D warnings`, full `cargo test --all-features --locked`
