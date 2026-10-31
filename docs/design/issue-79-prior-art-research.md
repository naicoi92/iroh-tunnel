# Issue #79 — Prior art: Windows user-background, graceful stop, console window, trust model

Ngày nghiên cứu: 2026-10-31. Chỉ dùng primary sources. Nhãn: **[Trực tiếp]** = nguồn nói rõ; **[Diễn giải]** = suy ra từ nguồn; **[Không xác minh được]** = chưa fetch/đọc được nguồn chính thức.

## 1. Tóm tắt

- Cơ chế "user background không cần admin" trên Windows được dùng thực tế gồm: Task Scheduler (Syncthing docs), Startup folder (Syncthing docs), `HKCU\...\Run` (crate `auto-launch`; Tailscale issue đề xuất). Windows Service (`sc.exe`, WinSW, NSSM, cloudflared) đều cần admin để cài [Trực tiếp: Syncthing docs nói chạy NSSM từ "administrator Command Prompt"]. Độ chắc: cao cho Syncthing/auto-launch/NSSM/cloudflared; thấp cho phần còn lại (xem §6).
- Microsoft docs **không** nêu `schtasks /end` gửi Ctrl event. Tài liệu `AllowHardTerminate` nêu Task Scheduler (service) cố đóng task bằng `WM_CLOSE`, rồi mới `TerminateProcess` nếu `AllowHardTerminate=true`. Process console không có window thì không nhận `WM_CLOSE` → **[Diễn giải]** rất có thể bị terminate cứng. Cần đo trên Windows thật.
- Syncthing tự ghi nhận: "ending the task in Task Scheduler terminates only the monitor process"; muốn thoát thật phải dùng Web GUI Shutdown (REST API). Đây là tiền lệ trực tiếp cho "stop qua kênh ứng dụng, không qua tín hiệu OS".
- `tokio::signal::windows` hỗ trợ ctrl_c/ctrl_break/ctrl_close/ctrl_logoff/ctrl_shutdown, qua `SetConsoleCtrlHandler` ⇒ chỉ có nghĩa với process có console. `CTRL_LOGOFF/SHUTDOWN` chỉ tới service (Microsoft Learn).
- Trust model: sshd `StrictModes` (mặc định yes) và sudoers (từ chối khi sai owner/mode) là tiền lệ **fail-closed**; `ChrootDirectory` kiểm tra vô điều kiện. Windows OpenSSH yêu cầu ACL chỉ Administrators+SYSTEM cho `administrators_authorized_keys`.
- Phần lớn dự án khác trong danh sách (rclone, restic, Docker Desktop, WireGuard, ngrok, Mullvad, mkcert, polkit, doas, nginx, Docker daemon) **chưa được xác minh** bằng primary source trong lượt này.

## 2. Bảng so sánh cơ chế Windows user-background

