# Issue #92 evidence — Windows roles in the user background

Mechanism (SM-04, Confirmed in #79): a Task Scheduler task for the current user (logon trigger, interactive token, `LeastPrivilege`), managed with `schtasks /Create /XML`, `/Run`, `/End`, `/Query`, `/Delete`, restart through the shared `RestartPolicy`, status through the existing status snapshot. `--system` on Windows stays unsupported.

## Acceptance map

Lifecycle tested at head `009bbfd`, run https://github.com/naicoi92/iroh-tunnel/actions/runs/41219495680 (re-run green on every later head) (windows-latest x64 and windows-11-arm ARM64, both green). `user_background_task_lifecycle` and `start_without_an_installed_task…` are opt-in (`IROH_TUNNEL_E2E_SERVICE=1`, set by that workflow) and return early otherwise, so a green ordinary `cargo test` does not exercise them. The lifecycle asserts a new PID after `start` and `restart`, that the old process is gone, `status` reporting running/not running from the role lock, and the same node id.

| Criterion | Test | Result |
|---|---|---|
| Real install, startup, status, stop on Windows | `tests/service_user_windows.rs::user_background_task_lifecycle` (installs, starts, queries, stops, starts, restarts, uninstalls; pinned absolute config; CWD `%SystemRoot%`) | pass on windows-latest (x64) and windows-11-arm (ARM64), workflow `windows-service-e2e` |
| `--system` unsupported, nothing registered | `system_scope_is_unsupported_on_windows` | pass x64 + ARM64 |
| Verbs on a missing task name the install command | `start_without_an_installed_task_names_the_install_command` | pass x64 + ARM64 |
| A state directory the task cannot carry is refused | `install_refuses_a_state_directory_the_task_cannot_carry` | pass x64 + ARM64 |
| Task definition (logon trigger, interactive token, no elevation, restart policy, escaping) | `service::task_xml` unit tests | pass on macOS host; compiled and linted for `x86_64-pc-windows-msvc` |
| Role ownership (RO-03) on a second start | lifecycle test: `service start` of a running role exits 6 | pass |

## Measurements (SM-04 "first measurements in #92")

- **`schtasks /End` is a hard termination, with no drain.** The tunnel process disappears in 0.17 s (ARM64) and 1.2 s (x64) and the status file is left behind (`graceful drain: false`). Recorded as a known limitation per SM-04/RO-03; no stop command or control channel was added. The stale snapshot is handled by the existing ownership liveness check (RO-03/AC-RO-05).
- **`/End` kills only the task's own process.** A first revision wrapped the binary in `cmd.exe /c set IROH_TUNNEL_STATE_DIR=… && …`; on x64 `/End` ended `cmd.exe` and left `iroh-tunnel.exe` running orphaned (the test timed out). The task now runs the binary directly, and a non-default `IROH_TUNNEL_STATE_DIR` is refused at install with exit 2, because a task cannot carry an environment variable.
- **Runner sessions:** interactive session 2, `UserInteractive=True`, x64 `AMD64` and `ARM64`.

## Not verified (disclosed limits)

- **Non-admin create/query/delete: MEASURED, with limits.** Step "Standard user create/query/delete" of `windows-service-e2e` (run https://github.com/naicoi92/iroh-tunnel/actions/runs/41222175846, x64 and ARM64) creates a local user in `Users` only, launches the real binary as it with `Start-Process -Credential` (`isAdmin=False`, not in Administrators), and runs `service install --user`, `status`, `uninstall`: exit 0 each, `schtasks /Query` afterwards fails as expected. No password prompt for the task (`InteractiveToken`, `LeastPrivilege`). Limits: the user was launched non-interactively, so the logon trigger and an actual run in that user's desktop session were not exercised (`status` correctly said not running); Group Policy that forbids task creation for standard users is not covered; the profile environment was set by the workflow.
- **Bug found by that probe and fixed:** the task was bound to `USERNAME`/`USERDOMAIN` from the inherited environment; a credentialed launch inherits the parent's variables, which named another account. The user is now read from the process token via `whoami`.
- **Console window vs Ctrl events not measured.** The task runs the console binary; whether it shows a window in a real desktop logon, and whether hiding it would change Ctrl-event delivery, needs a physical logon session. Since `/End` does not send a Ctrl event, a hidden window has no known drain cost today.
- **Logon trigger across a real logoff/logon cycle** is not exercised; runners are not rebooted. `schtasks /Run` after install is what is tested.
- **Restart-on-failure** is rendered (`RestartOnFailure`, 1 minute minimum interval, since Task Scheduler rejects a shorter interval) but a crash-and-restart on Windows is not exercised.
- **Architectures and OS not covered:** only GitHub-hosted Windows runner images (x64 `windows-latest`, ARM64 `windows-11-arm`). 32-bit x86, Windows Server and older Windows 10 are untested.
- **Permissions on config/state:** the pinned paths are exercised; ACLs are not checked (deferred with Windows system scope in SM-04). The test uses the runner's real default state directory because a task cannot carry `IROH_TUNNEL_STATE_DIR`.
- `install` replaces an existing task of the same name without asking (`/F`); same user, so no privilege boundary, but SM-02-style retarget confirmation is not applied.
- Task-registered detection reads the CSV task list, and access-denied classification reads the English message; on a localised Windows the latter degrades to exit 5 instead of 3.
- Windows system scope (Windows Service, DACL/reparse rules) is deferred in SM-04.
