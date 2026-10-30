# Issue #84 — configuration paths and macOS identity migration

Base: `99a853f` (`main`, includes PR #112). This record covers the #84
component only, not issue closure or redesign release acceptance (#97).

## Decisions and scope

CFG-01 and CFG-02 are Confirmed in `redesign-spec.md`. First-time
missing-both-configs behavior and migration synchronization remain decision
gates #98/#99 and are not assumed. Native blocker #83 is closed.

PR #112 implemented defaults, overrides, error guidance and macOS migration
with Unix-only end-to-end tests. This follow-up adds the behavioral test its
description listed as missing: Windows LocalAppData.

## AC matrix

| Requirement / case | Test | Platform |
| --- | --- | --- |
| CFG-01, CFG-PATH-01 Unix defaults ignore XDG; overrides | `tests/config_paths.rs` | macOS, Linux |
| CFG-01, CFG-PATH-01 Windows `LocalAppData\iroh-tunnel\config` default, XDG ignored | `tests/config_paths_windows.rs::config_default_is_local_app_data_ignoring_xdg` | Windows CI |
| CFG-01 Windows `...\state` default | `state_default_is_local_app_data_state` | Windows CI |
| CFG-01 explicit `--config` / `IROH_TUNNEL_STATE_DIR` | `explicit_config_override_wins`, `explicit_state_override_wins` | Windows CI |
| CFG-02, CFG-MIGRATE-01, CFG-ERROR-01 | `tests/config_paths.rs::macos_migration` | macOS |

The Windows known folder cannot be redirected per process, so the Windows
tests are read-only (`config path`, `status`, and `keygen` into a temp file)
and compare against `dirs::data_local_dir()` for the installing user.

## Not verified

- Windows tests were not run locally (no Windows target installed); evidence is
  the Windows CI job on the pull request head.
- Windows invalid/unreadable-config guidance is not tested, because it would
  write to the real LocalAppData; Unix coverage exists in `config_paths.rs`.
- Legacy macOS path comes from `dirs::config_dir()` use in tags v0.1.0–v0.4.2,
  not from a real older macOS install.
- Known limits from PR #112 remain (legacy symlink/dir not validated, no
  `hard_link` fallback, migration also runs on read-only commands).
