## 1. Model a not-running owner as known and absent

Blocked by: none

Owner: one writer for `crates/host/src/service/types.rs` and `crates/host/src/service/status.rs`. Platforms: the owner mapping is shared by every backend; the Unix status tests run on Linux and macOS, and the Windows build is covered by CI compiling the same code path.

- [x] 1.1 Write a failing host unit test in `crates/host/src/service/status.rs` for scenario "Role not running has a known absent owner": no role claim, `run_status(vec![])` reports liveness `Known(NotRunning)` and owner `Known(None)`
- [x] 1.2 Change `ServiceStatusReport.owner` in `crates/host/src/service/types.rs` to `Fact<Option<OwnerRecord>>` and update its rustdoc to state the not-running and unreadable meanings
- [x] 1.3 Update the owner mapping in `status_via` (`crates/host/src/service/status.rs`): `NotRunning` maps to `Known(None)`, `Running { owner: Some }` maps to `Known(Some(..))`, `Running { owner: None }` and probe errors stay `Unavailable`
- [x] 1.4 Keep `status_reports_missing_owner_as_unavailable` passing (scenario "Owner metadata unavailable") and make 1.1 pass
- [x] 1.5 Run `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features --locked -- -D warnings` and `cargo test --all-features --locked` and `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked`; confirm `tests/cli_golden.rs` passes unchanged (CLI output unchanged)
