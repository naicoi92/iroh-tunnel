## Context

`brainstorm.md` fixes the behaviour: register a root-only-writable binary directly, otherwise register a managed root-owned copy; refresh the copy only at `service install --system`; re-check the recorded binary at CLI start/restart; remove the copy at uninstall. This document records how.

Current state:
- `install.rs` resolves nothing itself: `InstallRequest.tunnel_executable` comes from `src/cli_service.rs::resolve_tunnel_executable` (`which iroh-tunnel`, else `current_exe`) and is written verbatim into the unit (`ExecStart=`), plist (`ProgramArguments[0]`) or BusyBox script (`DAEMON="…"`).
- `trust.rs::walk(path, trusted_uid, want_dir)` already does the no-follow, handle-relative descent from `/` and accepts an owner when `owner == 0 || owner == trusted_uid`. Passing `0` as the trusted uid makes it a root-only check.
- `verb.rs::verb_via` runs privilege → (start only) `ensure_not_running` → backend. Windows system verbs already re-verify the SCM's recorded binary (`system_verb.rs::start_system_checked`).
- Root-run registrations on Unix: system scope on systemd and launchd; every scope on BusyBox (`platform::system_wide_only()`). The brainstorm's "user scope is unchanged" holds for systemd and launchd. BusyBox has no user scope: a `--user` request writes the same root-run init script, so the binary rules apply to it too. Its config pin, trust and retarget behaviour stays as today (no pin), which is out of scope here.

## Goals / Non-Goals

**Goals:**
- Root never automatically executes a file a non-root principal can change, for registrations written by this version.
- `service install --system` keeps working for install.sh, Linux packages, Homebrew and cargo installs.
- CLI start/restart refuses an unsafe recorded binary (legacy registrations) with reinstall guidance.

**Non-Goals:**
- Windows (already covered by `windows-service-trust`).
- User-scope services on systemd/launchd (they run as the user).
- Checking at boot or service-manager restarts; a self-check inside the running service.
- Changing install.sh, packages, the Homebrew cask or self-update.

## Decisions

### D1. Bounded context and types
All of this lives in the host crate's service context (`crates/host/src/service/`). A new module `service_binary.rs` owns the invariant "the binary of a root-run registration is writable by root only" and the managed-copy lifecycle:
- `enum ServiceBinary { Direct(PathBuf), Managed { source: RunningExecutable, copy: PathBuf } }`. It is a pure value, compiled on every target.
- `choose(resolved, running, copy, verify) -> Result<ServiceBinary>`: pure apart from the injected `verify`; writes nothing.
- `materialize(&ServiceBinary, owners) -> Result<Option<AppliedEffect>>`: writes the copy for `Managed`.
- `release(copy, owners, referenced) -> Result<Option<AppliedEffect>>`: removes an unreferenced copy.
- `managed_copy_path(role)`: per-OS base, test override.

The OS-touching functions are `#[cfg(any(target_os = "linux", target_os = "macos"))]`; other targets always plan `Direct(resolved)` and never call them.

`trust.rs` gets an explicit owner set instead of a bare `u32` at the walk boundary (`enum TrustedOwners { RootOnly, RootAnd(u32) }`, RS-03), so the binary check cannot accidentally pass a pinned uid. `trust::verify_binary(path, owners)` checks a regular file, and production passes `RootOnly`. Existing helpers keep their behaviour through `RootAnd(uid)`.

Alternative considered: reuse `trust::read_verified(path, 0)`. Rejected: it reads the whole binary into a `String` and hides the "root only" intent behind a magic `0`.

### D2. Which binary is registered
At install of a root-run registration, after the existing trust and config checks (`plan_system_install`, and the BusyBox user-scope path of `plan_install`):
1. Canonicalize the resolved binary (`trust::canonical`). Root-owned links such as merged-usr `/bin` or `/usr/local/bin/it` resolve once here.
2. If the canonical path passes `verify_binary(…, RootOnly)`, register it (`Direct`).
3. Otherwise register `Managed { source: running executable, copy: <base>/<role>/iroh-tunnel }`.

The copy source is the **running executable**, not the resolved one: the administrator executed that file as root, so it is the one trusted. `RunningExecutable { display: PathBuf, read_from: PathBuf }` separates what the note shows from what is opened. On Linux `read_from` is `/proc/self/exe`, opened directly (never `read_link`), so the copy comes from the executing inode even if the path was replaced or unlinked after exec. On macOS it is `current_exe()`. `display` is `current_exe()` on both. `InstallRequest` gains `running_executable: RunningExecutable`, filled by the CLI; tests point `read_from` at a fixture.

Alternative considered: copy the resolved binary. Rejected: the resolved file may be a different binary the administrator never ran.