| Dự án | Cơ chế | start/stop/status | Cần admin? | Graceful stop? | Nguồn |
|---|---|---|---|---|---|
| Syncthing (docs) | Task Scheduler, trigger "At log on" (specific user); arg `--no-console --no-browser`; "stay invisible in background" | Thủ công qua taskschd.msc; docs không có CLI | Docs không nói; UI "Administrative Tool". Chọn "Run whether user is logged on or not" cần nhập password | **Không**: "ending the task terminates only the monitor process"; thoát thật qua Web GUI Shutdown | [Syncthing autostart](https://docs.syncthing.net/users/autostart.html) |
| Syncthing (docs) | Startup folder `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup` + shortcut với `--no-console --no-browser` | Không có stop/status | Không (per-user folder) **[Diễn giải]** | n/a | như trên |
| Syncthing (docs) | Service qua NSSM (cho headless server); docs nói đây không phải cách ưu tiên cho end-user | `sc start syncthing` | Có ("administrator Command Prompt") | NSSM stop methods | như trên |
| `auto-launch` crate | `HKCU\...\Run` (CurrentUser) hoặc `HKLM\...\Run`; mặc định thử HKLM, fallback HKCU nếu không đủ quyền | enable/disable/is_enabled; không có start/stop | HKCU: không; HKLM: có | n/a (chỉ đăng ký autostart) | [docs.rs auto_launch](https://docs.rs/auto-launch/latest/auto_launch/) |
| `tauri-plugin-autostart` | Wrapper autostart (enable/disable/isEnabled); trang docs **không nêu** cơ chế Windows | enable/is_enabled/disable | Không nêu | n/a | [Tauri autostart](https://v2.tauri.app/plugin/autostart/) |
| `service-manager` crate (chipsenkbeil/service-manager-rs) | Windows: `sc.exe` (Windows Service) hoặc WinSW; Linux systemd (có user level), launchd, OpenRC, rc.d. **Không có Task Scheduler/HKCU** trong danh sách | install/uninstall/start/stop/status; `RestartPolicy`: WinSW hỗ trợ đầy đủ, sc.exe hạn chế/không | Service ⇒ cần admin **[Diễn giải]** | Tùy sc.exe/WinSW | [docs.rs service_manager](https://docs.rs/service-manager/latest/service_manager/) |
| `windows-service` crate | API viết Windows Service (SCM control handler) | — | Service ⇒ admin **[Diễn giải]** | Có, qua `SERVICE_CONTROL_STOP` **[Diễn giải]** (đã fetch trang docs.rs nhưng không trích dẫn riêng) | [docs.rs windows_service](https://docs.rs/windows-service/latest/windows_service/) |
| WinSW | Wrapper service (XML config) | — | Service ⇒ admin **[Diễn giải]** | — (chưa trích) | [WinSW XML config](https://github.com/winsw/winsw/blob/v3/docs/xml-config-file.md) |
| NSSM | Wrapper service. Stop theo thứ tự: Ctrl-C tới console → `WM_CLOSE` tới windows → `WM_QUIT` tới threads → `TerminateProcess` (mỗi bước mặc định chờ 1500 ms; có `AppStopMethodSkip`) | `nssm install`, `sc start` | Có | Có, nhiều bước | [NSSM usage](https://nssm.cc/usage) |
| cloudflared | Windows Service hệ thống (`cloudflared service install`), config ở `%PROGRAMDATA%`/`%USERPROFILE%\.cloudflared`; failure recovery delay 20s, reset 24h; dùng `svc.IsAnInteractiveSession()` để phân biệt chạy service/console | `service install/uninstall` | Service ⇒ admin **[Diễn giải]** | Có, qua Go svc handler (`graceShutdownC`) | [windows_service.go](https://github.com/cloudflare/cloudflared/blob/master/cmd/cloudflared/windows_service.go), [Cloudflare docs](https://developers.cloudflare.com/tunnel/features/locally-managed-tunnels/as-a-service/windows/) |
| Tailscale (Windows) | Có "unattended mode" chạy khi chưa login (service); GUI `tailscale-ipn.exe` chạy startup. Issue #7345 (Scoop) đề xuất `HKCU\...\Run` cho `tailscale-ipn.exe` vì "traditional install depends on installer having admin privileges and adding to Startup items" | — | Cài đặt chuẩn cần admin | Không xác minh được | [run-unattended](https://tailscale.com/docs/how-to/run-unattended), [issue 7345](https://github.com/tailscale/tailscale/issues/7345) (chỉ từ search snippet, chưa đọc sâu) |
| rclone, restic/autorestic, Docker Desktop, WireGuard, ngrok, Mullvad, mkcert-like | **Không xác minh được** (chưa fetch primary source) | | | | |

Task Scheduler schema (primary, Microsoft Learn; **chỉ từ search snippet**, chưa fetch trọn): `Hidden` chỉ ẩn task khỏi UI (không phải ẩn cửa sổ process) — [Hidden](https://learn.microsoft.com/en-us/windows/win32/taskschd/taskschedulerschema-hidden-settingstype-element); `RestartOnFailure` — [link](https://learn.microsoft.com/en-us/windows/win32/taskschd/taskschedulerschema-restartonfailure-settingstype-element); `RunLevel` 0 = lowest, 1 = elevated — [link](https://learn.microsoft.com/en-us/windows/win32/taskschd/taskschedulerschema-runlevel-principaltype-element). Trang LogonTrigger đã fetch nhưng không trích được thông tin về InteractiveToken: **không xác minh được** trong lượt này.

## 3. Findings Q2 — Stop graceful

1. **IRunningTask::Stop** chỉ ghi "Stops this instance of the task"; trả `E_ACCESSDENIED` nếu user không có quyền stop, task disabled, hoặc không cho chạy on demand. Không nói gì về cơ chế (signal/terminate). [Trực tiếp] — [IRunningTask::Stop](https://learn.microsoft.com/en-us/windows/win32/api/taskschd/nf-taskschd-irunningtask-stop).
2. **schtasks /end**: "Stops only the instances of a program started by a scheduled task"; không nói gì về signal. [Trực tiếp] — [schtasks end](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/schtasks-end).
3. **AllowHardTerminate**: "The service will try to close the running task by sending the WM_CLOSE notification, and if the task does not respond, the task will be terminated only if this property is set to true" (bằng `TerminateProcess`). [Trực tiếp] — [AllowHardTerminate](https://learn.microsoft.com/en-us/windows/win32/taskschd/tasksettings-allowhardterminate). Tài liệu `TASK_TRIGGER_FLAG_KILL_AT_DURATION_END` cũng nêu `WM_CLOSE` rồi terminate nếu không có window ([mstask](https://learn.microsoft.com/en-us/windows/win32/api/mstask/ns-mstask-task_trigger), snippet). **[Diễn giải]**: process console không window ⇒ không có đích `WM_CLOSE`; Microsoft không nói có gửi `CTRL_C/CTRL_CLOSE` hay không ⇒ coi như **không có tín hiệu graceful đáng tin**, cần đo.
4. **Syncthing** xác nhận hành vi thực tế: end task chỉ kill monitor process; dùng Web GUI Shutdown. [Trực tiếp] — [Syncthing autostart](https://docs.syncthing.net/users/autostart.html).
5. **Console control events** (Microsoft Learn): handler chỉ cho *console process*; `CTRL_CLOSE_EVENT` gửi tới mọi process gắn console khi user đóng cửa sổ console hoặc End Task trong Task Manager, timeout ~5000 ms (`SPI_GETHUNGAPPTIMEOUT`); `CTRL_C/BREAK` không timeout; `CTRL_LOGOFF/SHUTDOWN` "received only by services" (interactive app bị kết thúc trước). [Trực tiếp] — [HandlerRoutine](https://learn.microsoft.com/en-us/windows/console/handlerroutine).
6. **tokio::signal::windows**: có `ctrl_c`, `ctrl_break`, `ctrl_close`, `ctrl_logoff`, `ctrl_shutdown`, nghe qua `SetConsoleCtrlHandler`. [Trực tiếp] — [module](https://docs.rs/tokio/latest/tokio/signal/windows/index.html), [ctrl_close](https://docs.rs/tokio/latest/tokio/signal/windows/fn.ctrl_close.html). Trang docs không nêu điều kiện (console/session) — điều kiện lấy từ HandlerRoutine ở mục 5.
7. **NSSM** (service wrapper) gửi Ctrl-C vào console của app trước, rồi `WM_CLOSE`, `WM_QUIT`, cuối cùng `TerminateProcess`; "GUI applications do not have consoles and will not respond to [Ctrl-C]". [Trực tiếp] — [NSSM usage](https://nssm.cc/usage).
8. **cloudflared** (Go) trên Windows dùng Service control handler (`svc` package) và kênh `graceShutdownC`, không dựa Ctrl event khi chạy service. [Diễn giải từ source] — [windows_service.go](https://github.com/cloudflare/cloudflared/blob/master/cmd/cloudflared/windows_service.go).
9. Tailscale, Go/Rust daemon khác: **không xác minh được**.

**Hàm ý [researcher inference]**: với RO-03 (không thêm stop command mới), trên Windows user-background không có đường stop graceful được tài liệu bảo đảm qua `schtasks /end`; nếu daemon đã có kênh shutdown nội bộ thì đó là cách Syncthing dùng; nếu không, chấp nhận hard-terminate và ghi rõ hạn chế. Cần đo (§6).

## 4. Findings Q3 — Console window

- `CREATE_NO_WINDOW` (0x08000000): "process is a console application that is being run without a console window. Therefore, the console handle for the application is not set"; bị bỏ qua nếu dùng với `CREATE_NEW_CONSOLE` hoặc `DETACHED_PROCESS`. [Trực tiếp] — [Process Creation Flags](https://learn.microsoft.com/en-us/windows/win32/procthread/process-creation-flags). `CREATE_NEW_PROCESS_GROUP` vô hiệu hóa CTRL+C cho nhóm mới (cùng trang).
- Rust: `std::os::windows::process::CommandExt::creation_flags(u32)` để đặt flag. [Trực tiếp] — [CommandExt](https://doc.rust-lang.org/std/os/windows/process/trait.CommandExt.html).
- Syncthing: dùng cờ ứng dụng `--no-console` (+ `--no-browser`) khi chạy từ Task Scheduler/Startup; docs nêu "stay invisible in background". Cơ chế bên trong (subsystem windows hay tự detach) **không xác minh được**. [Syncthing autostart](https://docs.syncthing.net/users/autostart.html).
- Task `Hidden` chỉ ẩn task khỏi UI, **không** ẩn cửa sổ process (xem snippet Hidden ở trên). Muốn không hiện console phải dùng cách khác (subsystem windows / launcher).
- `windows_subsystem = "windows"`: **không xác minh được** từ primary source trong lượt này (chưa fetch Rust Reference).
- Hệ quả [Diễn giải từ HandlerRoutine + CREATE_NO_WINDOW]: handler là cơ chế của *console process*; không có console handle (CREATE_NO_WINDOW) hoặc GUI subsystem thì khả năng nhận Ctrl events không được tài liệu bảo đảm → cần đo trên máy thật. NSSM docs xác nhận GUI app "do not have consoles and will not respond" với Ctrl-C.

## 5. Findings Q4 — Trust model

| Dự án | Kiểm tra gì | Fail-closed? | Nguồn |
|---|---|---|---|
| sshd `StrictModes` | Kiểm tra file mode và ownership của file user và home directory trước khi login; mặc định `yes`; `ChrootDirectory` kiểm tra vô điều kiện | Có (mặc định); admin có thể tắt (`no`) | [sshd_config(5)](https://man.openbsd.org/sshd_config.5) |
| sshd(8) authorized_keys | Nếu file, `~/.ssh` hoặc home "writable by other users" thì sshd không cho dùng trừ khi `StrictModes no` | Có | [sshd(8)](https://man.openbsd.org/sshd.8) |
| sudoers | Từ chối/báo lỗi: "/etc/sudoers is world writable" (mode mặc định 0440), sai owner/gid. Timestamp dir: bỏ qua nội dung nếu không owned by root hoặc writable bởi user khác root. `sudoedit` từ chối mở symlink trừ khi `sudoedit_follow`/`FOLLOW`. `fdexec`: tránh TOCTOU bằng thực thi qua file descriptor | Có (từ chối) | [sudoers manual](https://www.sudo.ws/docs/man/sudoers.man/) |
| OpenSSH on Windows | `administrators_authorized_keys`: ACL "chỉ cho phép Administrators và SYSTEM"; hướng dẫn `icacls /inheritance:r /grant Administrators:F /grant SYSTEM:F` (dùng SID `*S-1-5-32-544` cho locale khác) | Docs mô tả yêu cầu; hành vi khi sai ACL (từ chối?) không nêu trong trang này → không xác minh được | [Key-based auth OpenSSH Windows](https://learn.microsoft.com/en-us/windows-server/administration/openssh/openssh_keymanagement) |
| `open(2)` `O_NOFOLLOW` | Chỉ từ chối symlink ở **thành phần cuối**; symlink ở thành phần trước vẫn được theo (ELOOP nếu cuối là symlink). Đây là primitive, không phải policy | n/a | [open(2)](https://man7.org/linux/man-pages/man2/open.2.html) |
| Windows `CreateFileW` | Có cờ `FILE_FLAG_OPEN_REPARSE_POINT` (mở bản thân reparse point/symlink thay vì target) — danh sách flag xuất hiện trong trang; ngữ nghĩa chi tiết chưa trích | n/a | [CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew) |
| Windows `GetFinalPathNameByHandleW` | Trả "final path" đã resolve đầy đủ (symlink `C:\tmp\mydir`→`D:\yourdir` ⇒ `D:\yourdir`); dùng để xác minh đường dẫn thực sau khi đã mở handle | n/a | [GetFinalPathNameByHandleW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getfinalpathnamebyhandlew) |
| systemd `ConfigurationDirectory=` | Tạo thư mục config; mode chỉnh bằng `ConfigurationDirectoryMode=`; "adjusted to what is specified" nếu không khớp. Không phải kiểm tra trust file do user sửa | Điều chỉnh mode, không từ chối | [systemd.exec](https://www.freedesktop.org/software/systemd/man/latest/systemd.exec.html) |
| cron `/etc/cron.d` (root-owned, không group/other-writable) | Trang crontab(5) fetched **không chứa** cụm "owned by"; yêu cầu này **không xác minh được** từ nguồn đã đọc | Không xác minh được | [crontab(5)](https://man7.org/linux/man-pages/man5/crontab.5.html) |
| polkit, doas, Docker daemon config, nginx, Tailscale/WireGuard config perms | **Không xác minh được** (chưa fetch) | — | — |

Ghi chú [Diễn giải]: các tiền lệ xác nhận pattern "kiểm tra owner+mode trên fd đã mở, từ chối symlink, fail-closed" (sshd, sudoers) nhưng sudoers/sshd đều trả lỗi hoặc từ chối, không chỉ cảnh báo. Docs đã đọc không mô tả việc kiểm tra *mọi thư mục cha* ngoài việc sshd nhắc home directory và `~/.ssh`; yêu cầu "mọi thư mục cha" cho Windows là thiết kế riêng, không có tiền lệ trực tiếp đã xác minh.

## 6. Khoảng trống — cần đo trên Windows thật

1. `schtasks /end` / `IRunningTask::Stop` với process console không window: gửi `WM_CLOSE`? `CTRL_C/CTRL_CLOSE`? hay `TerminateProcess` ngay? Có phụ thuộc `AllowHardTerminate`? Docs không nói.
2. Với process chạy qua task (InteractiveToken / LogonTrigger, `Hidden`), có console/handler nào kích hoạt không; `tokio::signal::windows::ctrl_close/ctrl_c` có nhận gì khi stop? Có khác nhau giữa `CREATE_NO_WINDOW`, `DETACHED_PROCESS`, `windows_subsystem="windows"`?
3. Hành vi `RestartOnFailure` sau khi bị terminate bởi `/end` (có restart không? — docs nói restart "if the task fails for any reason", chưa kiểm).
4. `LogonTrigger` + `Principal` `LogonType=InteractiveToken` + `RunLevel=LeastPrivilege`: cài/xoá/query không cần admin cho task của chính user? Chưa xác minh bằng primary source.
5. Rclone, restic/autorestic, Docker Desktop, WireGuard, ngrok, Mullvad, mkcert, Tailscale (graceful stop), polkit, doas, nginx, Docker daemon, cron.d perms: chưa kiểm.
6. Rust `windows_subsystem` và hành vi Ctrl events: chưa kiểm primary.
7. OpenSSH Windows: hành vi khi ACL sai (từ chối hay chỉ cảnh báo) — cần source `sshd`/`win32-openssh`.
8. Ngữ nghĩa chi tiết `FILE_FLAG_OPEN_REPARSE_POINT` + kiểm tra owner/DACL (`GetSecurityInfo`) — chưa trích.
9. Cẩn trọng: nhiều URL repo GitHub/raw (service-manager-rs source, auto-launch source) trả 404 trong lượt này; claim về nội bộ crate chỉ dựa trên docs.rs.

## 7. Danh sách nguồn

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
- Tailscale run-unattended / issue 7345 — https://tailscale.com/docs/how-to/run-unattended , https://github.com/tailscale/tailscale/issues/7345 (chỉ snippet)
- sshd_config(5), sshd(8) — https://man.openbsd.org/sshd_config.5 , https://man.openbsd.org/sshd.8
- sudoers manual — https://www.sudo.ws/docs/man/sudoers.man/
- open(2) — https://man7.org/linux/man-pages/man2/open.2.html
- systemd.exec — https://www.freedesktop.org/software/systemd/man/latest/systemd.exec.html
- crontab(5) — https://man7.org/linux/man-pages/man5/crontab.5.html (không chứa claim cần tìm)

Rejected/deprioritized: StackOverflow / SuperUser về Task Scheduler stop (không phải primary; không dùng làm bằng chứng).
