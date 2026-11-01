# Issue #80 — Update recovery and signing trust: evidence and options

Status: **evidence and options gate. No product decision below is approved; no production code changes.** Base: `origin/main` b1e6502. Date: 2026-11-01. Traceability: DIST-02, DIST-03; stories 64–68; consumers #95 (updater), #100 (signed artifacts), #97 (release gate).

Confirmed inputs (not re-opened): verify pinned CI identity/signature plus checksum; failed verification without bypass keeps the installed binary; an explicit unsafe bypass warns; the updater never stops a running tunnel automatically and gives stop/update/restart guidance (#80 comments, spec DIST-02).

## 1. Current-state inventory

| Area | Observed | Source |
|---|---|---|
| Release signing | None. `release.yml` has `contents: read`/`write` only; no `id-token: write`, no cosign/attestation step. Output is `checksums.txt` and `checksums-darwin.txt` from `sha256sum`. | `.github/workflows/release.yml` lines 33–35, 90–99 |
| Installer trust | Checksum only; the checksum file is fetched from the same host as the archive, so it proves integrity against corruption, **not authenticity** against a compromised host. Installers print "signature verification is not available yet". | `install.sh` lines 30, 344; `install.ps1` lines 18, 318 |
| Updater | Does not exist (#95). | `src/` has no update module |
| Installer replacement | Installer stages, verifies the checksum, then replaces; skips identical files. Behavior on a running binary is not specified. | `install.sh` lines 225–264 |

Prerequisite for any signature verification: #100 must produce keyless-signed artifacts. Until then the verifier has no real material; fixtures alone cannot close UPDATE-VERIFY-01.

## 2. Replacement evidence (criterion 1: preservation of installed binaries)

Experiment: C program that sleeps, run as a live process; then three ways of replacing its file (`/tmp/e80`, reproducible in a few lines).

| Platform | Overwrite in place (`cp`) | Truncate-write | Write temp + `rename` over target |
|---|---|---|---|
| macOS arm64 (real, Darwin 24.6) | succeeded | succeeded | succeeded, old process kept running |
| Linux (Docker Desktop on macOS, alpine) | succeeded | succeeded | succeeded, old process alive |
| Windows | **not run** | **not run** | **not run** |

Limits, stated plainly:

- The Linux row is **not valid evidence**. Docker Desktop on Apple Silicon runs through a VM/emulation layer where an in-place write to a running executable did not return `ETXTBSY` as native Linux kernels normally do for running text files. The expected native behavior (in-place write fails with "Text file busy", rename succeeds) is **[Inference]** and must be re-measured on a native Linux runner in CI.
- The macOS in-place write also succeeded, but overwriting a running Mach-O in place risks the running process and invalidates code-signing state; rename-over is the only variant that is safe by construction, because the old process keeps its old inode.
- Windows: no Windows host was available. The following is from documentation only. **[Unverified on real Windows]**
  - `self_replace` docs (docs.rs): a running executable on Windows can be renamed but not unlinked or overwritten; the crate moves the running exe aside, places the new one, then needs a helper process to delete the leftover after exit.
  - `MoveFileExW` docs: `MOVEFILE_DELAY_UNTIL_REBOOT` completes the move after reboot; the return value reflects only that the registry entry was written, not that the move succeeded; it requires admin to write the pending-rename key for protected locations. **[Inference]** for system-wide install paths.

Conclusion for the test plan: the only replacement primitive demonstrated safe for a live process on Unix is *stage in the same directory, verify, then atomic rename*. A Windows locked-executable test must run on real Windows x64/ARM64 (`windows-service-e2e.yml` shows such runners exist in this repo).

## 3. Options and consequences

### 3.1 Replacement and failure recovery (criterion 1)

| Option | Mechanism | Failure outcome | Consequence |
|---|---|---|---|
| R1 (proposed) | Download to a staging dir **in the same directory/filesystem as the target**, verify there, then atomic rename. Any failure before rename leaves the installed binary untouched and removes staging. | Pre-rename failure: old binary intact, error with cause. Rename failure: old binary intact. | Needs write access to the install directory; system-wide installs require elevation → updater must stop with elevation guidance, not escalate itself. Needs ~1 binary of extra disk. |
| R2 | R1 plus keep one `<name>.previous` copy for manual rollback. | Same as R1; operator can restore manually. | Leaves a stale executable; on Windows, a locked `.previous` may block the next update. Adds a promise (rollback) that needs its own tests. |
| R3 | Overwrite in place. | Partial write can corrupt the only copy. | **Reject**: violates "failed update keeps old binary". |

Windows running-process policy (decision needed, only after R1):

| Option | Behavior | Consequence |
|---|---|---|
| W1 (proposed) | Detect that replacement is blocked by a running or locked file; **stop with guidance**, change nothing, keep old binary. Operator stops the tunnel, reruns update, restarts. | Consistent with the confirmed no-auto-stop rule. Weakest convenience. Detection must be tested on real Windows. |
| W2 | Rename the running exe aside, place the new one, remove the leftover on next start/reboot (self_replace pattern). | Process keeps running old code until restart, update succeeds without stopping the tunnel. Needs leftover cleanup and a helper process; does not auto-stop, so it does not violate the confirmed rule, but it adds a Windows-only mechanism requiring real-OS evidence. |
| W3 | `MOVEFILE_DELAY_UNTIL_REBOOT`. | Reports success before the move happens; version after the call is ambiguous; needs admin. **Not recommended.** |

Partial failure of a multi-binary state: because each binary updates independently (stories 62), there is no cross-binary transaction; "partial failure" reduces to a single-file outcome under R1. This is a simplification the owner should confirm.

### 3.2 Bypass semantics (criterion 1: explicit bypass warning)

The confirmed rule says a *deliberate unsafe bypass warns*. Exact scope is open. Candidates:

| Option | Scope | Consequence |
|---|---|---|
| B1 (proposed) | One flag (name TBD, e.g. `--insecure-skip-verify`) skips **signature/identity** verification only; checksum still checked. Prints a warning naming what was skipped. | A checksum from the same host proves integrity only; the warning must say so. |
| B2 | Separate flags for signature and checksum. | Finer control; larger attack surface and test matrix. |
| B3 | One flag skips both. | Simplest; removes all verification, so the warning must be loud. |

In every option: interactive confirmation is not required, non-interactive use is allowed, the bypass is never the default, and a verification failure without the flag keeps the installed binary (AC-DIST-03, AC-DIST-09).

### 3.3 Signing identity and rotation (criterion 2)

Verification identity for keyless signing is the pair issuer (`https://token.actions.githubusercontent.com`) plus certificate identity (the workflow ref that signs). Cosign docs: `--certificate-identity` and `--certificate-oidc-issuer` select the accepted identity; blobs verify with a bundle file containing certificate and signature.

Key design question: the updater binary embeds the pinned identity, so rotation (repo rename, workflow path change, owner change) can strand already-installed updaters.

| Option | Mechanism | Consequence |
|---|---|---|
| T1 (proposed) | Pin issuer plus an identity **pattern** bound to repo and release workflow (not to a ref/tag). Rotation = ship a new binary that accepts old and new identities; old updaters still verify releases signed under the old identity. | Requires the identity change to be rolled out in a release signed under the *old* identity first, i.e. a two-step rotation. Document it. |
| T2 | Fetch the trusted identity from a signed manifest at a stable URL. | Enables remote rotation but adds a second trust root and an availability dependency. |
| T3 | Out-of-band reinstall on rotation (installer script with new identity). | Simple; strands updaters, which fail closed (keep old binary). |

Fail-closed behavior in all options: an updater that cannot verify keeps the old binary and prints guidance (reinstall via installer). That is already the confirmed behavior; rotation policy only decides how often users hit it.

Library/tool choice (shell out to `cosign` vs a Rust sigstore verifier) is an implementation decision for #95 and is **not** made here; adding a dependency would need owner approval.

## 4. Acceptance criteria mapping

| #80 criterion | Covered by | State |
|---|---|---|
| Describe failure and recovery outcomes, bypass warning, preservation of installed binaries | §3.1, §3.2 | Options written; **awaiting owner choice** |
| User reviews signing identity rotation and platform replacement policy | §3.3, §3.1 Windows table | **Awaiting owner review** |

Downstream tests unblocked once chosen: UPDATE-BYPASS-01 (B), UPDATE-RECOVERY-01 (R, W, T), UPDATE-VERIFY-01 (needs #100), UPDATE-RUNNING-01 (no auto-stop; already confirmed).

## 5. Measurements still required (not done here)

1. Native Linux x64/arm64: confirm `ETXTBSY` on in-place write and success of rename-over, in CI.
2. Real Windows x64/ARM64: rename-aside, overwrite, delete and replace while the executable runs; antivirus interference not measured.
3. Cross-filesystem staging (target on a different mount than the temp dir): `rename` fails with `EXDEV`; R1's "same directory" rule avoids it.
4. First real keyless-signed artifact from #100 to verify the identity string actually produced by this repo's workflow.

## 6. Decisions requested from the owner

1. Replacement: R1 or R2? (recommended R1)
2. Windows running binary: W1 or W2? (recommended W1 for the first release)
3. Bypass scope: B1, B2 or B3? (recommended B1) and the flag name.
4. Rotation: T1, T2 or T3? (recommended T1)
5. Confirm per-binary independence removes the need for cross-binary rollback.
