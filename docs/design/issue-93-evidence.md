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
- Green: 15/15 pass (`cargo test --test installer`), `shellcheck` clean.
- Mutation check (test sensitivity): skipping the checksum comparison fails
  `checksum_mismatch_…`; making `confirm` always yes fails 3 decline tests;
  removing the conflict prompt fails `declining_a_file_conflict_…`.
- `cat install.sh | sh -s -- --help` works (the `curl | sh` path).

## Limits

- Not run: real `/usr/local/bin` with sudo, a real `/dev/tty`, macOS, Windows,
  musl/arm64 hosts. The test host is macOS; archives are named for
  `linux-amd64` via `IROH_TUNNEL_TARGET` and contain host-built executables.
- Not run against a published release: v0.4.2 has no role archives. The first
  release from this tree is needed for a true end-to-end run (#97).
- Checksum and archive come from the same host: this detects corruption, not a
  compromised host.
