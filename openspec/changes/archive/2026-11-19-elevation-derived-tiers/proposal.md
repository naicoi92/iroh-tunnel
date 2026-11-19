## Why

A system service runs as root/LocalSystem but reads one user's config and state from that user's home, so the tool must resolve the owning account (SUDO_USER, `--config` owner, a username prompt), pin its UID/SID and trust-check paths it does not own. Operators also pick a scope through `--user` / `--system` flags or a prompt. Deriving the tier from privilege and giving the system tier machine-wide files removes that account coupling and the scope choice, while non-admin users keep a login-session tier.

## What Changes

**Tier selection**
- From: `service` verbs take `--user` / `--system`; `install` prompts a non-elevated interactive user; other commands always use the invoking user's files.
- To: every command (`run`, quick mode, `config *`, `status`, `service *`) runs in the **system tier** when the process is root, under `sudo`, or an elevated Windows process ("Run as administrator"), and in the **user tier** otherwise. No flag, prompt or username input remains.
- Reason: one rule on every platform; no way for scope and privilege to disagree.
- Impact: **BREAKING** CLI change; `--user` / `--system` are removed with no deprecation period.

**System-tier locations**
- From: system services use the installing user's `~/.config/iroh-tunnel`, `~/.local/state/iroh-tunnel` or `LocalAppData\iroh-tunnel`.
- To: config in `/etc/iroh-tunnel` (Linux), `/Library/Application Support/iroh-tunnel/config` (macOS), `%ProgramData%\iroh-tunnel\config` (Windows); runtime state in `/run/iroh-tunnel`, `/Library/Application Support/iroh-tunnel/state`, `%ProgramData%\iroh-tunnel\state`. User-tier locations are unchanged.
- Impact: **BREAKING** for existing system services (see Migration below).

**Account coupling removed**
- Removed: invoking-user resolution, the `SUDO_USER`/`--config` owner agreement, the username prompt, the registration owner pin and `IROH_TUNNEL_TRUSTED_UID` / `IROH_TUNNEL_TRUSTED_SID`, retarget confirmation, explicit-home path helpers, and the Windows token-user/owner agreement.
- Added: system-tier files are trusted only when owned by root (Unix) or SYSTEM/Administrators/TrustedInstaller (Windows) and not writable by anyone else, checked at system install and at every service-manager start.

**Tiers coexist**
- From: a machine lock makes a system process and a user process of one role mutually exclusive, and one public identity is rejected across tiers.
- To: one process per role per tier; the two tiers of a role may run at once with their own keys. Each tier prints a notice when the other tier's instance of the role is running. The same-identity check applies within a tier.

**No migration**
- An existing system registration that still points at a user's files (recognised by its trust-pin environment) refuses to run with a config error naming the new system config path and the elevated `service install` rerun. Nothing is copied.

**Non-goals**: linger handling; a user tier on BusyBox/SysV hosts; splitting `secret_key` out of `<role>.toml`; detecting listener port clashes between tiers beyond a clear bind error.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `tunnel-configuration`: default config and state locations become tier-dependent; the explicit-home requirement is removed; the unusable-home error applies to the user tier only.
- `system-service-management`: scope selection, prompts, invoking-user resolution, owner pin, retarget and cross-scope refusals are replaced by tier-from-privilege; system installs use machine-wide paths with root-only trust; legacy registrations are refused with guidance; BusyBox needs root for every mutating verb.
- `role-ownership-shutdown`: one owner per role per tier; tiers coexist with a notice instead of mutual exclusion; the identity check is per tier.
- `windows-service-trust`: system-service config and state trust only SYSTEM, Administrators and TrustedInstaller; the pinned user SID is removed.
- `unix-service-binary-trust`: system-tier wording replaces `--system`; the BusyBox user-scope requirement is removed.
- `cli-entrypoints`: `--user` / `--system` are removed; service verbs follow the tier.
- `config-management-commands`: config commands resolve the tier's default file; system-tier config directories are created root-only.
- `frontend-independent-management`: host service operations derive the tier from privilege; scope, invoking-user and retarget needs are removed.
- `self-update`: the Windows locked-executable guidance names the elevated `service stop` commands without `--system`.
- `workspace-boundaries`: the exit-code scenario for a permission failure no longer uses `--system`.

## Impact

- **Code**: `crates/host/src/paths`, `ownership.rs`, `service/*` (install, system, system_windows, pin, trust, trust_windows, policy, platform, decide_install_scope, default_scope, parse_scope_answer, account, backend/*), `status/writer.rs`, `config/resolve_config_path.rs`; CLI `src/cli.rs`, `src/cli_service.rs`, `src/render.rs`, `src/status_cmd.rs`, `src/app.rs`.
- **Public host API**: `InstallRequest`, `InstallOutcome`, `InstallNeed`, `ServiceScope`, `Environment`, `ScopeDecision`, `Pin`, `RetargetConsent` and the scope parameter of the service verbs change or are removed.
- **Tests**: `tests/service_system.rs`, `tests/service_system_windows.rs`, `tests/service_user*.rs`, `tests/role_ownership.rs`, `tests/config_paths*.rs`, `tests/cli_golden.rs`, `tests/update_zip_windows.rs` and unit tests of the touched modules.
- **Docs**: README, CHANGELOG (breaking entry), docs/GLOSSARY.md (User tier, System tier).
- **Security**: system services no longer read user-writable files; Windows `%ProgramData%\iroh-tunnel\config` must not grant Users read access because configs hold secret keys.
- **Compatibility**: existing system services stop working until re-installed with elevation from a system-tier config; scripts passing `--user` / `--system` fail at argument parsing.
- **Dependencies**: none added.
