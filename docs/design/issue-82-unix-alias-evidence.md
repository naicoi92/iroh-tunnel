# Issue #82 bounded Unix archive aliases

Baseline `30b9f85a78ec738c0789a496bd490e378b4e9a25`. Live #82 body,
comments and native dependency refreshed before implementation; #81 closed.
Source: #82 CL1 and procedure CLI-ALIAS-01. Approved seam: actual Linux/macOS
GoReleaser archive producer, extracted aliases, independent real processes,
local relay, TCP client and target. No installation or update policy change.

## Acceptance matrix (recorded before tests)

| Criterion | Test | Expected-value source |
|---|---|---|
| CLI-ALIAS-01 / #82 CL1: produced aliases resolve and forward their intended roles independently | `tests/release_aliases.py` → `produced_archive_aliases_forward_tcp_independently` | Confirmed names: `it -> iroh-tunnel`, `its -> iroh-tunnel-serve`, `ita -> iroh-tunnel-access`; all-in-one `serve`/`access` and standalone roles exchange literal `matrix request` / `matrix response` through real TCP target/client. Resolved digest equals extracted intended binary; each role directory contains only alias and its binary. |

Test harness derives snapshot config from actual producer, changing only `dist`
and adding `--features=test-utils` to build flags. GoReleaser snapshot never
publishes; homebrew/Docker/nFPM and pre-build checks skipped by harness. Production
configs must never enable test-utils. Fixture binaries may contact only isolated
local relay; never public relays. Production snapshot checked separately.

Linux existing GNU/musl target matrix and archive names must remain unchanged.
Existing nFPM/Docker IDs and cask binary/alias ownership remain unchanged. Windows
producer and aliases, persistent-key option, installer/package/update ownership
and full #82 / #97 acceptance remain open. Cross-builds do not establish runtime
acceptance. Independent Standards and Spec reviews required after implementation.

## Producer implementation and ownership boundary

Both production configs now pass explicit `--bin=<name>` for each Rust build.
GNU role builds retain x86_64/aarch64 GNU targets; musl role builds retain
x86_64/aarch64/riscv64gc musl targets. macOS retains native aarch64 target.
Each existing archive collects its matching three build IDs and the tracked
`packaging/archive-aliases/{it,its,ita}` symlinks with `strip_parent: true`.
No new archive naming scheme. Existing all-in-one IDs remain the only nFPM and
Docker inputs. Existing Homebrew cask still installs only `iroh-tunnel` and its
existing postflight `it` link; new role binaries/aliases are in its staged
archive but are not declared cask installations. No package/cask ownership,
conflict, upgrade, replacement or uninstall policy changed.

Read GoReleaser Rust builder and archive documentation:
<https://goreleaser.com/customization/builds/rust/> and
<https://goreleaser.com/customization/archive/> (GoReleaser 2.18.1).
Used existing Rust builder default `zigbuild`, custom flags, archive build-ID
selection and additional files with parent stripping. Actual tar validation
establishes symlink preservation rather than assuming documentation implies it.

## Red → green (macOS arm64)

Before producer edit:

```sh
python3 tests/release_aliases.py --config .goreleaser.macos.yaml --output /tmp/iroh82-alias-red
```

GoReleaser built actual old producer tar successfully. Harness failed exactly:
`AssertionError: iroh-tunnel-0.4.2-SNAPSHOT-30b9f85-darwin-arm64.tar.gz: missing alias it`.
Red artifact SHA-256:
`7f9d4071d03a90b2d9c4f59be6ad428fadb7f7124f3b8f88c55e96a061de1cfc`.
Transcript `/tmp/iroh82-alias-red.log`; retained snapshot config in output dir.

After producer edit, final harness:

```sh
python3 tests/release_aliases.py --config .goreleaser.macos.yaml --output /tmp/iroh82-alias-verified
```

Passed. Fixture archive:
`/tmp/iroh82-alias-verified/dist/iroh-tunnel-0.4.2-SNAPSHOT-30b9f85-darwin-arm64.tar.gz`.
SHA-256 `d5f28c655b525f3dc0f8fcf850bd2cef7d154da0ba62197edbb24d729a2ae02f`,
verified against producer `checksums-darwin.txt`. Tar links and extracted links
both match confirmed relative targets. Each isolated directory contains only
one binary and its preserved producer alias, never a synthesized test link.
Resolved SHA-256 values:

