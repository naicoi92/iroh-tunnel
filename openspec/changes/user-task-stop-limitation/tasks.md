## 1. Document the Windows user-tier stop limitation

Blocked by: none

- [x] 1.1 In `docs/services.md` (Windows, User tier), replace "`stop` ends the task" with: `stop` ends the role without the graceful drain (Task Scheduler terminates it), open connections are cut, the status snapshot may stay behind and `<role> status` reports it stale, and the next start is not blocked (scenario "Documented alternative")
- [x] 1.2 In the same section, point to the system tier for a graceful stop on Windows, linking the System tier section (scenario "Documented alternative")

## 2. Align the Windows lifecycle test with the specified stop

Blocked by: none

- [ ] 2.1 In `tests/service_user_windows.rs`, replace the module comment's "SM-04 records terminate-without-drain as a known limitation" with a reference to the requirement "Windows user-tier stop terminates without draining"; keep the drain outcome reported, not asserted
- [ ] 2.2 In `user_background_task_lifecycle`, when the snapshot remains after stop, assert that `serve status` (user tier) warns the snapshot is stale on stderr and still prints it (scenario "Leftover snapshot reported stale")
- [ ] 2.3 Stop removing the leftover snapshot by hand before `service start`; wait for a snapshot whose pid differs from the stopped one, so the start is proven with leftovers in place (scenarios "Next start is not blocked", "Windows logon task lifecycle")
- [ ] 2.4 Run `cargo fmt --check` and, on a Windows host or the `lint (windows)` CI job, `cargo clippy --all-targets --all-features --locked -- -D warnings`; record that the Windows E2E (`IROH_TUNNEL_E2E_SERVICE=1`) runs only on the `windows` e2e job
