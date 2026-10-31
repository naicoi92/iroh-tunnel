# Issue #79 — Prior art: Windows user-background, graceful stop, console window, trust model

Research date: 2026-10-31. Primary sources where fetched; items known only from search snippets or docs.rs crate pages are marked. Labels: **[Verified]** = stated explicitly by the source; **[Inference]** = researcher inference from the source; **[Unverified]** = the primary source was not fetched or read.

## 1. Summary

- Mechanisms for "user background without admin" used in practice on Windows include: Task Scheduler (Syncthing docs), Startup folder (Syncthing docs), `HKCU\...\Run` (the `auto-launch` crate; proposed in a Tailscale issue). **[Verified]** only for NSSM: the Syncthing docs say NSSM is run from an "administrator Command Prompt". That a Windows Service needs admin to install is **[Inference]** for cloudflared, WinSW, `sc.exe` and the `windows-service` crate (not stated in the sources read). Confidence: high only for the claims actually read (Syncthing autostart mechanisms, `auto-launch` HKCU/HKLM behaviour, NSSM needing an administrator prompt); low for the rest (see §6).
- Microsoft docs do **not** say that `schtasks /end` sends a Ctrl event. The `AllowHardTerminate` docs say Task Scheduler (the service) tries to close the task with `WM_CLOSE`, and only then calls `TerminateProcess` if `AllowHardTerminate=true`. A console process without a window does not receive `WM_CLOSE` → **[Inference]** it is most likely hard-terminated. Needs measurement on real Windows.
- Syncthing documents it itself: "ending the task in Task Scheduler terminates only the monitor process"; a real exit requires Web GUI Shutdown (REST API). This is a direct precedent for "stop via an application channel, not via an OS signal".
- `tokio::signal::windows` supports ctrl_c/ctrl_break/ctrl_close/ctrl_logoff/ctrl_shutdown, via `SetConsoleCtrlHandler` ⇒ meaningful only for processes with a console. `CTRL_LOGOFF/SHUTDOWN` reach services only (Microsoft Learn).
- Trust model: sshd `StrictModes` (default yes) and sudoers (rejects wrong owner/mode) are **fail-closed** precedents; `ChrootDirectory` is checked unconditionally. Windows OpenSSH requires an ACL of only Administrators+SYSTEM for `administrators_authorized_keys`.
- Most other projects in the list (rclone, restic, Docker Desktop, WireGuard, ngrok, Mullvad, mkcert, polkit, doas, nginx, Docker daemon) were **not verified** against a primary source in this pass.

## 2. Comparison of Windows user-background mechanisms

