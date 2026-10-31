# Issue #93 — shell installer (Linux slice)

Baseline `907ac4c` (origin/main). Branch `redesign/issue-93`. Blocker #82 closed.

## Scope decision

Decided with the repository owner at pickup:

- **This PR delivers the Linux shell installer only.** #93 stays open.
- The PowerShell installer and macOS installer wait for #124, which builds the
  three role binaries for Windows x64/ARM64 and macOS arm64. Native blocker
  #124 was added to #93 and #100.
- Aliases: symlinks on Unix. Windows does not need aliases (owner decision).
- Verification: SHA-256 only. Signature verification waits for #100 / #80. This
  is a known gap against DIST-02 / AC-DIST-03.
- The installer does not register or change a service (93.CL2 not triggered; no
  #79/#91 policy is implied).

## Acceptance map

| Criterion | Evidence | Status |
|---|---|---|
| 93.AC1 default system-wide, all three | `default_flow_…`, `dry_run_defaults_…` | Linux PASS; Windows/macOS BLOCKED on #124 |
| 93.AC2 PATH and conflicts need confirmation | `declining_*`, `accepting_*`, `without_a_terminal_…` | Linux PASS |
| 93.CL1 default selection, decline, aliases → intended binaries | `default_flow_…` compares installed digests with archive members and runs every alias | Linux PASS; PowerShell not run |
| 93.CL2 service policy | `installer_never_touches_services` | N/A by design |

## Test seam

`tests/installer.rs` runs the real `install.sh` under `sh`. Real tar.gz
archives of the cargo-built executables and a real `checksums.txt` are served
over real HTTP by a local server (`IROH_TUNNEL_BASE_URL`). Prompt answers use
`IROH_TUNNEL_PROMPT_INPUT`, the same read path as `/dev/tty`.

## Evidence

- Red: before `install.sh` existed the suite could not run (script absent).
- Green: 18/18 pass (`cargo test --test installer`), `shellcheck` clean.
- Mutation check (test sensitivity): skipping the checksum comparison fails
  `checksum_mismatch_…`; making `confirm` always yes fails 3 decline tests;
  removing the conflict prompt fails `declining_a_file_conflict_…`.
- `cat install.sh | sh -s -- --help` works (the `curl | sh` path). All work is in
  `main`, called on the last line, so a truncated download is a syntax error and
  does nothing.
- Review (Standards + Spec, head `4cfdc85`): fixed a symlink-to-directory alias that
  was silently not replaced, partial install on a mid-loop failure (now staged,
  then renamed), leftover staging files, unvalidated `--version`/`--bin-dir`, a
  false test comment and a `$USER`-based root check. Each has a test. Left as
  noted: roles are checked with `--help`, not forwarded bytes (role forwarding is
  covered by `tests/role_binaries.rs` and #97); the final rename loop is not
  atomic across all files if the process is killed mid-loop (DIST-03 defers
  partial failures).

## Limits

- Not run: real `/usr/local/bin` with sudo, a real `/dev/tty`, macOS, Windows,
  musl/arm64 hosts. The test host is macOS; archives are named for
  `linux-amd64` via `IROH_TUNNEL_TARGET` and contain host-built executables.
- Not run against a published release: v0.4.2 has no role archives. The first
  release from this tree is needed for a true end-to-end run (#97).
- Checksum and archive come from the same host: this detects corruption, not a
  compromised host.

# Update — PowerShell and macOS installers

Baseline `c137636` (origin/main, after #124). Branch `redesign/issue-93-ps`. Native
blockers #82 and #124 are closed; their outputs are located in `main`:
`build.yml` publishes `<role>-<version>-windows-{amd64,arm64}.zip` (listed in
`checksums.txt`) and `<role>-<version>-darwin-arm64.tar.gz` (listed in
`checksums-darwin.txt`), see `release.yml` "Collect assets and checksums".

## Change

- `install.sh`: macOS Apple silicon (`darwin-arm64`). Checksums come from
  `checksums-darwin.txt`; the PATH entry is `/etc/paths.d/iroh-tunnel` (one
  directory per line). Intel macOS is rejected with guidance (no archive is
  published). Linux behavior is unchanged.
- `install.ps1`: Windows x64/ARM64. Default `%ProgramFiles%\iroh-tunnel`, all
  three `.exe`, no aliases (owner decision). SHA-256 from `checksums.txt`,
  staged then renamed, machine PATH appended only after confirmation, conflict
  prompt before any change, no-answer means no, never touches services.
- `tests/installer_ps.rs` (new) and macOS cases in `tests/installer.rs`; the
  local release server moved to `tests/common/release_server.rs`.

## Acceptance map

| Criterion | Evidence | Status |
|---|---|---|
| 93.AC1 default system-wide, all three | `default_flow_installs_all_three_executables`, `dry_run_defaults_to_a_system_wide_directory_and_all_roles`, `macos_default_flow_…` | Linux, macOS (host), PowerShell on pwsh/macOS PASS; real Windows run is CI only |
| 93.AC2 PATH and conflicts need confirmation | `declining_*`, `accepting_*`, `without_a_terminal_…` (PowerShell and macOS variants) | PASS as above |
| 93.CL1 decline/accept, artifacts equal intended binaries | installed digests compared with archive members; exact file list (no aliases/staging) | PASS |
| 93.CL2 service policy | `installer_never_touches_services` (both scripts) | N/A by design |

## Test seam

Real scripts, real `.zip` (`Compress-Archive`, as `build.yml`) and `.tar.gz`
archives, real HTTP from a local server. Prompts via
`IROH_TUNNEL_PROMPT_INPUT`. On a non-Windows host the machine PATH is a file
(`IROH_TUNNEL_PATH_FILE`); on Windows CI `registry_machine_path_is_changed_only_when_accepted`
writes and restores the real machine PATH.

## Evidence

- Red: `cargo test --test installer macos` 3 failed (404 on `checksums.txt`);
  `cargo test --test installer_ps` 12 of 14 failed before `install.ps1` existed.
- Green: `installer` 22/22, `installer_ps` 14/14 (macOS arm64 host, pwsh 7.6),
  full `cargo test --all-features --locked` clean, `shellcheck`, `clippy -D warnings`.
- Mutation: skipping the checksum compare fails `checksum_mismatch_…`; always
  answering yes fails 3 decline tests.

## Limits

- Not run by me: a real Windows host (registry PATH, running-executable
  replacement, `Program Files` ACLs, Windows PowerShell 5.1), Intel/ARM Windows,
  real macOS `/etc/paths.d` with sudo, a published release with role archives.
  The Windows CI job runs the registry test; pwsh/ubuntu CI runs the rest.
- Checksums only; signatures wait for #100/#80 (DIST-02 gap).
- Role bytes are not forwarded through installed binaries (#97).

## Review (Standards + Spec, base `c137636`, head `f5b5f7c`)

Fixed, each covered by the suite or a manual check:

- P1 machine PATH: `install.ps1` now reads and writes
  `HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment` with
  `DoNotExpandEnvironmentNames` and `REG_EXPAND_SZ`, so `%SystemRoot%` entries
  survive. The Windows-only registry test now asserts the exact value
  (`original;<dir>`) and restores the PATH through a drop guard. Not run on a
  Windows host by me; first run is the Windows CI job.
- A failed PATH write after install prints guidance instead of a misleading error.
- Architecture comes from `PROCESSOR_ARCHITECTURE`/`PROCESSOR_ARCHITEW6432`
  (no .NET 4.7.1 dependency); `-DryRun` checked for x64 host fallback and
  `IROH_TUNNEL_TARGET=windows-arm64`.
- `--profile-dir` is validated like `--bin-dir` (a path with a space could split
  the unquoted `rm -f` in cleanup); tests added.
- Two PowerShell prompt-absence assertions checked stdout and could never fail;
  they now check all output.
- `ci.yml` code filter includes `install.ps1`.
- Test hygiene: `run_darwin` duplicate removed (`Env.target`), stale header
  updated, release server has a read timeout and rejects `..`.

Not changed, disclosed: replacement is rename file by file, so a mid-loop
failure (e.g. a running `.exe` on Windows) can leave a mixed install; the
header of `install.ps1` now says so. Test-seam env vars
(`IROH_TUNNEL_PATH_FILE`, `IROH_TUNNEL_PROMPT_INPUT`) remain in the production
scripts and are documented. Platform detection (`uname`, Intel rejection) and
the default `/etc/paths.d` are not exercised by tests; Windows PowerShell 5.1 is
not tested (pwsh 7 only). #93 stays open until a real Windows run and
signatures (#100/#80).

## CI on head `ad4645a` (PR #129, run 41196340659)

All jobs pass (`lint`, `scripts`, `test (ubuntu-latest)`, `test (windows-latest)`, `ci-ok`).
`windows-latest` ran `installer_ps` 15/15 with PowerShell 7 on a real Windows
host, including `registry_machine_path_is_changed_only_when_accepted`: decline
leaves the machine PATH unchanged, accept yields `<original>;<dir>` with
unexpanded entries kept, and the guard restored the PATH afterwards. This closes
the "real Windows registry" gap for x64 under pwsh 7. Still not covered: Windows
ARM64, Windows PowerShell 5.1, Windows file replacement of a running executable.
