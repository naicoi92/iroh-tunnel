## Context

Paths, ownership and service management live in `iroh-tunnel-host`:
`crates/host/src/paths` (default config/state, `*_in(home)` helpers),
`ownership.rs` (user-tier lock in the default state dir, machine lock in
`/run`, `/var/run` or `%ProgramData%\iroh-tunnel`), and `service/*` (scope
decision, invoking-user resolution in `system.rs` / `system_windows.rs`, owner
pin in `pin.rs`, privileged trust in `trust.rs` / `trust_windows`, backends).
The CLI (`src/cli.rs`, `src/cli_service.rs`) parses `--user` / `--system`,
prompts for scope, username and retarget, and renders guidance.

`is_elevated()` already answers "root or elevated Administrator" on every
platform, and the service verbs other than `install` already default their
scope from it. The machine directory, its protected Windows DACL, the
root-only binary trust walk and the managed binary copy already exist and are
reused.

The bounded context is host-local process administration: the ubiquitous
terms are Role, Tier (user or system), Config, State directory, Owner record
and Registration. The invariants this change must keep are: at most one
process per role per tier; a system-tier process only acts on files that no
non-admin principal can change; secrets never appear in owner records,
notices or status output.

## Goals / Non-Goals

**Goals:**
- One `Tier` value, derived once from privilege, selects config path, state
  directory, lock directory and service backend scope for every command.
- System-tier files live in machine-wide directories and are trusted only when
  owned by root (Unix) or SYSTEM/Administrators/TrustedInstaller (Windows).
- The two tiers of one role run side by side; each tells the operator when
  the other is running.
- Delete the account-coupling code paths rather than leave them unused.

**Non-Goals:**
- Migration of existing system configs, linger handling, a BusyBox user tier,
  splitting the secret key into its own file, port-clash detection.
- Changes to the user-tier service backends (systemd user manager, LaunchAgent
  and logon task already live with the login session).

## Decisions

### D1. One `Tier` type, derived from privilege at the edge

Add `crates/host/src/tier.rs` with a public `Tier { User, System }`,
`Tier::from_elevated(bool)` and `Tier::current()`. It replaces both
`service::ServiceScope` and `ownership::Tier` (whose `Machine` variant becomes
`System`). The tier is a fact of the running process, so the existing
process-scoped entry points (`default_config_path(role)`,
`default_state_dir()`, `resolve_config_path`, `claim`, `probe`,
`read_role_status`, the runtime) keep their signatures and resolve
`Tier::current()` themselves; explicit `*_for(…, tier)` variants exist for the
service layer, which derives its tier from the injected `Environment`.
Alternative considered: thread a `tier` parameter through every config,
runtime and status signature. Rejected: it changes dozens of signatures for a
value that cannot differ within one process, and it breaks compile order
between independent work units.

Host service operations keep taking the injected `Environment { elevated }`
(already used by the scope policy) and derive the tier from it instead of
taking a scope, so the "no fallback" rule cannot be bypassed by a caller
passing `System` without privilege. Alternative considered: keep a scope
parameter and validate it against privilege. Rejected: it reintroduces the
second source of truth the change removes.

Test builds (`test-utils` only) may force the tier with
`IROH_TUNNEL_TEST_TIER=system|user`, so non-root tests exercise system-tier
path, lock and notice logic against temporary directories. Production builds
ignore the variable.

### D2. Tier-aware path resolution in `paths`

`default_config_path_for(role, tier)` and `default_state_dir_for(tier)` (the existing `default_config_path(role)` / `default_state_dir()` delegate with `Tier::current()`):

| Tier | Linux | macOS | Windows |
|---|---|---|---|
| User config | `~/.config/iroh-tunnel/<role>.toml` | same | `%LOCALAPPDATA%\iroh-tunnel\config\<role>.toml` |
| User state | `~/.local/state/iroh-tunnel` | same | `%LOCALAPPDATA%\iroh-tunnel\state` |
| System config | `/etc/iroh-tunnel/<role>.toml` | `/Library/Application Support/iroh-tunnel/config/<role>.toml` | `%ProgramData%\iroh-tunnel\config\<role>.toml` |
| System state | `/run/iroh-tunnel` | `/Library/Application Support/iroh-tunnel/state` | `%ProgramData%\iroh-tunnel\state` |

