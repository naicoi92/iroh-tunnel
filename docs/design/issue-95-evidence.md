# Issue #95 — verify and replace one standalone executable

Baseline `cf124d0` (origin/main). Branch `redesign/issue-95-standalone-update`. Blockers #80 and #82 closed at pickup. Policy from #80: bypass `--insecure` / `-i`, stage beside then rename, failure keeps the old binary, rotation deferred (DIST-03).

## Acceptance map

| Criterion | Evidence | Status |
|---|---|---|
| 95.AC1 signature + checksum; failure keeps old binary; warned bypass | Checksum: `changed_payload_fails_checksum_and_keeps_old_binary`. Signature: seam `SignatureVerifier`; `wrong_identity_fails_and_keeps_old_binary` (fixture), `missing_signature_bundle_fails_and_keeps_old_binary`, default verifier rejects (`update_without_verification_material_keeps_installed_binary`, real binary). Bypass: `insecure_update_installs_latest_and_warns`, `insecure_skips_checksum_of_a_changed_payload` | **PARTIAL**: no real keyless verification until #100 |
| 95.AC2 running process, replacement | `update_does_not_stop_a_running_process_of_the_old_binary` (real tunnel started from the replaced file, still alive, output tells to restart); `src/update.rs` unit tests (permissions kept, staging removed, failure keeps old); `unwritable_install_dir_exits_with_permission_code_and_keeps_binary` (exit 3) | Unix only |
| 95.CL1 default latest, explicit version, only the selected file changes | `insecure_update_installs_latest_and_warns`, `short_flag_and_explicit_version_select_that_release`, `verified_release_replaces_only_the_invoked_file`, `latest_never_downgrades_a_newer_installed_version` | Pass (local release server) |
| 95.CL2 with #96: package/container/alias ownership | Not implemented | **OPEN**: #96, alias deferred (DIST-03) |
| 95.CL3 no automatic stop; failure preserves; actual #100 artifacts | See AC1/AC2 | #100 artifacts **not validated** |

## Real-path evidence

- macOS arm64: `iroh-tunnel update --to 0.4.2 -i` against the real GitHub release v0.4.2 replaced the all-in-one binary (`--version` 0.4.2-dev → 0.4.2). Same command without `-i` fails with "cannot fetch the signature bundle" and changes nothing (release has no bundle).
- `iroh-tunnel-serve update` against v0.4.2 returns 404: the release has no standalone role archives (they come with #82/#124/#100 releases).
- Review: Standards and Spec reviews run on `cf124d0...ed60927`; findings addressed in `7642c0a` (staging name/TOCTOU, truncated entry, `--insecure --to` without checksums, no silent downgrade, exit 3 on permission).

## Gates

`cargo fmt --check`, `cargo clippy --all-targets --all-features --locked -D warnings`, `cargo test --all-features --locked` (333 pass), `cargo +1.91 check --all-targets --locked`: pass.

## Limits

- Signature verification is a fail-closed seam; the bundle name `<checksums>.sigstore.json` is an assumption for #100. Without `-i`, `update` cannot succeed today.
- Windows (x64/ARM64, zip path, locked running executable) and Linux native were not run; `tests/update.rs` is `cfg(unix)`. Evidence belongs to #97.
- Release tests use a local server and a shell-script payload, not signed final artifacts.
- New dependencies: `reqwest` (already in the tree), `rustls` (`ring`), `sha2`, `flate2`, `tar`, `zip`; prior art in `issue-95-update-prior-art-research.md`.

## Follow-up — keyless signature verifier (branch `redesign/issue-95-signature-verifier`)

`SigstoreVerifier` replaces the fail-closed placeholder. It verifies the `checksums*.txt` bundle offline against the embedded Sigstore public-good trusted root (certificate chain, SCT, transparency log), requires issuer `https://token.actions.githubusercontent.com` and a signer identity starting with `https://github.com/naicoi92/iroh-tunnel/.github/workflows/release.yml@refs/tags/v`. Wrong issuer, wrong identity, changed payload and missing, empty or invalid bundles fail and keep the installed binary.

Evidence uses a real `cosign sign-blob` bundle (`tests/fixtures/sigstore/`, copied from sigstore-rust test data; signer `w.vollprecht@gmail.com`, issuer GitHub OAuth), not artifacts of this project: genuine accepted for its own issuer/identity; payload change, wrong identity, wrong issuer, release pin, bad bundle rejected; real binary rejects the foreign bundle (exit 1, file unchanged).

Assumptions for #100 (to confirm there): bundle published as `<checksums file>.sigstore.json`, produced by `cosign sign-blob --new-bundle-format` (or an equivalent v0.3 message-signature bundle) in the release workflow on a `v*` tag; the repository is public (the transparency log is required).

New dependencies: `sigstore-verify`, `sigstore-trust-root`, `sigstore-types` 0.15 (`aws-lc-rs` provider; adds `aws-lc-sys`, a C build, to every target — see CI/snapshot evidence). Trusted root is embedded: a Sigstore key rotation needs a new release (DIST-03: reinstall).
