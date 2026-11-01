# Issue #80 — Update recovery and signing trust

Status: **decided by the owner in the agent session on 2026-11-01 (not yet echoed as an issue comment).** Decision and evidence gate; no production change. Base: `origin/main` b1e6502. Consumers: #95 (updater), #100 (signed artifacts), #97 (release gate). Traceability: DIST-02–03, stories 64–68.

## Principle

`update` is run by the user. The project builds the behavior the design confirms and does not add safeguards against the user's own choices.

## Decisions

| Topic | Decision |
|---|---|
| Verification | Pinned CI identity/signature plus checksum (confirmed earlier). |
| Verification failure | Report the error; the installed binary stays. |
| Bypass | `--insecure` / `-i` skips signature and checksum together and prints a warning. The user accepts responsibility. |
| Replacement | Stage the verified file beside the installed one, then rename over it. Any failure, including a file the OS keeps locked (Windows), reports the OS error and keeps the old binary. |
| Running tunnel | Never stopped automatically. The updater tells the operator to stop, update and restart (confirmed earlier). |
| Signing identity rotation | Not designed now (DIST-03, deferred). If the identity changes, verification fails and the user reinstalls with the installer. |
| Package or container ownership | Delegate to the package manager or image redeploy (AC-DIST-07, confirmed). |
| Partial failure | A failure fails the whole command; the user reruns it. Each binary updates independently (story 62), so no cross-binary rollback exists. |

`-i` is unused by any current command, so it does not conflict with `short_options_are_consistent_and_conflict_free` in `src/cli.rs`.

## Evidence

- `release.yml` has no signing step, and the installers verify only a checksum from the same host (`install.sh` line 344, `install.ps1` line 318). #100 must produce signed artifacts before #95 can verify against real material.
- Replacing a running executable by rename: measured on macOS arm64. The process kept running, and the old inode stayed valid.
- Not measured: native Linux and real Windows. A Docker run on macOS is not valid Linux evidence. #95 must test the locked-executable case on Windows x64/ARM64 and disclose any limit.

## Deferred (DIST-03)

Signing identity rotation, rollback, alias update behavior, Windows-specific replacement, package detection.

## Mapping

| #80 criterion | Where |
|---|---|
| Failure and recovery outcomes, bypass warning, preservation of installed binaries | Decisions table; DIST-02; UPDATE-BYPASS-01, UPDATE-RECOVERY-01 |
| User reviews rotation and platform replacement policy | Owner decision above; rotation deferred, replacement as stated |
