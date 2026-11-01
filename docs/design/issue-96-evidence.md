# Issue #96 — package and container update ownership

Baseline `bf54453` (origin/main). Branch `redesign/issue-96-update-ownership`. Blockers #94 and #95 closed at pickup. Policy: owner decision recorded on the issue (comment 6556642816), spec DIST-04 and the amended AC-DIST-07 row.

## Acceptance map

| Criterion | Evidence | Status |
|---|---|---|
| 96.AC1 package-owned artifacts updated through the manager; container update is redeployment | `tests/update_ownership.rs`: package file untouched by default and with `-i`; replaced only with `--yes`; Homebrew same; container never overwritten even with `-y`; real binary with `IROH_TUNNEL_CONTAINER=1` prints redeploy guidance and leaves the file unchanged. Real Linux arm64 (Debian 12 container): `/usr/bin/iroh-tunnel-serve` prints `apt-get install --only-upgrade` / `apk upgrade`, exit 0, file hash unchanged, also with `-i`; `/usr/local/bin` + `IROH_TUNNEL_CONTAINER=1 -i -y` unchanged | Pass (path-based detection) |
| 96.AC2 legacy bundled package does not silently update unrelated binaries | The legacy all-in-one package (`/usr/bin/iroh-tunnel`) falls under the same package rule; `only_the_invoked_file_changes_when_a_package_owner_is_overridden` | **PARTIAL**: no real legacy package fixture was run |
| 96.CL1 package, container and standalone paths together with the #95 updater; aliases | Standalone path unchanged and still passes (`tests/update.rs`, 14 tests); alias symlink resolved by `current_exe().canonicalize()` so only the target changes | Aliases: Unix by design; no dedicated alias test yet |
| 96.CL2 approved decisions only | Decisions in the issue comment; Windows alias and Windows package managers stay DIST-03 | Pass |
| Images set `IROH_TUNNEL_CONTAINER=1` | `Dockerfile`, `Dockerfile.goreleaser`, `.serve`, `.access`; `packaging/verify-dist.sh images` now runs `update --yes` in each image and expects the redeploy guidance | **Not run**: images are built by `snapshot.yml`; run it on this branch |

## Findings while verifying

- First version also treated `/.dockerenv` as a container marker. In a Debian container with a package-style install this overrode the package guidance (a hand-installed or packaged binary inside Docker is not an immutable image). Reproduced in the container, then the marker was removed; only the project's own variable marks an image.
- In a bare Debian container `update` fails with "No CA certificates were loaded from the system" until `ca-certificates` is installed. Not changed here (distroless images do not run `update` in place anyway); worth a hint if it comes up.

## Limits

- Detection is path- and variable-based, not a package database query (the decision allows either; a database query needs `dpkg`/`apk` at run time).
- No real `.deb`/`.apk` was installed for the `update` check; a file copied to `/usr/bin` stands in. `verify-dist.sh runtime` installs the real packages and could be extended with `update`.
- Windows and Homebrew were not run on real systems (Homebrew: path unit tests only).