On Linux, the system state directory is the existing machine lock directory
(`/run`, volatile). On macOS it is `/Library/Application Support/iroh-tunnel/state`
(decided during apply, Task 4): `/private/var/run` is `drwxrwxr-x root:daemon`,
group-writable without sticky bit, so the system-tier trust walk refuses any
child of it. Locks stay in `/var/run/iroh-tunnel` because lock files are not
trust-walked. The macOS state dir survives reboots; a stale snapshot is
already classified by pid/freshness. `/var/lib/iroh-tunnel`
keeps only the managed binary copies.
The system tier never reads `HOME`, `SUDO_USER` or the token user's profile;
the macOS legacy config migration runs for the user tier only.
`config_path_in` / `state_dir_in` / `windows_*_path_in` are deleted together
with their last callers (`service/system*.rs`, `LockDirs::resolve(home)`).
System installs pin the canonical form of both paths, as installs do today,
so the no-symlink trust walk accepts the defaults.
`IROH_TUNNEL_TEST_CONFIG_DIR` keeps overriding the config directory in test
builds for both tiers.

### D3. System-tier directory creation and trust

- Unix: an elevated process creates `/etc/iroh-tunnel` (or the macOS config
  directory) with mode 0755 owned by root on first write; config files stay
  0600 through the existing store. Nothing is chmod/chown-ed when it already
  exists.
- Windows: `%ProgramData%\iroh-tunnel` keeps its existing protected DACL
  (SYSTEM and Administrators full, Users read) because non-elevated processes
  must open lock and owner files there. The `config` subdirectory is created
  with its own protected DACL granting only SYSTEM and Administrators, because
  configs hold secret keys. An existing `config` directory or file whose DACL
  grants read to anyone else is refused before a secret is written or served
  (a confidentiality check added next to the existing write-rights check,
  applied to the config path only). The `state` subdirectory inherits the
  parent.
- The trust walk (`trust.rs` `TrustedOwners::RootOnly`; Windows
  `TrustedSet::machine()`) is applied to the config file and state directory
  at system install and at every start under a service manager, i.e. when the
  process is in the system tier **and** carries the service marker. A user
  service carries the marker too but runs unelevated, so it is never walked.
  Test builds (`test-utils`) may add one trusted uid/SID through
  `IROH_TUNNEL_TEST_TRUSTED_OWNER`, so trust success paths stay testable on
  non-root hosts; production builds trust admin principals only.
- Install creates a missing system state directory first (Unix 0755 root;
  Windows `state` under the machine directory), then canonicalizes, then
  verifies. Existing directories are never re-moded.
- Every elevated config write goes through the store's lock boundary
  (`ConfigLock::acquire`), which on Windows verifies the config directory's
  confidentiality before creating the lock, temp or config file when the
  target lies in the system-tier config directory (the spec's scope; an
  elevated write elsewhere, e.g. an explicit `--config` in a temp dir, is not
  confidentiality-checked). A foreground
  elevated run is not walked: root picks its own files, and container images
  bind-mount configs owned by host uids. `TrustedOwners::RootAnd` and
  `TrustedSet::for_user_files` are deleted.
- "Under a service manager" is detected from an explicit marker the
  registration sets: `IROH_TUNNEL_SERVICE=1` on Unix (systemd, launchd and
  BusyBox alike; BusyBox sets no manager variable of its own) and the existing
  `IROH_TUNNEL_SCM=1` on Windows.
- A system-tier `run` with an empty `secret_key` persists the generated key
  through the normal store, as the user tier does. The old refusal existed
  only because the file belonged to a user.

### D4. Legacy registrations fail with guidance