### D3. Managed directory
- Linux: `/var/lib/iroh-tunnel/bin/<role>/iroh-tunnel`. `/var/lib` is root-owned and writable on read-only-`/usr` and BusyBox hosts, where `/usr/local/libexec` may not be.
- macOS (Apple silicon only; no Intel builds are published): `/Library/Application Support/iroh-tunnel/bin/<role>/iroh-tunnel`, the system location for daemon support files, root-owned on a stock install and outside both Homebrew (`/opt/homebrew`) and `install.sh` (`/usr/local/bin`) prefixes, so neither package manager touches the copy.
- One copy **per role**, so re-installing one role never changes the binary another role's service starts. This settles the brainstorm's open question.
- Test builds (`test-utils`) take the base from `IROH_TUNNEL_TEST_MANAGED_BIN_DIR`, following `ownership::machine_dir`. Production builds ignore the variable.

### D4. Writing the copy
Directories are created through verified handles only: `trust::ensure_dir(path, owners, mode)`. It descends from `/` with the no-follow walk, verifying every existing component. For each missing component it calls `mkdirat(parent_fd, name, mode)`, opens the new child with `O_NOFOLLOW | O_DIRECTORY`, `fchmod`s that descriptor to `mode` (the umask cannot change it), and `fstat`-verifies it before descending. On `EEXIST` it only opens and verifies, never chmods. No pathname `chmod` is ever issued.

The copy is then published through the verified role-directory handle: open the temp file with `O_EXCL | O_NOFOLLOW`, stream from `read_from`, `fchmod 0755` on the temp handle, `fsync`, then `renameat`. On any failure before the rename, `unlinkat` the temp file, leaving the old copy and the registration untouched. This is the `trust::publish` pattern: `publish` gains an owner set, a `Read` source and a mode applied with `fchmod`, instead of a second implementation. Finally `verify_binary` checks the published file.

