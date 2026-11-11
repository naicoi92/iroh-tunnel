## Design Summary

A Linux or macOS system service runs its registered binary as root, but today only the pinned config and state paths are trust-checked (`system-service-management`: "Privileged service refuses unsafe config paths", "System install verifies files before registering"). When the registered binary sits where a non-root principal can write (Homebrew prefix, `~/.cargo/bin`, a user-owned `install.sh --bin-dir`), any process of that user can replace it and gain root at the next boot, restart or crash recovery (issue #186, mirroring #184 for Windows).

The intent is that root never automatically executes a file a non-root principal can change, **without** taking `service install --system` away from any supported install method (install.sh, Linux packages, Homebrew cask, cargo).

The key observation: an administrator running `sudo iroh-tunnel <role> service install --system` already executes that binary as root, so it is trusted at that moment. What must be guaranteed is that the service keeps running *that* binary, not a file a user can change later.

Behaviour:

1. **Install (`service install --system`).** The binary that would be registered (resolved as today, `system-service-management` "Binary resolution for registrations") is checked with the same no-follow descent from `/` used for config files, trusting **root only** (the pinned user is not trusted for the binary, unlike the config):
   - **Trusted** (e.g. `/usr/local/bin` from `install.sh`, `/usr/bin` from a Linux package): registered directly, as today. Self-update and package managers keep updating the service binary as now.
   - **Not trusted** (Homebrew, cargo, user-owned bin dir): the executable running the install is copied into a tool-managed directory writable only by root, and that copy is registered. A note tells the operator which binary the service runs and that `service install --system` must be re-run to pick up a newer version. The user's original file is never changed (no chown/chmod).
2. **CLI `service start --system` / `service restart --system`.** Re-verify the binary recorded in the registration, root-only, before asking the service manager to start it. Start/restart **never** copy or refresh the binary. An unsafe recorded binary (for example a service registered before this change from a user-writable path) is refused with a permission error (exit 3) naming path, owner and mode and telling the operator to re-run `service install --system`, which fixes it through step 1.
3. **Uninstall (`service uninstall --system`).** Removes the managed copy once no remaining system registration of any role refers to it.
4. **No runtime self-check.** The service process does not verify its own binary at start (a replaced binary would simply skip such a check). Already-running services and boot-time starts of existing registrations keep working; nothing stops on its own after upgrading iroh-tunnel.

Existing requirements altered:
- "Binary resolution for registrations" (`system-service-management`): for system scope the registered path may be the managed copy instead of the resolved binary.
- New requirements for binary trust timing and refusal on Linux/macOS, parallel to `windows-service-trust` "Trusted principals for a Windows system service" and "Trust check timing". Windows behaviour is unchanged.
- "Installers never touch services" (`install-release-packaging`) stays as is.

## Alternatives Considered

### Alternative A: Refuse any untrusted binary at install
- **Approach**: Fail closed at install and CLI start when the binary is not root-only writable; tell the user to reinstall via `install.sh` into `/usr/local/bin`.
- **Pros**: Smallest change; identical to Windows.
- **Cons**: Breaks `--system` for the official macOS install method (Homebrew, whose prefix is user-owned) and for cargo installs.
- **Why not chosen**: The reviewer rejected options that break specified functionality.

### Alternative B: Trust the pinned user for the binary when it is a Homebrew install
- **Approach**: Accept a binary writable by the pinned uid.
- **Pros**: No behaviour change for anyone.
- **Cons**: Keeps exactly the privilege escalation #186 closes; contradicts the Windows rule.
- **Why not chosen**: Does not fix the issue.

### Alternative C: Managed copy refreshed at install and at CLI start/restart
- **Approach**: As the agreed approach, but start/restart also refresh the copy from the binary the operator is running.
- **Pros**: `brew upgrade` + `service restart --system` picks the new version automatically.
- **Cons**: Start/restart silently change which binary root runs; more paths that write root-executed files.
- **Why not chosen**: The reviewer chose to keep start/restart free of binary changes; only an explicit install replaces the binary.

## Agreed Approach

Register a root-only-writable binary directly; otherwise register a managed root-owned copy of the executable running the install. Refresh the copy only on `service install --system`. Re-verify the recorded binary at CLI start/restart and refuse an unsafe one with reinstall guidance. Remove the copy on uninstall when unused. Chosen because it closes #186 while every supported install method keeps a working `--system` path, and only an explicit administrator action changes what root executes.

## Key Decisions

- Trusted principal for the binary and every directory above it: root only (pinned user excluded), consistent with Windows (#184).
- Copy refresh happens only on `service install --system` (reviewer decision); start/restart never modify the binary.
- No runtime self-check by the service binary.
- Legacy registrations with an unsafe binary keep running and keep starting at boot; only CLI start/restart refuse, with guidance to re-install. Needs a CHANGELOG note (behaviour change for CLI start/restart).
- Never chown/chmod the user's binary or directories; refusals name path, owner and mode as a permission error (exit 3).
- Applies to systemd, launchd and BusyBox system scope; user scope is unchanged.

## Open Questions

- Location of the managed directory per platform and copy naming/sharing between roles — left to design (a single copy shared by roles means a reinstall of one role changes the binary of the other role's service at its next start; design must state the chosen behaviour).
- Whether `service status --system` should show which binary is registered (managed copy vs direct) — left to design; not required for the fix.
