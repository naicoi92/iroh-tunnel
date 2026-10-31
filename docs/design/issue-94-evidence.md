# Issue #94 — separate role packages and images

Baseline `dcced29` (origin/main). Branch `redesign/issue-94`. Builds on PR #116
(per-executable packages, archives and images). Blocker #82 closed at pickup.

## Acceptance map

| Criterion | Evidence |
|---|---|
| 94.AC1 package and image contains the intended artifact, runs its role alone | `verify-dist.sh static` (exact members), `runtime` (install alone, package database owns binary and alias, run, remove), `images` (one executable, entrypoint), `forward` (real bytes) |
| 94.AC2 ownership on supported platforms, Windows where applicable | PARTIAL: Linux only. Windows owned by #93/#100/#97; macOS role casks were removed in #116 and are undecided. See Limits |
| 94.CL1 ownership, contents, aliases, independent execution; cross-build vs runtime separated | Table below |
| 94.CL2 service composition | Packages are binary-only: no unit, config dir or maintainer script, so no composition with #79/#91 is claimed |
| #100 / #97 | Out of scope: signed artifacts (#100), final artifact identities (#97) |

## Change in this ticket

The all-in-one package shipped `iroh-tunnel.service` (`ExecStart=/usr/local/bin/...`
while the package installs to `/usr/bin`, so the unit could not start) and a
postinstall script. Its account (`DynamicUser`) and trust settings have no approved
policy (RO-03 sets the system account to root; #79/#91 are open). All packages are
now binary-only, which 94.CL2 allows. `verify-dist.sh static` expects exactly the
binary and its alias link; against the previous dist it failed on all five
all-in-one packages (red), then passed after the change (green).
`verify-dist.sh forward` is new.

## Evidence (macOS arm64 host, Docker arm64, GoReleaser snapshot of this tree)

| Check | Result | Class |
|---|---|---|
| `static` | 15 packages (3 roles × deb amd64/arm64, apk amd64/arm64/riscv64) own exactly binary + alias; 15 archives hold only their binary | cross-build, inspected |
| `runtime` arm64 | each deb on `debian:12-slim`, each apk on `alpine:3.21`: installs alone, no other role binary, `dpkg -S` / `apk info -W` name the package as owner of the binary and the alias, alias resolves, `--help`, clean removal; all three co-install and removing one leaves the others | real runtime |
| `images` arm64 | all three images: intended entrypoint, one executable, `--help` | real runtime |
| `forward` arm64 | 6 cases: role images, role `.deb`s, role `.apk`s (Serve and Access each in a container with only that role) and the all-in-one image, `.deb` and `.apk` (`serve` and `access` subcommands); echo target in a third container; client bytes returned unchanged | real runtime, public n0 relays |
| `cargo test`, `clippy -D warnings`, `fmt`, `goreleaser check` | pass | |

`forward` uses the public n0 relays because release binaries accept only `https://`
relay URLs, so a local plain-HTTP relay cannot be used. It is opt-in and not part of
`snapshot.yml` (needs Internet and the Docker daemon of the host). The in-process
relay variant is `tests/role_binaries.rs` (9 tests, pass).

## Limits

- Image digests and multi-arch manifests are not recorded here; `images` checks entrypoint and contents of the local arm64 image. Immutable artifact identities are bound in #97.

- **Windows x64/ARM64**: no Windows binary, archive or package is built or run.
  Windows distribution is owned by #93 (PowerShell installer), #100 (signed
  artifacts for the supported release matrix) and #97 (real Windows runs). The
  acceptance text for 94.AC2 says "where applicable"; there is no Windows
  package or container format approved in the spec (no Windows container promise
  in IMAGE-CREATE-01). Not claimed as verified.
- **macOS**: the Homebrew cask stays all-in-one; per-role casks were removed in
  #116 and no spec text requires them. Not verified here.
- amd64 and riscv64: package contents verified statically only. `snapshot.yml`
  runs the runtime checks on the amd64 runner but this change has not yet been
  observed green there (UNVERIFIED until the merge snapshot); riscv64 has no runner.
- Package upgrade (old all-in-one package to this one, where `it` becomes
  package-owned and the unit disappears) is UNVERIFIED; legacy migration is #96.
- `redesign-acceptance.md` cases PACKAGE-CREATE-01 and IMAGE-CREATE-01 stay
  `NOT_RUN` for final-artifact acceptance (#97).
