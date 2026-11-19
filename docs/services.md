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

Verbs: `install`, `uninstall`, `start`, `stop`, `restart`, `status`;
`install` also takes `--config PATH`. The [tier](#user-tier-or-system-tier)
follows privilege: run with `sudo` (or from an elevated Windows terminal) for
a system service, without it for a user service.

Each standalone executable manages its own role without the combined binary.
For example, use `its service install` and `its service status` for Serve,
or `ita service install` and `ita service status` for Access. The same service
verbs and config flag apply; neither executable manages the other role.
Existing combined-binary registrations remain valid; reinstall a role with
its standalone executable to register that executable instead.

- [Backends](#backends)
- [User tier or system tier](#user-tier-or-system-tier)
- [What gets registered](#what-gets-registered)
- [System services](#system-services)
- [BusyBox and SysV init](#busybox-and-sysv-init)
- [Windows](#windows)

## Backends

| Host | Backend | User tier | System tier |
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

## User tier or system tier

- Every verb works in the tier of the process: root, `sudo` or an elevated
  Windows terminal is the system tier, anything else the user tier. There is
  no flag and no prompt; `--user` and `--system` are rejected as unknown
  arguments.
- The tier also picks the default config and state directory; see
  [File locations](configuration.md#file-locations). An administrator who
  wants a user service runs the command unelevated.
- BusyBox/SysV hosts have no user services: a non-root `install`,
  `uninstall`, `start`, `stop` or `restart` exits with code `3`, says to
  re-run with `sudo` and changes nothing.

A user service runs as you and needs no privileges. On Linux it runs while you
are logged in unless lingering is enabled (`loginctl enable-linger $USER`);
`install` never enables it for you.

The two tiers of a role are independent: each has its own registration,
config, key and lock, and both may run at once. When the other tier's instance
of the role is running, `install`, `start`, `restart` and `status` print
`note: <role> is also running as a system service; manage it with sudo or from
an elevated terminal` (user tier) or `note: <role> is also running in a user
session` (system tier) on stderr. Two instances that bind the same local
address fail at bind time, naming the address.

## What gets registered

- The absolute config path and state directory are written into the
  registration (`--config` is resolved against the current directory), so the
  service does not depend on its working directory or environment.
- `install` checks that the config exists and is valid before it registers
  anything; a bad config exits with code `2`.
- `service start` is refused with code `6` while the role already runs in the
  same tier (for example in the foreground). A running instance in the other
  tier only adds the note above.
- On macOS, `install` enables the launchd label, replaces any loaded copy and
  bootstraps the job. If bootstrap fails, a disabled label is the usual cause;
  check `launchctl print-disabled <domain>`.
- For the combined CLI, the registration runs `iroh-tunnel` found on `PATH`,
  or the absolute path of the running executable when there is none. A
  standalone invocation registers its own executable, not the combined CLI.
  A system service on Linux or macOS may run a managed copy instead; see
  [System services](#system-services).

## System services

A system service runs as root (LocalSystem on Windows) from machine-wide files
that only root or administrators can change.

```sh
sudo iroh-tunnel serve config keygen          # /etc/iroh-tunnel/serve.toml, root 0600
sudo iroh-tunnel serve config set -n ssh -t 22
sudo iroh-tunnel serve service install
```

- Without `--config` the service uses the system-tier config and state
  directory (`/etc/iroh-tunnel/<role>.toml` and `/run/iroh-tunnel` on Linux;
  `/Library/Application Support/iroh-tunnel/config/<role>.toml` and
  `/Library/Application Support/iroh-tunnel/state` on macOS). `HOME` and
  `SUDO_USER` are never read. If the config does not exist, install exits with
  code `2`, naming the path and `sudo iroh-tunnel <role> config keygen`.
- Before it registers, and at every start by the service manager, the config,
  the state directory and every directory above them must be owned by root and
  not writable by group or others, and must not be reached through a symbolic
  link. A refusal exits with code `3` (or is logged at a service start) naming
  the path, owner and mode; nothing is repaired. A config owned by a user, such
  as `--config /home/alice/serve.toml`, is refused this way.
- A config with an empty `secret_key` gets a generated key written into it on
  the first service start; the file stays root-owned with mode `0600`.
- A foreground `sudo iroh-tunnel <role> run` is not checked: root picks its own
  files, and containers often bind-mount configs owned by other uids.
- Registrations from versions before the tier change point at a user's files.
  They refuse to start with a config error (exit code `2`) that names the
  system config path and `sudo iroh-tunnel <role> service install`; see the
  migration steps in the [changelog](../CHANGELOG.md).
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
- After upgrading iroh-tunnel, re-run `sudo iroh-tunnel <role> service
  install` to refresh the copy. Until then the service keeps running the old
  copy: `brew upgrade` and `cargo install` do not change it. For a standalone
  service, use the matching command, for example `sudo its service install`
  or `sudo ita service install`.
- `sudo ... service start` and `sudo ... service restart` check the binary the
  service manager will run, read from its loaded definition. If a non-root user
  could change it, or the manager's definition cannot be read, the command
  exits with code `3`, naming the unsafe path and its owner and mode where
  known (or why the binary could not be determined), and issues no start,
  stop or restart, so a running service keeps running. Re-run the `sudo ...
  service install` command the message prints.
  Starts by systemd, launchd or the init script at boot, on failure, or run
  directly are not checked.
- When install needs a managed copy, it exits with code `3`, naming the
  directory, owner and mode, if an existing managed directory is owned by a
  non-root user or is group- or world-writable. Install never repairs it; fix
  it as root, then install again.
- `sudo ... service uninstall` removes the role's copy and its empty directory,
  unless another root-run registration of that role (a unit file, drop-in, init
  script or loaded definition) still names it. If a registration source or the
  loaded definition cannot be read, the copy is kept. A role installed from a
  trusted binary has no copy to remove, and uninstall succeeds.

## BusyBox and SysV init

On Linux hosts without systemd, such as buildroot devices like the Sipeed
NanoKVM, `install` writes `/etc/init.d/S96iroh-tunnel-<role>`. `S96` starts
it after networking. There are no user services: all verbs except `status`
need root, and a non-root call exits with code `3` and changes nothing.
Because that script
runs as root, the [binary trust rules](#system-services) apply to it too: its
`DAEMON` line names the registered binary or the managed copy. `stop` sends
`SIGTERM`, then `SIGKILL` after 5 seconds.

The script stores every path as a single-quoted shell literal, so spaces or
shell metacharacters in the binary, config or state path are never expanded.
The PID file records the daemon's PID and the executable it launched.
`start`, `stop` and `status` only act on that process: before any signal the
script checks `/proc/<pid>/exe` against the recorded executable. A stale PID
file (dead or zombie process, malformed content or another program) is removed and
never signalled. If the identity cannot be checked, for example when `/proc`
is missing, nothing is signalled and the PID file is kept: `start` and `stop`
exit 1 and `status` exits 4 (unknown). `status` exits 3 when the daemon is not
running.

Scripts written by older versions do not have these checks. Run `service
install` again to rewrite the script; it recognises the daemon started by the
old script and restarts it. If the re-install changes both the binary name and
the config path, the old daemon may not be recognised and the new one is
refused by the ownership lock; stop the old process by hand once. To downgrade,
run `service stop` with the current version first, then install the previous
version. Otherwise the older script cannot read the new PID file and leaves the
daemon running unmanaged.

## Windows

### User tier

A user service is a Task Scheduler task that starts at your logon with least
privilege and restarts on failure. No administrator rights are needed.

- `stop` ends the task; `restart` waits up to 15 seconds for the old process
  to exit before starting a new one.
- Install refuses a non-default `IROH_TUNNEL_STATE_DIR`.
- An administrator's non-elevated terminal is the user tier: `install`
  registers this task, never a Windows Service.

### System tier

A system service is a Windows Service that runs as LocalSystem and starts at
boot. Run every command for it from an elevated terminal ("Run as
administrator").

```powershell
iroh-tunnel serve config keygen     # %ProgramData%\iroh-tunnel\config\serve.toml
iroh-tunnel serve config set -n ssh -t 22
iroh-tunnel serve service install
```

- The service runs `%ProgramData%\iroh-tunnel\config\<role>.toml` (or the
  `--config` given) with state in `%ProgramData%\iroh-tunnel\state`. No user
  profile is read and no account is pinned; only a Windows Service is
  registered, never a scheduled task.
- An elevated `config` command creates `%ProgramData%\iroh-tunnel\config`
  readable and writable by SYSTEM and Administrators only, because configs hold
  secret keys. An existing config directory or file that grants read access to
  anyone else (for example `Users`) is refused with code `3` before a key is
  written or served, naming the directory and the account; it is never
  repaired.
- A config with an empty `secret_key` gets a generated key written into it on
  the first service start.
- `stop` asks the service to drain (it reports `STOP_PENDING` while open
  streams finish) and removes its status file.
- The user task and the Windows Service of a role may both exist and run, each
  with its own config and key. A start of the service while another
  system-tier process holds the role stops cleanly (no restart); a user
  instance does not stop it.
- A service registered by an earlier version (with
  `IROH_TUNNEL_TRUSTED_SID`) refuses to start with a config error naming
  `%ProgramData%\iroh-tunnel\config\<role>.toml`; create it and re-run
  `service install` from an elevated terminal.

When a system service holds an executable open during an update, stop the
matching role from an elevated PowerShell ("Run as administrator"):

```powershell
# Standalone executable for each role:
iroh-tunnel-serve.exe service stop
iroh-tunnel-access.exe service stop

# Combined executable for each role:
iroh-tunnel.exe serve service stop
iroh-tunnel.exe access service stop
```

Stop any other process holding the executable before retrying; the updater
never stops it automatically. If the updater itself keeps the file locked,
retry from another copy. For a user task, run the same commands from a
non-elevated terminal.

#### Trust rules

Before it registers, and at every service start, the config file, the state
directory and every directory above them must be owned by and writable only
by SYSTEM, Administrators or TrustedInstaller, and the config directory and
file must not be readable by anyone else. No component may be a junction,
symbolic link or mount point. The service binary and its directories must be
writable only by SYSTEM, Administrators and TrustedInstaller, so a binary
under your profile is refused. A refusal exits with code `3` and names the path, the account and
the rights at fault.

The product never changes access on your files. To fix a refusal, remove the
grant it names (for example `icacls <path> /remove *S-1-1-0` for Everyone) or
move the file, then run `service start` again from an elevated terminal.

The installer (`install.ps1`) run from an elevated session creates its install
directory with this protection. From a non-elevated session it creates a
plain directory, and an elevated `service install` then refuses the binary in
it.

The binary check runs at install and in an elevated `service start|restart`. A
start by the Service Control Manager at boot or through recovery does not
re-check the binary; keep its directory protected.

#### Machine directory

`%ProgramData%\iroh-tunnel` holds the machine-wide role locks, the system
tier's `config` and `state` directories and the service log
`logs\<role>.log` (10 MiB, one rotated file). The first system service
start (or elevated run) creates it readable by Users and writable only by
SYSTEM and Administrators, so the first start already logs.
An existing directory that others can change is refused, at install and at
service start, and never repaired: delete it from an elevated terminal and
install again. Uninstall leaves the directory and its log in place; after
uninstalling every role, remove it from an elevated terminal with
`Remove-Item -Recurse "$env:ProgramData\iroh-tunnel"`.