| Project | Mechanism | start/stop/status | Needs admin? | Graceful stop? | Source |
|---|---|---|---|---|---|
| Syncthing (docs) | Task Scheduler, trigger "At log on" (specific user); args `--no-console --no-browser`; "stay invisible in background" | Manual via taskschd.msc; docs have no CLI | Docs do not say; UI is "Administrative Tool". Choosing "Run whether user is logged on or not" requires entering a password | **No**: "ending the task terminates only the monitor process"; real exit via Web GUI Shutdown | [Syncthing autostart](https://docs.syncthing.net/users/autostart.html) |
| Syncthing (docs) | Startup folder `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup` + shortcut with `--no-console --no-browser` | No stop/status | No (per-user folder) **[Inference]** | n/a | as above |
| Syncthing (docs) | Service via NSSM (for headless servers); docs say this is not the preferred approach for end users | `sc start syncthing` | Yes ("administrator Command Prompt") | NSSM stop methods | as above |
| `auto-launch` crate | `HKCU\...\Run` (CurrentUser) or `HKLM\...\Run`; by default tries HKLM, falls back to HKCU if privileges are insufficient | enable/disable/is_enabled; no start/stop | HKCU: no; HKLM: yes | n/a (only registers autostart) | [docs.rs auto_launch](https://docs.rs/auto-launch/latest/auto_launch/) |
| `tauri-plugin-autostart` | Autostart wrapper (enable/disable/isEnabled); the docs page **does not state** the Windows mechanism | enable/is_enabled/disable | Not stated | n/a | [Tauri autostart](https://v2.tauri.app/plugin/autostart/) |
| `service-manager` crate (chipsenkbeil/service-manager-rs) | Windows: `sc.exe` (Windows Service) or WinSW; Linux systemd (with user level), launchd, OpenRC, rc.d. **No Task Scheduler/HKCU** in the list | install/uninstall/start/stop/status; `RestartPolicy`: WinSW fully supported, sc.exe limited/none | Service ⇒ needs admin **[Inference]** | Depends on sc.exe/WinSW | [docs.rs service_manager](https://docs.rs/service-manager/latest/service_manager/) |
| `windows-service` crate | API for writing a Windows Service (SCM control handler) | — | Service ⇒ admin **[Inference]** | Yes, via `SERVICE_CONTROL_STOP` **[Inference]** (docs.rs page fetched but not quoted specifically) | [docs.rs windows_service](https://docs.rs/windows-service/latest/windows_service/) |
| WinSW | Service wrapper (XML config) | — | Service ⇒ admin **[Inference]** | — (not quoted) | [WinSW XML config](https://github.com/winsw/winsw/blob/v3/docs/xml-config-file.md) |
| NSSM | Service wrapper. Stop order: Ctrl-C to console → `WM_CLOSE` to windows → `WM_QUIT` to threads → `TerminateProcess` (each step waits 1500 ms by default; `AppStopMethodSkip` exists) | `nssm install`, `sc start` | Yes | Yes, multiple steps | [NSSM usage](https://nssm.cc/usage) |
| cloudflared | System Windows Service (`cloudflared service install`), config in `%PROGRAMDATA%`/`%USERPROFILE%\.cloudflared`; failure recovery delay 20s, reset 24h; uses `svc.IsAnInteractiveSession()` to distinguish service/console runs | `service install/uninstall` | Service ⇒ admin **[Inference]** | Yes, via the Go svc handler (`graceShutdownC`) | [windows_service.go](https://github.com/cloudflare/cloudflared/blob/master/cmd/cloudflared/windows_service.go), [Cloudflare docs](https://developers.cloudflare.com/tunnel/features/locally-managed-tunnels/as-a-service/windows/) |
| Tailscale (Windows) | Has "unattended mode" that runs before login (service); GUI `tailscale-ipn.exe` runs at startup. Issue #7345 (Scoop) proposes `HKCU\...\Run` for `tailscale-ipn.exe` because the "traditional install depends on installer having admin privileges and adding to Startup items" | — | Standard install needs admin | Unverified | [run-unattended](https://tailscale.com/docs/how-to/run-unattended), [issue 7345](https://github.com/tailscale/tailscale/issues/7345) (search snippet only, not read in depth) |
| rclone, restic/autorestic, Docker Desktop, WireGuard, ngrok, Mullvad, mkcert-like | **Unverified** (primary source not fetched) | | | | |

Task Scheduler schema (primary, Microsoft Learn; **search snippet only**, page not fully fetched): `Hidden` only hides the task from the UI (it does not hide the process window) — [Hidden](https://learn.microsoft.com/en-us/windows/win32/taskschd/taskschedulerschema-hidden-settingstype-element); `RestartOnFailure` — [link](https://learn.microsoft.com/en-us/windows/win32/taskschd/taskschedulerschema-restartonfailure-settingstype-element); `RunLevel` 0 = lowest, 1 = elevated — [link](https://learn.microsoft.com/en-us/windows/win32/taskschd/taskschedulerschema-runlevel-principaltype-element). The LogonTrigger page was fetched but no information about InteractiveToken could be extracted: **unverified** in this pass.

## 3. Findings Q2 — Graceful stop

1. **IRunningTask::Stop** only says "Stops this instance of the task"; returns `E_ACCESSDENIED` if the user lacks permission to stop it, the task is disabled, or it is not allowed to run on demand. Says nothing about the mechanism (signal/terminate). [Verified] — [IRunningTask::Stop](https://learn.microsoft.com/en-us/windows/win32/api/taskschd/nf-taskschd-irunningtask-stop).
2. **schtasks /end**: "Stops only the instances of a program started by a scheduled task"; says nothing about signals. [Verified] — [schtasks end](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/schtasks-end).
3. **AllowHardTerminate**: "The service will try to close the running task by sending the WM_CLOSE notification, and if the task does not respond, the task will be terminated only if this property is set to true" (via `TerminateProcess`). [Verified] — [AllowHardTerminate](https://learn.microsoft.com/en-us/windows/win32/taskschd/tasksettings-allowhardterminate). The `TASK_TRIGGER_FLAG_KILL_AT_DURATION_END` docs also mention `WM_CLOSE` and then terminate if there is no window ([mstask](https://learn.microsoft.com/en-us/windows/win32/api/mstask/ns-mstask-task_trigger), snippet). **[Inference]**: a console process without a window ⇒ no `WM_CLOSE` target; Microsoft does not say whether `CTRL_C/CTRL_CLOSE` is sent ⇒ treat as **no reliable graceful signal**, needs measurement.
4. **Syncthing** confirms the real-world behaviour: ending the task only kills the monitor process; use Web GUI Shutdown. [Verified] — [Syncthing autostart](https://docs.syncthing.net/users/autostart.html).
5. **Console control events** (Microsoft Learn): a handler applies only to a *console process*; `CTRL_CLOSE_EVENT` is sent to every process attached to a console when the user closes the console window or uses End Task in Task Manager, timeout ~5000 ms (`SPI_GETHUNGAPPTIMEOUT`); `CTRL_C/BREAK` have no timeout; `CTRL_LOGOFF/SHUTDOWN` are "received only by services" (interactive apps are terminated first). [Verified] — [HandlerRoutine](https://learn.microsoft.com/en-us/windows/console/handlerroutine).
6. **tokio::signal::windows**: provides `ctrl_c`, `ctrl_break`, `ctrl_close`, `ctrl_logoff`, `ctrl_shutdown`, listening via `SetConsoleCtrlHandler`. [Verified] — [module](https://docs.rs/tokio/latest/tokio/signal/windows/index.html), [ctrl_close](https://docs.rs/tokio/latest/tokio/signal/windows/fn.ctrl_close.html). The docs page does not state the condition (console/session) — the condition comes from HandlerRoutine in item 5.
7. **NSSM** (service wrapper) sends Ctrl-C to the app's console first, then `WM_CLOSE`, `WM_QUIT`, and finally `TerminateProcess`; "GUI applications do not have consoles and will not respond to [Ctrl-C]". [Verified] — [NSSM usage](https://nssm.cc/usage).
8. **cloudflared** (Go) on Windows uses the Service control handler (`svc` package) and the `graceShutdownC` channel, not relying on Ctrl events when running as a service. [Inference from source] — [windows_service.go](https://github.com/cloudflare/cloudflared/blob/master/cmd/cloudflared/windows_service.go).
9. Tailscale, other Go/Rust daemons: **unverified**.

**Implication [Inference]**: for RO-03 (no new stop command), on Windows user-background there is no documented, guaranteed graceful stop path via `schtasks /end`; if the daemon already has an internal shutdown channel, that is what Syncthing uses; if not, accept hard-terminate and document the limitation. Needs measurement (§6).

## 4. Findings Q3 — Console window

- `CREATE_NO_WINDOW` (0x08000000): "process is a console application that is being run without a console window. Therefore, the console handle for the application is not set"; ignored when combined with `CREATE_NEW_CONSOLE` or `DETACHED_PROCESS`. [Verified] — [Process Creation Flags](https://learn.microsoft.com/en-us/windows/win32/procthread/process-creation-flags). `CREATE_NEW_PROCESS_GROUP` disables CTRL+C for the new group (same page).
- Rust: `std::os::windows::process::CommandExt::creation_flags(u32)` sets the flags. [Verified] — [CommandExt](https://doc.rust-lang.org/std/os/windows/process/trait.CommandExt.html).
- Syncthing: uses the application flag `--no-console` (+ `--no-browser`) when run from Task Scheduler/Startup; docs say "stay invisible in background". The internal mechanism (windows subsystem or self-detach) is **unverified**. [Syncthing autostart](https://docs.syncthing.net/users/autostart.html).
- Task `Hidden` only hides the task from the UI and does **not** hide the process window (see the Hidden snippet above). Avoiding a console window requires another approach (windows subsystem / launcher).
- `windows_subsystem = "windows"`: **unverified** from a primary source in this pass (Rust Reference not fetched).
- Consequence [Inference from HandlerRoutine + CREATE_NO_WINDOW]: the handler is a mechanism of a *console process*; with no console handle (CREATE_NO_WINDOW) or a GUI subsystem, the ability to receive Ctrl events is not documented as guaranteed → needs measurement on a real machine. NSSM docs confirm that GUI apps "do not have consoles and will not respond" to Ctrl-C.

## 5. Findings Q4 — Trust model

| Project | What is checked | Fail-closed? | Source |
|---|---|---|---|
| sshd `StrictModes` | Checks file mode and ownership of the user's files and home directory before login; default `yes`; `ChrootDirectory` is checked unconditionally | Yes (default); admin can disable (`no`) | [sshd_config(5)](https://man.openbsd.org/sshd_config.5) |
| sshd(8) authorized_keys | If the file, `~/.ssh` or home is "writable by other users", sshd refuses to use it unless `StrictModes no` | Yes | [sshd(8)](https://man.openbsd.org/sshd.8) |
| sudoers | Refuses/errors: "/etc/sudoers is world writable" (default mode 0440), wrong owner/gid. Timestamp dir: contents ignored if not owned by root or writable by someone other than root. `sudoedit` refuses to open symlinks unless `sudoedit_follow`/`FOLLOW`. `fdexec`: avoids TOCTOU by executing via file descriptor | Yes (refuses) | [sudoers manual](https://www.sudo.ws/docs/man/sudoers.man/) |
| OpenSSH on Windows | `administrators_authorized_keys`: ACL "only allows Administrators and SYSTEM"; instructions `icacls /inheritance:r /grant Administrators:F /grant SYSTEM:F` (use SID `*S-1-5-32-544` for other locales) | Docs describe the requirement; behaviour on a wrong ACL (refuse?) is not stated on this page → unverified | [Key-based auth OpenSSH Windows](https://learn.microsoft.com/en-us/windows-server/administration/openssh/openssh_keymanagement) |
| `open(2)` `O_NOFOLLOW` | Rejects symlinks only in the **final component**; symlinks in earlier components are still followed (ELOOP if the last is a symlink). This is a primitive, not a policy | n/a | [open(2)](https://man7.org/linux/man-pages/man2/open.2.html) |
| Windows `CreateFileW` | Has the `FILE_FLAG_OPEN_REPARSE_POINT` flag (opens the reparse point/symlink itself instead of the target) — the flag list appears on the page; detailed semantics not quoted | n/a | [CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew) |
| Windows `GetFinalPathNameByHandleW` | Returns the fully resolved "final path" (symlink `C:\tmp\mydir`→`D:\yourdir` ⇒ `D:\yourdir`); used to verify the real path after a handle is opened | n/a | [GetFinalPathNameByHandleW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getfinalpathnamebyhandlew) |
| systemd `ConfigurationDirectory=` | Creates the config directory; mode adjusted with `ConfigurationDirectoryMode=`; "adjusted to what is specified" if mismatched. Not a trust check of user-editable files | Adjusts mode, does not refuse | [systemd.exec](https://www.freedesktop.org/software/systemd/man/latest/systemd.exec.html) |
| cron `/etc/cron.d` (root-owned, not group/other-writable) | The fetched crontab(5) page **does not contain** the phrase "owned by"; this requirement is **unverified** from the sources read | Unverified | [crontab(5)](https://man7.org/linux/man-pages/man5/crontab.5.html) |
| polkit, doas, Docker daemon config, nginx, Tailscale/WireGuard config perms | **Unverified** (not fetched) | — | — |

Note [Inference]: the precedents confirm the pattern "check owner+mode on the opened fd, reject symlinks, fail closed" (sshd, sudoers), but sudoers/sshd both return an error or refuse, not merely warn. The docs read do not describe checking *every parent directory* beyond sshd mentioning the home directory and `~/.ssh`; the "every parent directory" requirement for Windows is a bespoke design with no directly verified precedent.

## 6. Gaps — need measurement on real Windows

1. `schtasks /end` / `IRunningTask::Stop` with a console process without a window: sends `WM_CLOSE`? `CTRL_C/CTRL_CLOSE`? or `TerminateProcess` immediately? Does it depend on `AllowHardTerminate`? The docs do not say.
2. For a process run via a task (InteractiveToken / LogonTrigger, `Hidden`), is any console/handler activated; does `tokio::signal::windows::ctrl_close/ctrl_c` receive anything on stop? Does it differ between `CREATE_NO_WINDOW`, `DETACHED_PROCESS`, `windows_subsystem="windows"`?
3. `RestartOnFailure` behaviour after termination by `/end` (does it restart? — docs say restart "if the task fails for any reason", not tested).
4. `LogonTrigger` + `Principal` `LogonType=InteractiveToken` + `RunLevel=LeastPrivilege`: install/delete/query without admin for the user's own task? Not verified against a primary source.
5. Rclone, restic/autorestic, Docker Desktop, WireGuard, ngrok, Mullvad, mkcert, Tailscale (graceful stop), polkit, doas, nginx, Docker daemon, cron.d perms: not checked.
6. Rust `windows_subsystem` and Ctrl event behaviour: primary source not checked.
7. OpenSSH Windows: behaviour on a wrong ACL (refuse or only warn) — needs `sshd`/`win32-openssh` source.
8. Detailed semantics of `FILE_FLAG_OPEN_REPARSE_POINT` + owner/DACL checks (`GetSecurityInfo`) — not quoted.
9. Caution: many GitHub repo/raw URLs (service-manager-rs source, auto-launch source) returned 404 in this pass; claims about crate internals rest on docs.rs only.
10. **[Unverified]** Whether a non-admin user can create, delete and query their own logon-triggered scheduled task using an interactive token and no password. This must be the first measurement in #92.

## 7. Source list

Kept:
- IRunningTask::Stop — https://learn.microsoft.com/en-us/windows/win32/api/taskschd/nf-taskschd-irunningtask-stop
- schtasks end — https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/schtasks-end
- TaskSettings.AllowHardTerminate — https://learn.microsoft.com/en-us/windows/win32/taskschd/tasksettings-allowhardterminate
- TASK_TRIGGER (mstask.h) — https://learn.microsoft.com/en-us/windows/win32/api/mstask/ns-mstask-task_trigger (snippet)
- Task Scheduler schema: Hidden / RestartOnFailure / RunLevel / Settings (snippet) — https://learn.microsoft.com/en-us/windows/win32/taskschd/taskschedulerschema-hidden-settingstype-element
- HandlerRoutine — https://learn.microsoft.com/en-us/windows/console/handlerroutine
- Process Creation Flags — https://learn.microsoft.com/en-us/windows/win32/procthread/process-creation-flags
- CreateFileW — https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew
- GetFinalPathNameByHandleW — https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getfinalpathnamebyhandlew
- OpenSSH key management (Windows) — https://learn.microsoft.com/en-us/windows-server/administration/openssh/openssh_keymanagement
- tokio::signal::windows — https://docs.rs/tokio/latest/tokio/signal/windows/index.html ; ctrl_close — https://docs.rs/tokio/latest/tokio/signal/windows/fn.ctrl_close.html
- std CommandExt — https://doc.rust-lang.org/std/os/windows/process/trait.CommandExt.html
- Syncthing autostart — https://docs.syncthing.net/users/autostart.html
- NSSM usage — https://nssm.cc/usage
- cloudflared windows_service.go — https://github.com/cloudflare/cloudflared/blob/master/cmd/cloudflared/windows_service.go ; Cloudflare docs — https://developers.cloudflare.com/tunnel/features/locally-managed-tunnels/as-a-service/windows/
- auto-launch docs — https://docs.rs/auto-launch/latest/auto_launch/
- service-manager docs — https://docs.rs/service-manager/latest/service_manager/
- windows-service docs — https://docs.rs/windows-service/latest/windows_service/
- tauri autostart — https://v2.tauri.app/plugin/autostart/
- WinSW XML config — https://github.com/winsw/winsw/blob/v3/docs/xml-config-file.md
- Tailscale run-unattended / issue 7345 — https://tailscale.com/docs/how-to/run-unattended , https://github.com/tailscale/tailscale/issues/7345 (snippet only)
- sshd_config(5), sshd(8) — https://man.openbsd.org/sshd_config.5 , https://man.openbsd.org/sshd.8
- sudoers manual — https://www.sudo.ws/docs/man/sudoers.man/
- open(2) — https://man7.org/linux/man-pages/man2/open.2.html
- systemd.exec — https://www.freedesktop.org/software/systemd/man/latest/systemd.exec.html
- crontab(5) — https://man7.org/linux/man-pages/man5/crontab.5.html (does not contain the claim being sought)

Rejected/deprioritized: StackOverflow / SuperUser on Task Scheduler stop (not primary; not used as evidence).