A process started with `IROH_TUNNEL_TRUSTED_UID` or `IROH_TUNNEL_TRUSTED_SID`
in its environment comes from a pre-change system registration that pins a
user's file. `run` refuses it before reading any config, with a configuration
error (exit 2) that names the expected system config path from D2 and says to
create it and re-run `service install` from an elevated shell. Alternative
considered: honouring the pinned path once. Rejected by the owner (no
migration). An even older registration with neither variable is recognised
as today: a root process under systemd (`INVOCATION_ID`) or launchd
(`XPC_SERVICE_NAME`) without `IROH_TUNNEL_SERVICE=1`. It gets the same
configuration error. BusyBox scripts that old cannot be told apart from a
foreground root run and are left to the D3 rules of a foreground run.
An empty or `0` `XPC_SERVICE_NAME` is not launchd: macOS Terminal exports
`XPC_SERVICE_NAME=0`, which `sudo -E` keeps. Caveat: an `INVOCATION_ID`
inherited from a terminal that systemd started (e.g. GNOME Terminal) through
`sudo -E` cannot be told apart from a systemd unit, so such a foreground run
is refused; the troubleshooting guide says to use `sudo` without `-E`.

### D5. Ownership: per-tier locks plus a presence lock

```
user dir   (~/.local/state/iroh-tunnel | %LOCALAPPDATA%\...\state)
  <role>.lock          user process: exclusive
  <role>.owner.json    0600
machine dir (/run/iroh-tunnel | /var/run/iroh-tunnel | %ProgramData%\iroh-tunnel)
  <role>.claim.lock    system process: exclusive     (0600 / admin-only DACL)
  <role>.lock          system process: shared        (0644, detection only)
  <role>.owner.json    system process record         (0644)
  <role>.user.lock     user processes: shared        (0644, created by system)
```

- A user process claims its user-tier lock exclusively (one per user per
  role), then opens `<role>.user.lock` read-only and holds it shared when the
  file exists. It never creates machine-tier files.
- A system process claims `<role>.claim.lock` exclusively (one per machine
  per role; admin-only so no non-admin can block the start — decided in the
  final review: a world-readable exclusive lock let any user hold it shared
  and make the service exit 6 forever under `RestartPreventExitStatus=6`),
  then holds `<role>.lock` shared for detection, and creates `<role>.user.lock` if missing, without locking it; a
  system `service install` creates it too, so a user instance started after
  the install but before the first service start is visible. Lock and
  presence files are created 0644 with the mode applied on the open file, so
  the umask cannot hide them.
- Upgrade transition (decided after the final review): an earlier-version
  system process holds `<role>.lock` exclusively and knows no claim lock. A
  system claim, `probe_tier(System)` and `wait_released_in(System)` treat
  "`<role>.lock` held exclusively AND `<role>.owner.json` owned by
  root/Administrators names a live pid" as a running system instance (claim
  → exit 6 "from an earlier version … stop it first"). A non-admin cannot
  forge that record, so the DoS fix holds. Residual: a `kill -9`-left record
  whose pid was reused while an attacker holds the lock; `/run` clears on
  reboot.
- Other-tier detection: a user process tests `<role>.lock` with an exclusive
  lock (busy means a system instance runs; a malicious shared holder can only
  cause a false notice); a system process tests
  `<role>.user.lock` with an exclusive lock (busy means at least one user
  instance runs). Neither test blocks the start.
- Known limitation: a user instance started before the presence file ever
  existed on this boot is not seen by a later system start. The presence file
  appears with the first system-tier run or install on the boot (the Unix
  machine directory is on a volatile filesystem).
- The same-identity check (`publish_identity`) inspects only the claimant's
  own tier directory.
- `probe(role)` and `wait_released(role, timeout)` look only at the current
  tier's own lock (`probe_tier`/`wait_released_in` take an explicit tier for
  the service layer). So `status` pairs the tier's own snapshot with its own
  owner, and a Windows system restart waits for the system process, never a
  user holder.

Alternative considered: keep one machine lock that user processes hold shared
and only downgrade the refusal to a notice. Rejected: the exclusive system
claim would then fail whenever a user instance runs, so the system tier could
not start at all.

### D6. Other-tier notice

