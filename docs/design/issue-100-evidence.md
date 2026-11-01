# Issue #100 evidence — signed release artifacts

Baseline `ad7094c` (origin/main). Blockers #80, #82 and #124 closed at pickup. Policy from #80: keyless identity rotation deferred (DIST-03); `--insecure` / `-i` bypass unchanged.

## Design

`release.yml` on a `v*` tag: `gate` ∥ `build` → `package` → `sign` → `verify` → `draft`.

- `sign` computes `checksums.txt` / `checksums-darwin.txt` and signs each with `cosign sign-blob --new-bundle-format` (keyless, Sigstore public-good). Output `<checksums>.sigstore.json` is the name `update` fetches. The job runs inside `release.yml` itself, so the certificate identity is `https://github.com/naicoi92/iroh-tunnel/.github/workflows/release.yml@refs/tags/<tag>`, issuer `https://token.actions.githubusercontent.com` — the pin in `src/update.rs`.
- `packaging/verify-release-set.sh` checks the full matrix: 3 roles × (5 Linux archives + 2 Windows zips + 5 packages) in `checksums.txt`, 3 darwin-arm64 archives in `checksums-darwin.txt`, both bundles; every listed file present and matching; nothing extra.
- `verify` runs `tests/release_artifacts.rs -- --ignored` on the REAL files with the verifier from #95 (`SigstoreVerifier`).
- `draft` uploads to a DRAFT release and stops. A published release is never modified.
- `promote.yml` (manual, requires an evidence URL) re-downloads the draft, repeats the set check and signature verification at the tag's commit, publishes, then updates the Homebrew cask from the published checksums. Image push and cask update no longer run on a tag.

## Acceptance

| AC | Evidence | Status |
|---|---|---|
| 100.AC1 every role/all-in-one artifact with checksum + keyless bundle bound to CI issuer/identity | Run [41315774710](https://github.com/naicoi92/iroh-tunnel/actions/runs/41315774710) on tag `v0.4.3-rc.2` (commit `44809c2`): 43 assets (3 roles × 5 Linux archives, 2 Windows, 5 packages, 3 darwin; 2 checksums; 2 bundles). `signed_release_bundles_verify_for_pinned_issuer_and_exact_identity` passed for both bundles, identity of this run (the verifier matches the identity as a prefix) and default `SigstoreVerifier::release()` | PASS (rc, draft) |
| 100.AC2 real production + #95 verifier incl. wrong identity, changed payload, missing/invalid material; immutable IDs | `signed_release_rejects_wrong_identity_and_wrong_issuer`, `..._changed_payload_and_swapped_bundle`, `..._missing_or_invalid_verification_material` passed on the real files. Every asset's sha256 is printed in the `verify` job log and step summary. End to end with the real `iroh-tunnel-serve update` binary against the draft's assets served locally: genuine → installed `0.4.2-dev` → `0.4.3-rc.2` without `-i`; changed archive → checksum mismatch; changed checksums → "message digest in bundle does not match artifact hash"; missing bundle → 404; each left the binary byte-identical (sha256 prefix `d17a0a1f…` unchanged) | PASS on macOS arm64 |
| 100.AC3 public promotion gated on complete set and final #97 evidence | Tag never publishes. `promote.yml` needs a manual dispatch with an evidence URL in this repository, refuses non-drafts, re-verifies. Not exercised end to end: publishing the rc would be public | CONFIGURED, not run. The gate is a manual dispatch with a recorded evidence URL (human attestation); it is not machine-checked against #97 |

Immutable identifiers of the signed inputs (rc.2): `checksums.txt` `sha256:e167b64c…6335b`, `checksums.txt.sigstore.json` `sha256:1ae34d86…df379`, `checksums-darwin.txt` `sha256:d5b1d98c…ebf969`, `checksums-darwin.txt.sigstore.json` `sha256:2822abfc…8a2d`; full digests of all assets: `issue-100-rc2-asset-digests.txt`.

## Red / green

- Red: `release_set_*` against a missing `packaging/verify-release-set.sh` — 5 failed.
- Green: 5 passed after adding the script. Removing each of the 44 expected files in turn is rejected and named.
- Real-file tests are `#[ignore]` and ran only in CI against run 41315774710.

## Limits and open decisions

- Only a prerelease tag was produced. The draft `v0.4.3-rc.2` and its tags are disposable; the final stable set and #97 acceptance remain open (RELEASE-SIGN-01 final integration).
- Verified by running `update` on macOS arm64 only; Linux and Windows runtimes were not exercised against the signed files. Their archives are signed and listed identically.
- `ghcr.io` image push moved out of the tag workflow into no workflow yet: after promotion no job pushes the images. Pushing them is public promotion; adding a gated job to `promote.yml` or a follow-up issue is **open for the owner** and must be settled before release.
- `promote.yml` was not run end to end (it would publish). Its inputs are validated and passed through `env:`; actions are pinned to version tags, not commit SHAs, as in the rest of the repository.
- `macos-14` is being retired (brownout failed the first rc); `build.yml` now uses `macos-15`.
- cosign pinned to `v3.1.4` via `sigstore/cosign-installer@v4.1.2`. A Sigstore trusted-root rotation needs a new release (DIST-03).
