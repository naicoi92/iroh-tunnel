# Troubleshooting

**The Serve NodeId changes after every restart.**
In quick mode, each start uses a fresh identity; pass `--key-file`. With a
config file, the key is saved on the first `run`; if the file is read-only,
`run` fails instead. Run `config keygen` once, or put a key from
`config keygen --print` into the file.

**`run` fails with "already running" (exit code 6).**
Each role runs as one process per user. Another `serve` (or `access`) is
running, maybe as a service. `iroh-tunnel <role> status` shows the owner; stop
it with `Ctrl+C` or `service stop`. A Serve and an Access on the same host also
need different keys.

**Access starts but clients hang or time out.**
Check, in order:

1. The `node_id` is the Serve's current NodeId, and the service `name` matches
   on both sides.
2. Serve is running: `iroh-tunnel serve status`.
3. Both sides can reach a common relay. With custom relays, Serve must be
   registered on every relay Access dials through, and a token-enforcing
   relay needs the right `relay_token`.
4. The Serve host can connect to the target (`host:port` in its config). Serve
   gives each channel 10 seconds to connect.

Run both sides with `-vv` and compare the `peer=` ids in the logs.

**The connection stays on the relay.**
Hole punching did not succeed, often because of symmetric NAT or a firewall
that blocks outgoing UDP. Traffic still flows, encrypted, through the relay.
`status` and the `path changed` log lines show when a direct path appears.
A [self-hosted relay](self-hosted-relay.md) close to both peers lowers
latency.

**A second client hangs against an old Serve.**
Serve older than 0.2.0 accepts one stream per connection, and Access always
reuses one connection. Upgrade Serve; `multiplex = false` is ignored.

**The `.apk` will not run on Debian or Ubuntu (or the `.deb` on Alpine).**
The `.apk` is built for musl, the `.deb` for glibc. Use the package for the
host's libc.

**`update` refuses to replace the executable.**
It was installed by Homebrew, a Linux package or a container image. Update it
through that tool, or pass `--yes` to replace a Homebrew or package file
anyway. On Windows, stop the running executable first.

**`service install --system` exits with code 3.**
System services need root; the error prints the `sudo` command to run. On
Windows, open a terminal with "Run as administrator" and run the printed
command there.

**`service start --system` or `restart --system` exits with code 3 and names a binary.**
On Linux and macOS the service manager would run a binary that a non-root user
can change (for example one under your home directory, or a directory above it
that is group- or world-writable). The message names the path, owner and mode.
Nothing was started, stopped or restarted, so a running service keeps running.
Re-run the install as root, then start again:

```sh
sudo iroh-tunnel <role> service install --system
sudo iroh-tunnel <role> service start --system
```

Install registers the binary directly if it is root-owned, or otherwise a
root-owned copy at `/var/lib/iroh-tunnel/bin/<role>/iroh-tunnel` (Linux) or
`/Library/Application Support/iroh-tunnel/bin/<role>/iroh-tunnel` (macOS). See
[System services](services.md#system-services).

**A Windows system service refuses to start: `permission error`.**
The config, state directory, binary or `%ProgramData%\iroh-tunnel` can be
changed by an account the service does not trust, or the path goes through a
junction or link. The message names the path, account and rights; the service
log is `%ProgramData%\iroh-tunnel\logs\serve.log` (or `access.log`). Remove
that access or move the file, then run `service start --system`. See
[Windows system scope](services.md#system-scope).

**`the service binary ... is not protected`.**
The `iroh-tunnel.exe` on `PATH` is in a directory you can write, such as your
profile. Install it with `install.ps1` from an elevated terminal (or into
`%ProgramFiles%`), then install the service again.

**`config edit` says my change was rejected.**
The edited file did not validate, or the file changed while you were editing.
The live file is unchanged and your edit is in `<config>.rejected`.

**Is UDP supported?**
Experimentally, and both peers need 0.5.0 or later. See [UDP](udp.md).

**Can Serve and Access run on the same machine?**
Yes. Use separate configs (the defaults already are) and different keys.

**How do I see what the tunnel is doing?**
Use `-vv` (debug) or `RUST_LOG=iroh_tunnel=debug`, and `iroh-tunnel <role>
status`. See [Monitoring](monitoring.md).

Still stuck? Open an
[issue](https://github.com/naicoi92/iroh-tunnel/issues) with
`iroh-tunnel --version`, your OS, how you installed, and logs at `-vv`.
