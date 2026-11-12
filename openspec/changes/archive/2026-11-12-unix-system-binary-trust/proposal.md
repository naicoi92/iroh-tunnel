## Why

A Linux or macOS system service runs its registered binary as root, but only the pinned config and state paths are trust-checked. A binary in a user-writable location (Homebrew prefix, `~/.cargo/bin`, a user-owned `--bin-dir`) can be replaced by that user and is then run as root at the next boot, restart or crash recovery (#186). Windows already refuses this (#184); Unix must close the same hole without taking `service install --system` away from any supported install method.

## What Changes

**System install binary**
- From: the binary found on `PATH` (else the current executable) is registered as is, whoever can write it.
- To: the resolved binary is canonicalized and checked root-only (no-follow descent from `/`; the pinned user is not trusted). A passing binary is registered directly. Otherwise a root-owned copy of the running executable is written to a per-role managed directory and that copy is registered, with a note telling the operator to re-run `service install --system` after upgrading. The copy is refreshed only by `service install --system`.
- Reason: root must never automatically run a file a non-root principal can change; the administrator running the install as root is the moment that binary is trusted.
- Impact: non-breaking for install; Homebrew and cargo users get a working system service from a managed copy.

**CLI `service start` / `service restart` for a root-run registration**
- From: the service manager is asked to start whatever the registration names.
- To: the binary recorded in the registration is re-checked root-only first; an unsafe one is refused with a permission error (exit 3) naming path, owner and mode and telling the operator to re-run `service install --system`. Start and restart never copy or replace the binary.
- Reason: catches a registration made before this change, or a binary directory loosened after install.
- Impact: **Breaking** for CLI start/restart of an existing system service whose binary is user-writable; it keeps running and keeps starting at boot until the operator re-installs. CHANGELOG note required.

**Uninstall**
- `service uninstall` of a root-run registration also removes that role's managed copy.

Non-goals: Windows behaviour (unchanged); user-scope services on systemd/launchd; a binary self-check inside the running service; changing installers, self-update or packaging; checking binaries at boot-time starts by the service manager.

## Capabilities

### New Capabilities
- `unix-service-binary-trust`: which principals a root-run Unix registration trusts for its binary, how the binary is checked and refused, the managed root-owned copy (creation at install, removal at uninstall), and when the check runs (install, CLI start/restart) and when it does not (service-manager starts).

### Modified Capabilities
- `system-service-management`: "Binary resolution for registrations" changes for root-run Unix registrations: the registered path is the canonical resolved binary when it is root-only writable, otherwise the managed copy of the running executable.

## Impact

- Code: `crates/host/src/service/trust.rs` (root-only trust mode), new managed-binary module under `crates/host/src/service/`, `install.rs` (plan and execute), `verb.rs` (start/restart pre-check), backends `systemd.rs`, `launchd.rs`, `busybox.rs` (read the registered binary back; uninstall cleanup), `native.rs` + `src/render.rs` (new note), `types.rs` (new applied effects).
- Docs: `docs/services.md`, `CHANGELOG.md`.
- Security: closes local privilege escalation through a user-writable service binary on Linux/macOS. No new dependency, no config or CLI flag change, no DB/schema change.
- Platforms: Linux (systemd, BusyBox) and macOS (launchd); Windows untouched.
