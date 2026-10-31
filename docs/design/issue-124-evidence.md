# Issue #124 — role binaries for Windows and macOS

Baseline `bcb6577` (origin/main). Branch `redesign/issue-124`. No native blockers.

## Change

- `.goreleaser.yaml`: three Windows builds (one per executable) for
  `x86_64-pc-windows-gnu` and `aarch64-pc-windows-gnullvm`, cross-built from Linux
  with cargo-zigbuild. `.zip` archives named `<role>-<version>-windows-<arch>.zip`,
  listed in `checksums.txt`.
- `.goreleaser.macos.yaml`: `iroh-tunnel-serve` and `iroh-tunnel-access` builds and
  archives for `aarch64-apple-darwin`. The Homebrew cask is restricted to the
  all-in-one archive (`ids: [macos]`); per-role casks stay undecided (#94 evidence).
- `packaging/verify-dist.sh`: new `windows` and `macos` modes (exactly one
  executable per archive, digest against the checksum file, PE or Mach-O format).
- `.github/workflows/snapshot.yml`: runs both modes, uploads the Windows archives,
  and a `windows-runtime` job runs each role's `--help` on real Windows x64
  (`windows-latest`) and ARM64 (`windows-11-arm`). The workflow now also runs on
  pull requests that touch release configuration.

## Acceptance map

| Criterion | Evidence | Status |
|---|---|---|
| 124.AC1 one archive per role for macOS arm64, Windows x64, Windows ARM64, in the checksum file | `verify-dist.sh windows` 6/6, `macos` 3/3 on a local snapshot | PASS (cross-build, unsigned) |
| 124.AC2 each binary starts and prints help on a real host; cross-build vs runtime separate | macOS arm64: run locally and in CI (`macos` mode). Windows x64/ARM64: CI `windows runtime (amd64)` and `(arm64)` ran all three roles (`PROCESSOR_ARCHITECTURE` AMD64 and ARM64), digest checked first | PASS (start-up `--help` only) |
| 124.AC3 names follow the existing convention | `<role>-<version>-<os>-<arch>`; Windows uses `.zip` | PASS |

## Evidence (macOS arm64 host, GoReleaser snapshot of this tree)

- Red: `verify-dist.sh windows` with the ARM64 serve archive removed fails
  (`no single iroh-tunnel-serve archive for windows-arm64`); with one archive
  appended a byte it fails (`checksum differs`). Against the previous dist (before
  this change) no Windows archive existed and no role macOS archive existed.
- Green: `windows` 6/6 ok; `macos` 3/3 digest ok and 3/3 run (`--help`) on this host.
- All six Windows executables are `PE32+ … x86-64` / `Aarch64`; all three macOS
  executables are Mach-O arm64.
- `goreleaser check` passes for both configs; `shellcheck --severity=warning` clean.

## Limits

- Windows binaries are cross-built (GNU ABI, not MSVC). The local run does not
  execute them; only the CI `windows-runtime` job does, and it checks `--help`
  start-up, not forwarding. ARM64 coverage depends on the `windows-11-arm` runner.
- Not signed (#100/#80). Not published: no tag was pushed; `release.yml` runs the
  same configs on a tag and was not executed.
- macOS Intel and Windows packages/containers are out of scope.
- A real tunnel across Windows (bytes) is for #97.
- The tag flow (`release.yml`) is not gated by `windows-runtime` or `verify-dist.sh`;
  those run in `snapshot.yml` only. A release job that publishes Windows archives
  without running them is for #100/#97 to gate.
- `install.sh` still says role binaries for macOS/Windows are unpublished; that text
  and the PowerShell/macOS installers belong to #93.