`ownership` exposes `other_tier_running(role, tier) -> Option<Tier>`: any
I/O error while probing the other tier is logged at debug and returns `None`.
This is distinct from the claimant's own tier: failing to resolve or lock the
own tier's directory stays an error (a system-tier claim without a resolvable
machine directory fails; a user-tier claim never needs it).
The CLI prints one stderr line, before any bind or service-manager call, in:
`run` (configured and quick), `status`, and `service install|start|restart|status`:

- user tier: `note: <role> is also running as a system service; manage it with sudo / an elevated terminal`
- system tier: `note: <role> is also running in a user session`

The notice never names another account and never reads another tier's owner
record. It is also returned to non-terminal callers as a
`ServiceNote::OtherTierRunning` on service operations, so frontends get it
without parsing text.

### D7. Service layer simplification

- `install` no longer takes `scope`, `sudo_user`, `invoking_user` or
  `retarget`; `InstallOutcome::Needs` and `InstallNeed` are removed (no need
  remains), so `install` returns `Result<ServiceReport, ServiceFailure>`.
  `Pin`, `PinOwner`, `RetargetConsent`, `decide_install_scope`,
  `default_scope`, `parse_scope_answer`, `ScopeDecision`, `system.rs`,
  `system_windows.rs`, `account.rs` and `account_name.rs` are deleted.
- Cross-tier install refusals (`plan_install` pre-checks against the other
  tier, Windows cross-scope registration) are removed. Same-tier behaviour
  (start refused while the tier's lock is held, re-install over its own
  running service) is unchanged.
- Registrations stop writing the pin comment and the trust env, and start
  writing `IROH_TUNNEL_SERVICE=1` (Unix) next to `IROH_TUNNEL_STATE_DIR`.
- BusyBox: a non-elevated mutating verb fails with the existing permission
  error ("this host has no per-user services; re-run with sudo"). The
  user-scope-as-system-script note and its tier comparison are removed.
- Binary trust (`effective_binary`, managed copies) is unchanged apart from
  guidance text naming `sudo iroh-tunnel <role> service install` without
  `--system`.

### D8. CLI surface

- `ScopeArgs` and every `--user` / `--system` flag are removed from `cli.rs`;
  clap rejects them as unknown arguments.
- The scope prompt, username prompt and retarget prompt in `cli_service.rs`
  are removed.
- `render.rs` and every guidance string drop `--system`/`--user`; elevated
  guidance is `sudo <command>` on Unix and "from an elevated terminal (Run as
  administrator)" on Windows.
- `update` guidance on Windows names `iroh-tunnel serve service stop` and
  `iroh-tunnel access service stop`, run from an elevated terminal.

## Risks / Trade-offs

- [Existing system services stop after upgrade] → D4 error names the new path
  and the rerun; CHANGELOG has a Breaking entry with the manual steps.
- [Scripts passing `--user` / `--system` break] → owner-accepted; CHANGELOG
  lists it.
- [An administrator who wants their user tier must run unelevated] → stated in
  README; `sudo -u <user>` also works on Unix.
- [Port clash between tiers] → the other-tier notice is printed before the
  bind, and the bind error names the address.
- [Presence lock blind spot (D5)] → documented; the notice is informational
  only, so missing it costs no safety.
- [Windows `%ProgramData%\iroh-tunnel\config` created by a non-admin before the
  first elevated run] → the D3 trust walk refuses it, naming the directory and
  owner, and never repairs it (same rule as the machine directory).
- [Unix system config dir pre-created with a loose mode] → refused by the trust
  walk at install and at service start, with path, owner and mode.

## Migration Plan

No automatic migration. Operators with a system service:
1. Upgrade the binary.
2. Create the system config with elevation, for example by running
   `sudo iroh-tunnel serve config keygen` and `sudo iroh-tunnel serve config set ...`,
   or by copying the old file to `/etc/iroh-tunnel/serve.toml` with mode 0600
   and owner root. A copied file keeps its key; the old user file must then not
   be run in the user tier with the same key.
3. Re-run `sudo iroh-tunnel <role> service install`.

Rollback: reinstall the previous release and re-run its `service install
--system`; the old user files were never touched.

## Open Questions

None blocking. Notice wording may be polished during implementation without
changing when it is printed.
