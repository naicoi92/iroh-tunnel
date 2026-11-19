## Design Summary

### Intent

A system service today runs as root/LocalSystem but reads its config and
state from one user's home (`~/.config/iroh-tunnel`, `~/.local/state/iroh-tunnel`,
`LocalAppData\iroh-tunnel`). That forces the tool to work out *which* user owns
the files (SUDO_USER, `--config` owner, an interactive username prompt), pin
that user's UID/SID into the registration, and trust-check paths inside a home
directory it does not own. Separately, the operator must pick a scope with
`--user` / `--system` or answer a scope prompt.

The owner wants two clearly separated tiers whose choice follows only from how
the command is run, with no per-account handling for the system tier:

- **User tier**: a normal, non-admin user runs the tool. Files live in that
  user's home. A registered service runs only while the user is logged in:
  it stops at logout and starts again at the next login.
- **System tier**: the tool runs as root, through `sudo`, or (Windows) from an
  elevated "Run as administrator" process. Files live in machine-wide
  locations. A registered service runs from boot, whether or not anyone is
  logged in.

### Goals

- The tier is derived only from privilege: not root and not via sudo/elevation
  means user tier; root, sudo or elevated means system tier. This applies on
  Linux, macOS and Windows, to every command (`run`, `config *`, `status`,
  `service *`).
- System-tier config and state live in machine-wide locations:
  - Linux: config in `/etc/iroh-tunnel`; state under `/var/lib/iroh-tunnel`,
    with runtime files under `/run/iroh-tunnel`.
  - macOS: `/Library/Application Support/iroh-tunnel` (config and state
    subdirectories); runtime files under `/var/run/iroh-tunnel`.
  - Windows: `%ProgramData%\iroh-tunnel` (config and state subdirectories).
- User-tier locations stay where they are today.
- No username input, no scope prompt, and no `--user` / `--system` flags.
- The two tiers of one role may run at the same time. Each has its own config
  and therefore its own key and node id.
