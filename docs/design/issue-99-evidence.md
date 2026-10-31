# Issue #99 — configuration and identity lifecycle evidence

Implements #98 D1–D5 (D6 and AC-CFG-12 shipped in #130). Base `origin/main` 92a3551. Platform: macOS (Darwin 24.6, arm64), debug build, Rust 1.99.1. Linux runs in CI. **Windows is not run: owner-only ACL and lock behavior are UNVERIFIED** (RS-13, #97 rule).

## Traceability

| Decision | Behavior | Code | Test (`tests/config_lifecycle.rs` unless noted) |
|---|---|---|---|
| D1 / AC-CFG-07a | `keygen` asks before replacing a key; `--yes`; non-TTY without `--yes` fails; first key no prompt | `config_cmd::keygen`, `confirm_rotation` | `keygen_rotation_needs_confirmation`, `first_keygen_does_not_prompt` |
| D2 / AC-CFG-07b | `show` redacts `secret_key`, `relay_token`; `--reveal` | `config_cmd::show`, `redact_secrets` | `show_redacts_secrets_unless_revealed` |
| D3 / AC-CFG-08 | `run`/`list`/`show` without config: one guidance error, exit 2, nothing created | `config_cmd::require_existing`, `app.rs` | `read_commands_without_config_share_one_guidance_error_and_create_nothing` |
| D4 / AC-CFG-05 | Every mutation validates; file bytes unchanged on failure | `config_cmd::mutate`, `edit` | `invalid_mutations_fail_and_leave_file_bytes_unchanged`, `rejected_edit_keeps_prior_file_and_a_private_sidecar` |
| D4 / AC-CFG-06 | Concurrent writers serialize | `config_store::ConfigLock` | `concurrent_adds_serialize_without_losing_updates` (8 processes), `lock_held_by_another_writer_times_out_with_a_clear_error`, `edit_conflicting_with_a_concurrent_change_is_rejected` |
| D4 / AC-CFG-10 | Atomic save (temp, fsync, rename, dir fsync) | `config_store::write_atomic` | `killed_or_concurrent_saves_never_expose_a_truncated_file` (25 SIGKILLs, concurrent reader) |
| D4 / AC-CFG-11 | `--config` on every subcommand | `cli.rs`, `app::dispatch_config` | `every_config_subcommand_honors_config_and_ignores_the_default` |
| D5 / AC-CFG-09 | `run`-generated key persisted `0600`, reused on restart | `config_store::persist_generated_key` | `run_persists_a_generated_key_owner_only_and_reuses_it` |

## Open items resolved here

- **Lock mechanism:** `std::fs::File::try_lock` (`flock` on Unix, `LockFileEx` on Windows) on sibling `<config>.lock`, polled every 25 ms for up to 10 s, then `another process is writing …`. Test override `IROH_TUNNEL_TEST_CONFIG_LOCK_TIMEOUT_MS` exists only with `test-utils`. Windows: UNVERIFIED.
- **Sidecar:** a rejected `edit` keeps its text in `<config>.rejected` (`0600`); the config is untouched. `edit` works on a private copy and applies only if the live file is unchanged since the copy (otherwise it reports a conflict and keeps the text), so an editor session cannot overwrite a concurrent change.
- **Windows permissions:** not enforced; the temp file inherits the directory ACL. Owner-only ACL parity needs Windows evidence and stays open (service contexts: #79).

## Intentional behavior changes (AC3)

`keygen` over an existing key needs `--yes` or a terminal; `show` redacts; `list` without config exits 2 (was 0); `run` without config says "no configuration at …" (was "failed to read config … No such file"); `keygen`/`remove`/`edit` now validate; `edit` output is rewritten by the app (retired `multiplex` dropped, TOML re-rendered with comments kept); read-only config files are refused rather than overwritten. Existing tests were updated accordingly (`config_multiplex_compat.rs`: `--yes`, edit expectation; `config_paths.rs`: `show --reveal`, `keygen --yes`). Migration safeguards CFG-01–03 and AC-CFG-01–04 unchanged and green.

## Evidence

- **Before (red):** `cargo test --test config_lifecycle` on the unchanged code: 10 of 13 failed (`unexpected argument '--config'/'--yes'/'--reveal'`, `run` without config reads the file, 0644 key).
- **After:** `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features --locked -- -D warnings` clean; `cargo test --all-features --locked` all suites green (13 new in `config_lifecycle`).
- **Manual run, isolated HOME:** `list|show|run` with no config → identical error, exit 2, `$HOME` stays empty. `keygen` twice with `</dev/null` → second exits 2, key kept. `show` prints `secret_key = "<redacted>"`; `--reveal` prints it. Config created `600`. `run` on a `0644` config without key, killed with `kill -9` and restarted: mode `600`, identical key hash and identical `NodeId` on both runs.

## Limits

Single machine, macOS. No Windows, no Linux local run. The interactive prompt is tested via the `IROH_TUNNEL_TEST_INTERACTIVE` hook with piped stdin, not a pty. Crash atomicity is tested by killing the process around rename; power-loss durability is by construction (fsync file and directory), not observed.
