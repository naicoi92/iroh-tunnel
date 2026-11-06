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
| Windows | Task Scheduler task `iroh-tunnel-<role>` | logon task | not supported |

On Linux the backend is chosen at runtime: systemd when `/run/systemd/system`
exists or PID 1 is systemd, otherwise the init-script backend.

Restart after a failure depends on the backend:

| Backend | Restart |
|---|---|
| systemd | After 5 seconds, except after exit code `6` (the role is already running) |
| launchd | After any unsuccessful exit, at launchd's own pace |
| BusyBox init script | None; the script only starts the role at boot |
| Windows task | Retried every minute |

## User or system scope

- `install` with `--user` or `--system` uses that scope. Without a flag, root
  gets a system service, an interactive non-root user is asked, and a
  non-interactive non-root caller gets a user service.
- Other verbs without a flag use system scope when run elevated and user scope
  otherwise. They never ask.
- A system-scope command without privilege exits with code `3` and prints the
  `sudo ...` command to run, including the absolute config path. Nothing is
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
- The registration runs the `iroh-tunnel` found on `PATH`, or the absolute
  path of the running executable when there is none.

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

## BusyBox and SysV init

On Linux hosts without systemd, such as buildroot devices like the Sipeed
NanoKVM, `install` writes `/etc/init.d/S96iroh-tunnel-<role>`. `S96` starts
it after networking. All verbs except `status` need root, and a user-scope
request installs the same system-wide script, with a note. `stop` sends
`SIGTERM`, then `SIGKILL` after 5 seconds.

## Windows

Windows supports user services only: a Task Scheduler task that starts at
your logon with least privilege and restarts on failure. No administrator
rights are needed. `--system` exits with code `5` and suggests `--user`.

- `stop` ends the task; `restart` waits up to 15 seconds for the old process
  to exit before starting a new one.
- Install refuses a non-default `IROH_TUNNEL_STATE_DIR`.