- When a command runs in one tier while the same role is running in the other
  tier, it prints a notice saying so (user command: "running as a system
  service"; system command: "a user-session instance is running").
- The system tier never reads or writes anything inside a user's home.

### Non-goals

- No migration of existing system installs (see Key Decisions).
- No handling of systemd linger. If a user enables linger, their user service
  outliving logout is their own decision.
- No user tier on BusyBox/SysV init hosts. A non-admin user there can only run
  roles in the foreground.
- No splitting of `secret_key` out of `<role>.toml` in this change.
- No detection of listener port clashes between the two tiers beyond a clear
  bind error.

### Anti-goals

- Root or an elevated process silently falling back to user-tier files, or a
  non-elevated process silently touching system-tier files.
- Self-elevation, or any prompt that asks which tier to use.
- A system service that still depends on a particular user's account or home.

### Existing requirements this change alters

Stated explicitly so none are re-decided silently:

- `tunnel-configuration`: *Default configuration directory per platform*,
  *Default state directory per platform* and *Unusable home directory is a
  config error with guidance* gain a system-tier branch; *Root service install
  uses the target user's home paths* is removed.
- `system-service-management`: *Install scope selection*, *Scope prompt never
  guesses*, *Default scope for verbs on an existing service* and *System scope
  requires privilege without elevation or fallback* are replaced by tier from
  privilege. *System install resolves the invoking user, never root's home*,
  *SUDO_USER and --config must agree*, *System install uses the user's default
  config and state locations*, *Registration pins the owner*, *Retargeting
  another user's service needs confirmation*, *Windows installing user*,
  *Windows token user and --config owner must agree*, *Windows retarget
  confirmation*, *Root under a service manager without a pin is refused* and
  *Windows cross-scope registration is refused* are removed or rewritten.
  *Privileged service refuses unsafe config paths* and *Privileged service
  never writes a generated key* are rewritten for machine-wide paths. *Start
  and conflicting install refused when the role already runs* no longer refuses
  across tiers. *BusyBox backend needs root and ignores user scope* changes
  because there is no user-scope request any more.
- `role-ownership-shutdown`: *Machine-tier lock between system service and
  user processes* changes from mutual exclusion to coexistence with a notice.
  *Same public identity rejected across roles* is limited to one tier. *Single
  running process per role* and *Ownership scope is the user's default state
  directory* get a per-tier reading.
- `windows-service-trust`: *Trusted principals for a Windows system service*
  drops the pinned user SID; *Never repair access on user files* no longer
  applies to system-tier files.
- `cli-entrypoints`: *Service scope selection and privilege* is replaced.
- `unix-service-binary-trust`: *BusyBox user-scope requests are root-run
  registrations* is removed.
- `frontend-independent-management`: *Service operations enforce existing
  policy*, *Insufficient information yields a structured need* and *Retarget
  confirmation is specific and fresh* change because scope and account are no
  longer inputs.

## Alternatives Considered

### Alternative A: Machine tier only
- **Approach**: Drop the user tier; every config, state and service is
  machine-wide.
- **Pros**: One set of paths; the smallest code base.
- **Cons**: Every use, including a foreground `access` on a laptop, needs
  admin rights.
- **Why not chosen**: The owner requires non-admin users to keep using the
  tool.

### Alternative B: Two tiers, scope still chosen by flags and prompt
- **Approach**: Move system files to machine-wide locations but keep
  `--user` / `--system`, the install prompt and per-verb defaults.
- **Pros**: No CLI break; scripts that pass the flags keep working.
- **Cons**: Keeps a second way to pick the tier that can disagree with
  privilege, plus the prompt and its error paths.
- **Why not chosen**: The owner wants the tier decided by root/sudo/elevation
  alone and accepts the breaking change.

### Alternative C: Two tiers, tier derived from privilege (chosen)
- **Approach**: As described in the summary.
- **Pros**: One rule on every platform and command; deletes the
  user-resolution, owner-pinning and cross-account trust machinery; system
  services no longer depend on an account.
- **Cons**: Breaking CLI change; existing system installs must be
  re-installed; an administrator who wants to manage their own user tier must
  run without elevation.

## Agreed Approach

Alternative C. It is the only option that keeps the tool usable without admin
rights while removing every system-tier dependency on a user's home and
account, which is the reason for the change.

## Key Decisions

- **Tier rule** (owner): not root and not sudo/elevated means user tier; root,
  sudo or an elevated Windows process means system tier. On Windows, a member
  of Administrators in a normal (non-elevated) PowerShell is user tier; "Run
  as administrator" is system tier.
- **Flags** (owner): `--user` and `--system` are removed outright, with no
  deprecation period; this is a documented breaking change.
- **Lifetime** (owner): the user tier lives with the login session (systemd
  user manager, LaunchAgent, logon task); the system tier lives from boot.
- **Linger** (owner): not detected, not warned about, not changed.
- **Coexistence** (owner): both tiers of one role may run at once; each tier
  prints a notice when the other tier's instance of the role is running.
- **Identity** (owner): separate config files mean separate keys, so the tiers
  do not conflict on node id. A same-key check between tiers is dropped.
- **No migration** (owner): an existing system service whose registration
  still points at a user's home is not migrated. After the update it fails
  with a config error that names the expected system config path
  (`/etc/iroh-tunnel/<role>.toml` or the platform equivalent) and tells the
  operator to create it and re-run `service install` with elevation.
- **Port clashes** (agreed recommendation): the tiers may clash on a local
  listener port. The bind error names the address, and the other-tier notice
  is printed before the bind so the cause is visible.
- **Machine-wide paths** (agreed recommendation): as listed under Goals.
- **Overrides** (my call): `--config` and `IROH_TUNNEL_STATE_DIR` stay
  available in both tiers.
- **Trust checks** (my call): the "owned by root/Administrators and not
  writable by anyone else" check on system-tier paths applies to a process
  started by the service manager, as the privileged checks do today. A
  foreground root run (for example a distroless container with a read-only
  bind-mounted config) is not checked, so the shipped container images and
  `docker-compose.yml` keep working. Containers run as root and so resolve to
  `/etc/iroh-tunnel/<role>.toml` by default, matching the compose demo.

## Open Questions

- Which notice text and stream (stderr) to use for the other-tier notice, and
  on which commands it appears beyond `run`, `status` and `service *`. Left to
  design.
- How the system tier detects "a user-session instance is running" without
  reading any user's home. Left to design (the existing shared machine lock
  held by user processes is the candidate).
- The exact way a legacy system registration is recognised. Left to design.