A directory that fails the walk (another user's, group-writable) is a permission error (exit 3) naming path, owner and mode; nothing is registered and nothing below it is created. Replacing the file while an old service process runs is safe on Unix: rename leaves the old inode in place.

### D5. Ordering inside install
Planning stays write-free: the plan computes `ServiceBinary` and returns before any effect, so a retarget or scope prompt writes nothing. Execution writes the copy first, recorded as `AppliedEffect::ManagedBinaryWritten(path)`, then the registration with `ServiceBinary::path()`. When the registration step fails after the copy was written, the copy stays and `ServiceFailure.applied` lists it; the next install overwrites it. A `ServiceNote::ManagedBinary { role, source, copy }` is emitted before `Installed`.

### D6. Re-check at CLI start/restart
In `verb_via`, for `Start` and `Restart` of a root-run registration on Linux or macOS, after the privilege check (and after `ensure_not_running` for start): ask the backend for the **effective** binary with a read-only query, `verify_binary` it, and only then call the backend verb. A refusal issues no start, stop or restart command, so a running service keeps running.

The query result is `enum EffectiveBinary { NotLoaded, Binary(PathBuf), Unknown(String) }`: `NotLoaded` skips the check and lets the backend verb report as today; `Unknown` is a permission error with reinstall guidance.
- systemd: `systemctl show -p LoadState -p ExecStart <unit>`. `LoadState=not-found` → `NotLoaded`. `LoadState=loaded` with exactly one `{ path=<binary> ; … }` → `Binary`. Loaded but empty, malformed or with several entries → `Unknown`. A failing query → `Unknown`.
- launchd: `launchctl print system/<label>`. Exit 0 with a `program = <binary>` line (falling back to the first `arguments` entry) → `Binary`. Stderr containing `Could not find service` → `NotLoaded` (observed: exit 113 on macOS 15). Any other failure, or no program → `Unknown`.
- BusyBox: the init script is re-read on every run, so `DAEMON="…"` from the file is the effective binary. A missing script → `NotLoaded`; no `DAEMON` → `Unknown`.

The parsers are pure functions in a portable module, `service/effective_binary.rs`, unit-tested on every OS against captured outputs (with their source and version noted in the fixture). The query adapters live in the OS-gated backends and run through `Native`, so tests use `FakeRunner`.

Alternatives considered: parse the registration file (rejected by spec review: a loaded launchd job or a systemd drop-in can differ from the file on disk); record the binary in the pin comment (rejected: legacy registrations lack it, and it is not what runs).

### D7. Cleanup of a managed copy
`release(copy, owners, referenced)` runs after a root-run uninstall succeeds and after an install that registered `Direct`. It keeps the copy if `referenced()` returns `true` or an error (conservative). `platform::managed_copy_referenced(native, sources, needle)` returns `true` when any source contains the escape-free tail `iroh-tunnel/bin/<role>/iroh-tunnel` (no renderer rewrites it), or when any source cannot be read or queried:
- `RegistrationSources { files, drop_in_dirs }` built by `RegistrationSources::for_role(role)`: the systemd system unit, the BusyBox init script and the launchd daemon plist (with the test suffix in test builds), plus `/etc/systemd/system/<unit>.d` and `/run/systemd/system/<unit>.d` (`*.conf`). Tests build one over a temp root.
- the loaded definition: the D6 query of the active backend, where `Binary(p)` containing the tail counts, `NotLoaded` does not, and `Unknown` counts.

When unreferenced, removal is handle-relative and never follows a link. A missing role directory (`symlink_metadata` → `ENOENT`) is absent. Otherwise the role directory is walked root-only, then `unlinkat(dirfd, "iroh-tunnel", 0)` runs (`ENOENT` is absent, a dangling or live link is unlinked itself), then `unlinkat(basefd, role, AT_REMOVEDIR)` ignoring `ENOTEMPTY`. A follow-based `exists()` is never used. The removal is reported as `AppliedEffect::ManagedBinaryRemoved(path)`. A cleanup failure after the registration was removed returns a `ServiceFailure` whose `applied` already lists `RegistrationRemoved`.

### D8. Error classification
Every trust-policy refusal (an unsafe binary, directory or managed directory, or an unreadable effective definition) is `CliError::Permission` (exit 3). Messages name path, owner uid and mode where known (`Entry::describe`). Start/restart refusals also give `sudo iroh-tunnel <role> service install --system` as the fix. Operational I/O failures while copying or publishing (read, write, rename) keep their own context. They are not reclassified as permission errors, and they leave the old copy and the registration unchanged. The original user binary is never chowned or chmodded.

### D9. Test seams
- **Trust policy:** tests run as a non-root user, so "root" is stood in by `TrustedOwners::RootAnd(me())` and "another user" by `RootAnd(other)` where `other != me()`. Every `service_binary` and `trust` function takes the owner set as a parameter; only the production call sites pass `RootOnly`. `root_only_*` tests use real system files (`/bin/sh`) and fixture files owned by the test user.
- **Managed base:** `IROH_TUNNEL_TEST_MANAGED_BIN_DIR`, honoured only with `test-utils`. It replaces only the root; the suffix `iroh-tunnel/bin/<role>/iroh-tunnel` is always appended, so the reference needle matches managed paths in tests too.
- **Owner policy:** a pure `check_entry(owner, mode, kind, owners)` predicate decides trust from metadata, so "owned by another user" is unit-tested without a second account. Real ownership cases run in privileged E2E.
- **Existing registration:** planning reads it through an injected `existing` function.
- **Verb execution:** `verb_steps(native, notes, verb, role, guard, backend, release_after_uninstall)` takes its effects as closures, like `install_steps`; closures that need the runner receive `&mut Native`.
- **Registration sources:** `RegistrationSources` is a value, so tests pass temp paths.
- **Backend selection:** `platform::root_run(scope, system_wide_only: bool)` is pure; the install plan step `plan_binary(req, scope, system_wide_only, verify)` is a pure function tested for systemd, launchd and BusyBox user scope without probing the host.
- **Install execution:** `install_steps(native, notes, role, binary, materialize, register, release_after_direct)` takes its effects as closures (those needing the runner receive `&mut Native`), so ordering and `applied` contents are tested without touching `/etc`.
- **Process-global state:** the umask test and any test needing a fresh `HOME` run in a child process through `fixtures::in_isolated_home`.
- **Run path:** "no self-check at service start" is proven at process level in the privileged E2E: start a legacy init script directly, as boot does.

## Risks / Trade-offs

- [macOS has no `/proc/self/exe`; a user able to write the source could swap it between exec and copy] → An attacker with that ability could equally have swapped it before the administrator ran it. On Linux the copy comes from the executing inode. The note names the source.
- [Upgrades via `brew upgrade` or `cargo install` do not reach the service until `service install --system` is re-run] → Accepted by the reviewer. The install note says so, and docs/services.md documents it.
- [Legacy registrations with a user-writable binary keep running and starting at boot] → Accepted: nothing stops on its own. CLI start/restart refuse with reinstall guidance; CHANGELOG **Breaking** note.
- [Parsing manager output (`systemctl show`, `launchctl print`) can change across versions] → Pure parsers tested on captured output. Unparseable output from a loaded unit fails closed (exit 3) with reinstall guidance and never starts.
- [Reference check by substring] → It errs towards keeping a copy: any file mentioning the role tail, or an unreadable one, keeps it.
- [A distro or user making `/var/lib` or `/Library/Application Support` group-writable] → Install fails closed (exit 3) naming the directory.

## Migration Plan

No data migration. Existing system services keep working. On the next CLI start/restart, one with a user-writable binary is refused until `sudo iroh-tunnel <role> service install --system` is re-run, which writes the managed copy and re-registers. Rollback: an older binary ignores the managed directory. A re-install with it registers the resolved binary again, and leftover copies are harmless.

## Open Questions

None blocking. `service status --system` showing the registered binary is left out of scope.