- `it` / `iroh-tunnel`: `515afd9083fbe01ac4da6102b05f55aef1032fb0f81192145124c80e7244dc42`
- `its` / `iroh-tunnel-serve`: `26f3b8a4fd346e053b125410e31f14a762300ae62d43bb78db29fddc2fd08707`
- `ita` / `iroh-tunnel-access`: `113f35f6346e35b4001e1064565f71bd92d5b9c930f0fc1c75a7a90d8e39a368`

Each alias `--version` returned its intended binary name and `0.4.2-dev`.
Real-process test passed: `it serve` PID 33731 / `it access` PID 33732 and
`its` PID 33752 / `ita` PID 33753 each delivered exact `matrix request` to
fixed TCP target `127.0.0.1:50738` and `matrix response` to client. Local relay
fixture only, no public relay. Test-owned children killed/reaped by existing
process guard. Transcript `/tmp/iroh82-alias-verified.log`.

Acceptance matrix: 1/1 bounded criterion exercised on macOS arm64. This is not
full CLI-ALIAS-01 platform acceptance. Same archive producer inspection and
host-compatible runtime harness can run with `.goreleaser.yaml` on Linux;
foreign architecture archives inspected but explicitly not executed.

## Production snapshot (no test feature, no publication)

Copied actual macOS config changing only `dist` to
`/tmp/iroh82-production/dist`; ran:

```sh
goreleaser release --snapshot --clean --config /tmp/iroh82-production.yaml
```

Passed including original before hooks, all three `cargo zigbuild --release
--bin=...` builds, archive, checksum, generated cask. No feature override,
`--snapshot` skips publish. Transcript `/tmp/iroh82-production.log`.
Production archive SHA-256:
`836d0e06158029ba37eb8fec02cbcf8383927f9309a4dd9d699c94205f54db82`.
Extracted actual relative links; all three aliases returned correct `--version`.
Generated cask contains only `binary "iroh-tunnel"` and unchanged postflight.
Production artifact TCP roundtrip not claimed: local relay requires test-utils;
production TLS verification never disabled. Snapshot version in archive name
and binary `0.4.2-dev` follow existing build.rs version contract, unchanged here.
Archive hashes differ across runs because producer embeds file metadata; no
reproducible-build claim.

## Validation

On macOS arm64, Rust 1.91.1:

- `cargo fmt --all -- --check`: passed.
- `cargo +1.91.1 clippy --all-targets --all-features --locked -- -D warnings`: passed.
- `cargo +1.91.1 test --locked --test quick_run -- --nocapture`: 17 passed,
  1 ignored (producer test explicitly run by harness above).
- `cargo +1.91.1 test --locked --test cli_role_matrix`: 3 passed.
- `cargo +1.91.1 test --all-features --locked`: 148 passed, 6 ignored
  (producer fixture test, existing interop case, four public-relay tests).
- Both `goreleaser check --config .goreleaser.yaml` and
  `goreleaser check --config .goreleaser.macos.yaml`: passed.
- `git diff --check`: passed.
- Structural comparison against baseline: original archive names, original
  build target lists, nFPM, Docker, cask, release and checksum sections unchanged.

Producer snapshots and harness use installed Rust 1.99.1, cargo-zigbuild 0.23.0,
Zig 0.16.0, GoReleaser 2.18.1. These do not prove release CI's pinned Zig 0.13
and Rust 1.91 producer path; Rust tests/lints above separately use 1.91.1.
No ignored public-relay test run. Relevant Rust standards: RS-01 (reuse cohesive
fixture), RS-05 (context/propagation), RS-08 (owned child cleanup), RS-11 (Path,
preserved producer links), RS-13/15 (Unix gating and isolated relay), RS-14
(criterion-derived real artifact test), RS-16 (locked MSRV validation).

Linux snapshot attempt and final commit recorded below. Independent Standards
and Spec review remains required; implementation self-check is not independent
review. No PR, push, merge or issue-close action.

## Linux producer limitation

Attempted real full-matrix config with only `dist` changed, no test feature:

```sh
goreleaser release --snapshot --clean --config /tmp/iroh82-linux-production.yaml --skip=before,docker,nfpm --parallelism=1 --timeout=2m
```

Cross-build produced x86_64 GNU all-in-one ELF (`ece980fe006e2508b844ed645f7e0752ec04b0f1b011e5e906d8d819a0ff62cb`),
then aarch64 GNU build exceeded timeout. Final output: `release timed out after
3m18s error=context deadline exceeded`. Cleanup found only GoReleaser process;
no build processes remained afterward. No Linux tar or role build completion claimed. Transcript
`/tmp/iroh82-linux-production.log`. Linux glibc/musl/riscv64 producer and runtime
acceptance remains pending suitable CI, as do package/Docker build checks (not
changed or run locally). No Linux runtime on this macOS host.
