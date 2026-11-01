# Issue #96 — package and container update ownership

Baseline `bf54453` (origin/main). Branch `redesign/issue-96-update-ownership`. Blockers #94 and #95 closed at pickup. Policy: owner decision recorded on the issue (comment 6556642816), spec DIST-04 and the amended AC-DIST-07 row.

## Acceptance map

| Criterion | Evidence | Status |
|---|---|---|
| 96.AC1 package-owned artifacts updated through the manager; container update is redeployment | `tests/update_ownership.rs`: package file untouched by default and with `-i`; replaced only with `--yes`; Homebrew same; container never overwritten even with `-y`; real binary with `IROH_TUNNEL_CONTAINER=1` prints redeploy guidance and leaves the file unchanged. Real Linux arm64 (Debian 12 container): `/usr/bin/iroh-tunnel-serve` prints `apt-get install --only-upgrade` / `apk upgrade`, exit 0, file hash unchanged, also with `-i`; `/usr/local/bin` + `IROH_TUNNEL_CONTAINER=1 -i -y` unchanged | Pass (path-based detection) |
| 96.AC2 legacy bundled package does not silently update unrelated binaries | The legacy all-in-one package (`/usr/bin/iroh-tunnel`) falls under the same package rule; `only_the_invoked_file_changes_when_a_package_owner_is_overridden` | Pass for the current all-in-one package (`iroh-tunnel_*.deb/.apk`) with a real install; a historical bundled release package was not available |
| 96.CL1 package, container and standalone paths together with the #95 updater; aliases | Standalone path unchanged and still passes (`tests/update.rs`, 14 tests); alias symlink resolved by `current_exe().canonicalize()` so only the target changes | Pass on real `.deb`/`.apk` (alias `its`/`it`/`ita`); Windows alias stays DIST-03 |
| 96.CL2 approved decisions only | Decisions in the issue comment; Windows alias and Windows package managers stay DIST-03 | Pass |
| Images set `IROH_TUNNEL_CONTAINER=1` | `Dockerfile`, `Dockerfile.goreleaser`, `.serve`, `.access`; `packaging/verify-dist.sh images` now runs `update --yes` in each image and expects the redeploy guidance | Run locally: `Dockerfile.goreleaser.serve` built with a Linux arm64 binary (distroless): `docker run iroh96-serve update --yes` and `update -i` print the redeploy guidance, exit 0, `IROH_TUNNEL_CONTAINER=1` in the image env. `verify-dist.sh images` itself was not run (CI does not call it; needs all three images); the other three Dockerfiles carry the same line |

## Real-package e2e (Docker, native arm64, packages built by `snapshot` run 41298054818 at `4879644`)

Real `.deb`/`.apk` from the snapshot, installed with `dpkg -i` / `apk add`; the binaries contain the ownership code. Ubuntu 24.04 and Alpine 3.21 both: 10/10 checks.

| Check | Ubuntu (.deb) | Alpine (.apk) |
|---|---|---|
| `iroh-tunnel-serve update`: names the package, prints `apt-get install --only-upgrade` / `apk upgrade`, exit 0, file hash unchanged | pass | pass |
| alias `its update` resolves to the serve package, unchanged | pass | pass |
| `update -i` does not override ownership | pass | pass |
| `ita update` leaves both access and serve unchanged | pass | pass |
| `--yes -i --to 9.9.9` (local release) replaces only serve; access hash unchanged; warning that the package database will differ | pass | pass |
| legacy all-in-one package (`iroh-tunnel`, alias `it`) beside a role package: guidance, neither binary changed | pass | pass |
| after `--yes`: no staging file in `/usr/bin`; `dpkg -r` / `apk del` removes the replaced file and the alias; reinstalling restores the packaged binary | pass | pass |

`packaging/verify-dist.sh runtime` now repeats the update check for each package, full name and alias (both `.deb` on debian:12-slim and `.apk` on alpine:3.21): all green on the snapshot packages; red against the v0.4.2 binary (`unrecognized subcommand 'update'`, exit 2).

## Findings while verifying

- First version also treated `/.dockerenv` as a container marker. In a Debian container with a package-style install this overrode the package guidance (a hand-installed or packaged binary inside Docker is not an immutable image). Reproduced in the container, then the marker was removed; only the project's own variable marks an image.
- In a bare Debian container `update` fails with "No CA certificates were loaded from the system" until `ca-certificates` is installed. Not changed here (distroless images do not run `update` in place anyway); worth a hint if it comes up.

## Limits

- Detection is path- and variable-based, not a package database query (the decision allows either; a database query needs `dpkg`/`apk` at run time).
- Docker ran natively on arm64 only; amd64 and riscv64 packages were not run (no native runner).
- Windows and Homebrew were not run on real systems (Homebrew: path unit tests only).
