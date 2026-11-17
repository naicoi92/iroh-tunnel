## 1. Fail-closed machine directory resolution

Blocked by: none

- [x] 1.1 Add platform-neutral `machine_dir_from_program_data` (replacing `windows_machine_dir`) with unit tests: `Ok(path)` → `Some(MachineDir{path/iroh-tunnel, verify: true})`; `Err` → error mentioning `%ProgramData%` (scenario "Windows ProgramData cannot be resolved") — `crates/host/src/ownership.rs`
- [x] 1.2 Change `machine_dir()` to `Result<Option<MachineDir>>` on all three cfg variants; Windows production delegates to 1.1 — `crates/host/src/ownership.rs`
- [x] 1.3 Introduce `TierDirs` resolved before any lock is taken; `claim` → `claim_in(role, TierDirs)`, `probe_tiers` → `probe_tiers_in(role, &TierDirs)`; `RoleClaim` keeps its `TierDirs`; `LockDirs::resolve` propagates the error. seams `TierDirs::resolve_with(machine)` and `LockDirs::resolve_with(home, machine)`. Unit tests: a failing resolver makes `TierDirs` resolution (claim/probe/status path) and `LockDirs` resolution (install path) return the error; that claim and probe take no lock on this error is structural and checked in code review (scenarios "Windows ProgramData cannot be resolved", "Install lock directories when ProgramData cannot be resolved") — `crates/host/src/ownership.rs`
- [x] 1.4 Update callers: `service/install.rs` propagates with `?`; `service/log_file.rs::machine_log_dir` stays best-effort (`Err` → `None`, documented) — `crates/host/src/service/install.rs`, `crates/host/src/service/log_file.rs`
- [x] 1.5 Preserve "No machine lock file" (non-elevated, missing file → user tier only): existing tests stay green

## 2. Mandatory, fail-closed owner records and identity check

Blocked by: 1

- [x] 2.1 `write_owner` returns `Result<()>` with path context; machine-tier record mode `0o644` on Unix independent of umask, user tier `0o600`; tests: write failure (missing directory) names the path; machine record readable under umask 077 (scenarios "Owner record cannot be written", "Machine record readable under a restrictive umask", "Owner record round trip") — `crates/host/src/ownership.rs`
- [x] 2.2 `claim_in` and `publish_identity` propagate publish failures — `crates/host/src/ownership.rs`
- [x] 2.3 Tri-state `read_owner_record` (`Absent` / `Present` / `Unreadable`); `read_owner` stays the informational `Option` view — `crates/host/src/ownership.rs`
- [x] 2.4 `publish_identity` checks each tier where the other role's lock is held exclusively (`held_exclusively`): corrupt or unreadable record → `CliError::Conflict` naming the path; absent or no node id → proceed; lock free or machine lock held only shared → record ignored. Unit tests for scenarios "Running other role with an unreadable owner record", "Running other role that has not published yet", "Corrupt record of a role that is not running", "Corrupt machine record while the machine lock is held only shared", plus same-key conflict — `crates/host/src/ownership.rs`
- [x] 2.5 Process-level test: serve running, `serve.owner.json` corrupted, access started → exit 6, stderr names the record path (scenario "Running other role with a corrupt owner record"); existing "Access started with the serve key" stays green — `tests/role_ownership.rs`

## 3. Verification

Blocked by: 2

- [x] 3.1 `cargo fmt --check`; `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`; `cargo test --all-features --locked` (with `RUBY=/opt/homebrew/Library/Homebrew/vendor/portable-ruby/current/bin/ruby`)
- [x] 3.2 Check new production code against the no-panic policy (#216) and update the module docs in `ownership.rs` (owner records are no longer best-effort)
