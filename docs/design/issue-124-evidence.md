# Issue #124 — role binaries for Windows and macOS

Baseline `d9f9ec6` (origin/main, after the native parallel build in #127).
Branch `redesign/issue-124-native`. No native blockers.

The first version of this work extended GoReleaser (cross-built Windows). #127
removed GoReleaser, so it was redone on the native matrix: every target builds on
its own runner.

## Change

- `.github/workflows/build.yml`: two new matrix targets, `windows-amd64`
  (`windows-latest`, `x86_64-pc-windows-msvc`) and `windows-arm64`
  (`windows-11-arm`, `aarch64-pc-windows-msvc`), built natively with MSVC. The
  macOS target now builds all three executables (it built only `iroh-tunnel`).
  The `bins` matrix field is gone: every target builds `--bins`. The smoke test
  starts each executable on its own runner. Windows gets one `.zip` per
  executable (`<role>-<version>-windows-<arch>.zip`, flat, with README and
  licenses).
- `package.yml`: also runs `verify-dist.sh windows` and `macos`.
- `release.yml`: `checksums.txt` now lists the `.zip` files; macOS stays in
  `checksums-darwin.txt`.
- `packaging/verify-dist.sh`: new `windows` and `macos` modes (exactly one
  executable per archive, PE or Mach-O format for the platform).
- Homebrew cask untouched: still the all-in-one archive only (role casks are
  undecided, #94 evidence).

## Acceptance map

| Criterion | Evidence | Status |
|---|---|---|
| 124.AC1 one archive per role for macOS arm64, Windows x64, Windows ARM64, in the checksum files | `verify-dist.sh windows` and `macos` in `package.yml`; checksum step in `release.yml` | PASS: snapshot runs 41192153962 (`6f63674`) and 41193286466 (`f7ab25f`, final build) — `package` ran `verify-dist.sh windows` (6 archives) and `macos` (3 archives); `release.yml` now fails if `checksums.txt` lacks any of the 6 `.zip` or `checksums-darwin.txt` any of the 3 role tarballs (not exercised: no tag) |
| 124.AC2 each binary starts and prints help on a real host; cross-build vs runtime separate | smoke test in `build.yml` runs on the native runner of each target, and prints the runner OS and architecture | PASS: same run, all three roles started on Windows X64, Windows ARM64 and macOS ARM64 (and Linux X64/ARM64); start-up only |
| 124.AC3 names follow the existing convention | `<role>-<version>-<os>-<arch>`; Windows uses `.zip` | PASS: `verify-dist.sh` globs matched `<role>-0.4.2-dev-windows-{amd64,arm64}.zip` and `-darwin-arm64.tar.gz` |

## Limits

- Start-up (`--help`, `--version`) only; no bytes are forwarded on Windows (#97).
- Not signed (#100/#80). The tag flow (`release.yml`) was not run; `snapshot.yml`
  rehearses everything except publishing.
- `windows-11-arm` runners are free for public repositories only.
- MSVC build, not the GNU/gnullvm cross-build of the first attempt. The build
  needs the MSVC toolchain on the runner (present on GitHub-hosted runners).
- macOS Intel and Windows packages/containers are out of scope.

## CI failure found during review

PR `scripts` job (actionlint 1.7.7) rejected `windows-11-arm` as an unknown runner
label, which failed `ci-ok`. The build itself was fine. Fixed with
`.github/actionlint.yaml`; reproduced locally with the same actionlint version
(red), green after the fix, and a bogus label still fails.
