# #91 evidence — system service across the privilege boundary

Head: this branch. Platform: Linux (Docker `rust:1.91-bookworm`, real root, BusyBox-init backend, no systemd).
Command: `docker run … -e IROH_TUNNEL_E2E_PRIVILEGED=1 rust:1.91-bookworm cargo test --all-features --locked --test service_system`

| Case | Observed (real root, real accounts `it91a`/`it91b`/`it91x`) |
| --- | --- |
| SM-UNKNOWN-01 | no `SUDO_USER`, no terminal: exit 2 with `sudo iroh-tunnel … --system` and `--config` guidance; nothing registered; root's home never used |
| SM-ELEVATED-01 | root, `HOME=/root`: system scope, no scope prompt; registration pins `it91a`'s config/state |
| SM-PATH-01 | registration carries the canonical absolute paths and `IROH_TUNNEL_TRUSTED_UID`; the service serves from them |
| SM-EDIT-01 | user edit + admin `service restart --system` applies; invalid TOML is reported and not served; owner/mode of the user's file unchanged |
| SM-RETARGET-01 | `n`/empty/no-terminal: exit 5, registration byte-identical, both configs unchanged; `y` retargets, both configs kept; same user again needs no prompt |
| SM-TRUST-01 | service refuses (exit 3, names path/owner/mode) for: 0666 file, file owned by another uid, 0777 on each parent, parent owned by another uid, file symlink, parent swapped for a symlink; runs again once fixed; no chown/chmod |

Mapping unit coverage: `src/service/trust.rs` (owner, mode, sticky, symlink, `..`), `pin.rs`, `system.rs`, `account.rs`.
Unprivileged system install/verbs: `tests/service_user.rs` (unchanged, passing).

## Limits

- Real runs: Linux/BusyBox only. systemd system scope and macOS LaunchDaemon share the code path (pin comment + `IROH_TUNNEL_TRUSTED_UID`) and are covered by template unit tests, not a real root run (no systemd container / macOS root available here). Requires follow-up evidence in #97.
- The check is a runtime walk at start/restart (no hot reload). A race between the check and the open is closed by `openat` + `O_NOFOLLOW` on verified handles; a rename of a *verified ancestor* mid-walk cannot redirect an already-opened handle. Not fuzzed under a concurrent attacker.
- State dir: only checked for safety and existence; the service does not create it as root. The status/lock writes inside it still use paths, not handles (remaining TOCTOU window for a *trusted* owner only).
- Registrations from before this change carry no pin: treated as "unknown owner" and confirmed before retarget.
- A system service no longer generates and writes a missing `secret_key` into the user's file (it would leave a root-owned file); it fails with `config keygen` guidance.
