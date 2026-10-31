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

| RO-03 crash | real `kill -9` of the service leaves `serve-status.json` and a stray `.tmp`; the next restart starts cleanly and replaces the snapshot |
| Root write safety | a symlink planted at the status file name is replaced, never written through (victim file unchanged) |
| SM-04 canonical pin | a home that is a symlink is resolved once at install; the service runs from the resolved paths |

Mapping unit coverage: `src/service/trust.rs` (owner, mode, sticky, symlink, `..`), `pin.rs`, `system.rs`, `account.rs`.
Unprivileged system install/verbs: `tests/service_user.rs` (unchanged, passing).

## Independent review dispositions

Standards + Spec reviews run on `af6c10c`. Fixed: state dir canonicalized; root status writes via verified handle; pin parse accepts only comment lines (busybox `CONFIG=` spoof); registration written atomically 0644/0755; unreadable registration is an unknown owner, not "nothing installed"; null `pw_dir` check; crash-leftover test. Open/accepted: pre-pin BusyBox registrations are not detected (see Decisions); no file-size cap on the config read; macOS/systemd real-root evidence deferred to #97.

Second review pass (`e78ef44`): fixed RS-12 `SAFETY` comments on `renameat`/`unlinkat`, explicit `let _ =` on best-effort unlink, debug log when `read_in` refuses a directory, and a named conflict error when `SUDO_USER` and `--config` belong to different users (unit-tested). Left for a decision: whether "safety confirmation" (91.CL1) means only the retarget prompt; a `TrustedDir` type for the `(path, uid)` clump and splitting `trust.rs` (smells, judgement); `sudo` `env_reset` dropping `IROH_TUNNEL_STATE_DIR`; legacy registrations without a pin.

Decisions (per the redesign spec): (1) "safety confirmation" for an elevated install is the SM-02 retarget prompt; re-installing for the same user asks nothing, because idempotency is "proposed, not confirmed" (SM Operations) — no extra prompt added. (2) `TrustedDir` / splitting `trust.rs` is deferred to a follow-up (RS-01: no speculative generality; smell is judgement only). (3) SM-04 "refuse (fail closed)": root started by systemd (`INVOCATION_ID`) or launchd (`XPC_SERVICE_NAME`) with no `IROH_TUNNEL_TRUSTED_UID` is refused with re-install guidance (exit 3); a foreground `sudo … run` is unaffected. Real-root E2E asserts the refusal and that the live service is unaffected. BusyBox init sets no manager variable, so a pre-pin BusyBox script is not detected; it is only fixed by re-install.

## Limits

- Real runs: Linux/BusyBox only. systemd system scope and macOS LaunchDaemon share the code path (pin comment + `IROH_TUNNEL_TRUSTED_UID`) and are covered by template unit tests, not a real root run (no systemd container / macOS root available here). Requires follow-up evidence in #97.
- The check is a runtime walk at start/restart (no hot reload). A race between the check and the open is closed by `openat` + `O_NOFOLLOW` on verified handles; a rename of a *verified ancestor* mid-walk cannot redirect an already-opened handle. Not fuzzed under a concurrent attacker.
- State dir: must exist (the service never creates directories in a user's home as root). Status writes use verified handles; the role lock/owner record live outside the user's home (spec, SM-04 Implemented).
- Registrations from before this change carry no pin: treated as "unknown owner" and confirmed before retarget.
- A system service no longer generates and writes a missing `secret_key` into the user's file (it would leave a root-owned file); it fails with `config keygen` guidance.
