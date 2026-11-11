# Running as a service

`iroh-tunnel <role> service ...` registers a role with the host's service
manager so it starts at boot or login. It uses
the role's [config file](configuration.md); quick-mode flags do not apply.

```sh
iroh-tunnel serve config set -n ssh -t 22
iroh-tunnel serve service install            # install, enable and start
iroh-tunnel serve service status
iroh-tunnel serve service restart
iroh-tunnel serve service stop
iroh-tunnel serve service uninstall
```

Verbs: `install`, `uninstall`, `start`, `stop`, `restart`, `status`. Each
takes `--user` or `--system`; `install` also takes `--config PATH`.

Only the all-in-one `iroh-tunnel` manages services. The standalone
`iroh-tunnel-serve` and `iroh-tunnel-access` refuse `service` and point to
`iroh-tunnel <role> service ...`.

- [Backends](#backends)
- [User or system scope](#user-or-system-scope)
- [What gets registered](#what-gets-registered)
- [System services](#system-services)
- [BusyBox and SysV init](#busybox-and-sysv-init)
- [Windows](#windows)

## Backends

| Host | Backend | User scope | System scope |
|---|---|---|---|
| Linux with systemd | systemd unit `iroh-tunnel-<role>.service` | `~/.config/systemd/user/` (`systemctl --user`) | `/etc/systemd/system/` |
| Linux without systemd | init script `/etc/init.d/S96iroh-tunnel-<role>` | system only | `/etc/init.d/` |
| macOS | launchd job `dev.iroh-tunnel.<role>` | LaunchAgent | LaunchDaemon |
| Windows | Task Scheduler task or Windows Service `iroh-tunnel-<role>` | logon task | Windows Service (LocalSystem) |

On Linux the backend is chosen at runtime: systemd when `/run/systemd/system`
exists or PID 1 is systemd, otherwise the init-script backend.

Restart after a failure depends on the backend:

| Backend | Restart |
|---|---|
| systemd | After 5 seconds, except after exit code `6` (the role is already running) |
| launchd | After any unsuccessful exit, at launchd's own pace |
| BusyBox init script | None; the script only starts the role at boot |
| Windows task | Retried every minute |
| Windows Service | After 5 seconds, except after exit code `6` (the role is already running) |

## User or system scope

- `install` with `--user` or `--system` uses that scope. Without a flag, root
  (or an elevated Windows terminal) gets a system service, an interactive
  non-root user is asked, and a non-interactive non-root caller gets a user
  service.
- Other verbs without a flag use system scope when run elevated and user scope
  otherwise. They never ask.
- A system-scope command without privilege exits with code `3` and prints the
  `sudo ...` command to run (on Windows: run it from an elevated terminal),
  including the absolute config path. Nothing is
  elevated for you, and nothing falls back to a user service.

A user service runs as you and needs no privileges. On Linux it runs while you
are logged in unless lingering is enabled (`loginctl enable-linger $USER`).

## What gets registered

- The absolute config path and state directory are written into the
  registration (`--config` is resolved against the current directory), so the
  service does not depend on its working directory or environment.
- `install` checks that the config exists and is valid before it registers
  anything; a bad config exits with code `2`.
- `service start` is refused with code `6` while the role already runs in the
  foreground.
- `install` is refused with code `6` while the role is held at the other
  scope (for example a running system service when you install `--user`). The
  error names the holder, how to stop it (`Ctrl+C`, or `iroh-tunnel <role>
  service stop --system` or `--user`) and how to remove it (`iroh-tunnel <role>
  service uninstall --system` or `--user`). Nothing is stopped automatically.
- On macOS, `install` enables the launchd label, replaces any loaded copy and
  bootstraps the job. If bootstrap fails, a disabled label is the usual cause;
  check `launchctl print-disabled <domain>`.
- The registration runs the `iroh-tunnel` found on `PATH`, or the absolute
  path of the running executable when there is none. A system service on Linux
  or macOS may run a managed copy instead; see [System services](#system-services).

## System services

A system service runs as root but trusts one user's files, so you can manage
its config without root.

```sh
sudo iroh-tunnel serve service install --system
```

- The owner is taken from `SUDO_USER`, then from the owner of an explicit
  `--config` directory. If they disagree the install fails, naming both users.
- The service uses that user's default config and state paths, which must
  already exist: run `iroh-tunnel serve config keygen` as that user first.
  Root never creates directories in that user's home.
- Before it registers, install checks that the config and state directory are
  owned by root or that user, are not writable by others and are not reached
  through unsafe links. The service checks them again at every start and fails
  closed.
- The service never writes a generated key. A config without `secret_key`
  fails with a message to run `config keygen` as the owner.
- Reinstalling over a service owned by another user asks for confirmation;
  without it the command exits with code `5` and changes nothing.
- Binary trust (Linux and macOS): install resolves the binary to its canonical
  path and checks it. The file and every directory above it must be owned by
  root and not writable by group or others; a sticky directory such as `/tmp`
  is accepted only when the entry below it is root-owned. If the check fails
  (Homebrew, `cargo install`, a binary under your home), install registers a
  root-owned copy of the running executable instead, at
  `/var/lib/iroh-tunnel/bin/<role>/iroh-tunnel` on Linux or
  `/Library/Application Support/iroh-tunnel/bin/<role>/iroh-tunnel` on macOS,
  and prints a note. Each role has its own copy. A binary that passes the check,
  such as a root-owned `0755` file in `/usr/local/bin`, is registered as is,
  with no copy.
- After upgrading iroh-tunnel, re-run `sudo iroh-tunnel <role> service install
  --system` to refresh the copy. Until then the service keeps running the old
  copy: `brew upgrade` and `cargo install` do not change it.
- `service start --system` and `service restart --system` check the binary the
  service manager will run, read from its loaded definition. If a non-root user
  could change it, or the manager's definition cannot be read, the command
  exits with code `3`, naming the unsafe path and its owner and mode where
  known, and issues no start, stop or restart, so a running service keeps
  running. Re-run the `service install --system` command the message prints.
  Starts by systemd, launchd or the init script at boot, on failure, or run
  directly are not checked.
- When install needs a managed copy, it exits with code `3`, naming the
  directory, owner and mode, if an existing managed directory is owned by a
  non-root user or is group- or world-writable. Install never repairs it; fix it as root, then install again.
- `service uninstall --system` removes the role's copy and its empty directory,
  unless another root-run registration of that role (a unit file, drop-in, init
  script or loaded definition) still names it. If a registration source or the
  loaded definition cannot be read, the copy is kept. A role installed from a trusted
  binary has no copy to remove, and uninstall succeeds.

## BusyBox and SysV init

On Linux hosts without systemd, such as buildroot devices like the Sipeed
NanoKVM, `install` writes `/etc/init.d/S96iroh-tunnel-<role>`. `S96` starts
it after networking. All verbs except `status` need root, and a user-scope
request installs the same system-wide script, with a note. Because that script
runs as root, the [binary trust rules](#system-services) apply to it too: its
`DAEMON` line names the registered binary or the managed copy. `stop` sends
`SIGTERM`, then `SIGKILL` after 5 seconds.

## Windows

### User scope

A user service is a Task Scheduler task that starts at your logon with least
privilege and restarts on failure. No administrator rights are needed.

- `stop` ends the task; `restart` waits up to 15 seconds for the old process
  to exit before starting a new one.
- Install refuses a non-default `IROH_TUNNEL_STATE_DIR`.

### System scope

A system service is a Windows Service that runs as LocalSystem, starts at
boot and trusts one user's files, the same model as on Linux and macOS.

```powershell
# From an elevated terminal ("Run as administrator"):
iroh-tunnel serve service install
```

- From an elevated terminal, `install` without a flag targets system scope.
  Without elevation, `--system` exits with code `3` and prints the command to
  run from an elevated terminal. Nothing is elevated for you.
- The installing user (the token user, also when elevated) is pinned: the
  service uses that user's default `%LOCALAPPDATA%` config and state paths,
  or the `--config` given. A `--config` in a directory owned by another user
  exits with code `2`, naming both accounts. A directory owned by
  Administrators is fine.
- The pin is stored in the service description. Reinstalling over a service
  pinned to another user asks for confirmation; without it the command exits
  with code `5` and changes nothing.
- The service never writes a generated key: a config without `secret_key`
  fails and the log says to run `config keygen` as the owner.
- `stop` asks the service to drain (it reports `STOP_PENDING` while open
  streams finish) and removes its status file.
- A role cannot be installed in both scopes: installing a system service while
  the user task exists (or the reverse) exits with code `5`, names the
  `service uninstall` command for the other scope and changes nothing.
- While the system service runs, a user start of the same role exits with code
  `6`; a system start while a user process runs stops cleanly (no restart).

#### Trust rules

Before it registers, and at every service start, the config file, the state
directory and every directory above them must be owned by and writable only
by SYSTEM, Administrators, TrustedInstaller or the pinned user. No component
may be a junction, symbolic link or mount point. The service binary and its
directories must be writable only by SYSTEM, Administrators and
TrustedInstaller, never by the pinned user, so a binary under your profile is
refused. A refusal exits with code `3` and names the path, the account and
the rights at fault.

The product never changes access on your files. To fix a refusal, remove the
grant it names (for example `icacls <path> /remove *S-1-1-0` for Everyone) or
move the file, then run `service start --system` again.

The installer (`install.ps1`) run from an elevated session creates its install
directory with this protection. From a non-elevated session it creates a
plain directory, and `service install --system` then refuses the binary in it.

The binary check runs at install and in `service start|restart --system`. A
start by the Service Control Manager at boot or through recovery does not
re-check the binary; keep its directory protected.

#### Machine directory

`%ProgramData%\iroh-tunnel` holds the machine-wide role locks and the service
log `logs\<role>.log` (10 MiB, one rotated file). The first system service
start (or elevated run) creates it readable by Users and writable only by
SYSTEM and Administrators, so the first start already logs.
An existing directory that others can change is refused, at install and at
service start, and never repaired: delete it from an elevated terminal and
install again. Uninstall leaves the directory and its log in place; after
uninstalling every role, remove it from an elevated terminal with
`Remove-Item -Recurse "$env:ProgramData\iroh-tunnel"`.
