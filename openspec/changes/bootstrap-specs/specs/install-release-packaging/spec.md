## ADDED Requirements

### Requirement: Unix installer installs three role binaries with aliases
The system SHALL provide `install.sh`, which installs `iroh-tunnel`, `iroh-tunnel-serve` and `iroh-tunnel-access` into `/usr/local/bin` by default, each with a symlink alias (`it`, `its`, `ita`) that points at the binary by relative name.
Evidence: install.sh:47, install.sh:304-333, tests/installer.rs:232, tests/installer.rs:253

#### Scenario: Default install
- **WHEN** `install.sh` runs against a release containing all three archives and the user confirms prompts
- **THEN** the three executables are installed and each alias runs the matching role binary

#### Scenario: Dry run
- **WHEN** `install.sh --dry-run` runs
- **THEN** it prints the target, download URL, destination directory and the three roles with aliases, and changes nothing

### Requirement: Unix installer platform detection
The system SHALL select the release target from the OS and CPU: Linux amd64/arm64 (glibc, or musl when `ldd` reports musl), riscv64 (musl build only), and macOS Apple silicon (`darwin-arm64`); it SHALL reject Intel macOS, other operating systems and unknown CPU architectures with an error. `IROH_TUNNEL_TARGET` overrides detection.
Evidence: install.sh:110-138, tests/installer.rs:535-557

#### Scenario: Intel macOS
- **WHEN** `install.sh` runs on a Darwin host that is not arm64
- **THEN** it fails and points at the Homebrew cask

#### Scenario: Target override
- **WHEN** `IROH_TUNNEL_TARGET=linux-arm64-musl` is set
- **THEN** the installer downloads archives for that target regardless of the host

### Requirement: Unix installer option and input validation
The system SHALL accept `--version`, `--bin-dir`, `--profile-dir`, `--yes`/`-y`, `--dry-run` and `--help`/`-h`, and SHALL reject unknown options, a missing option value, a non-absolute or unsafe-character `--bin-dir`/`--profile-dir`, and a `--version` that does not look like `X.Y.Z`. A leading `v` on the version is stripped.
Evidence: install.sh:82-106, tests/installer.rs:500

#### Scenario: Malformed version
- **WHEN** `--version` is given a value such as `abc`
- **THEN** the installer exits with an error and installs nothing

#### Scenario: Relative directory
- **WHEN** `--bin-dir` is a relative path
- **THEN** the installer exits with an error that the path must be absolute

### Requirement: Release resolution from the release host
The system SHALL download from `$IROH_TUNNEL_BASE_URL` (default `https://github.com/naicoi92/iroh-tunnel/releases`) at `/download/v<version>` for an explicit version or `/latest/download` otherwise, and for latest SHALL derive the version from the checksum file entry for the target. macOS uses `checksums-darwin.txt`; other targets use `checksums.txt`.
Evidence: install.sh:49-50, install.sh:190-230, tests/installer.rs:271, tests/installer.rs:557

#### Scenario: Latest on Linux
- **WHEN** no `--version` is given
- **THEN** the version is read from the `iroh-tunnel-<version>-<target>.tar.gz` line of `checksums.txt`

#### Scenario: Latest has no archive for the target
- **WHEN** the checksum file lists no `iroh-tunnel` archive for the target
- **THEN** the installer fails with a message that the latest release has no archive for that target

### Requirement: Unix installer verifies checksums before changing anything
The system SHALL download every role archive and compare its SHA-256 with the checksum file, extract it, and require a regular (non-symlink) executable of the role name, all before touching the install directory; on any failure it SHALL exit non-zero and keep existing files. Only checksums are verified (no signature check), and the final output says so.
Evidence: install.sh:232-249, install.sh:343-344, tests/installer.rs:402, tests/installer.rs:419

#### Scenario: Checksum mismatch
- **WHEN** one archive's digest differs from the checksum file
- **THEN** the installer reports a checksum mismatch, installs nothing and leaves existing files unchanged

#### Scenario: Release missing a role
- **WHEN** the checksum file does not list one of the three role archives
- **THEN** the installer fails and installs nothing

### Requirement: Unix installer asks consent before replacement and PATH change
The system SHALL ask before replacing differing install files or adding a PATH snippet, treat missing answers as no, and honor `--yes`; declining replacement SHALL change nothing, while declining PATH changes SHALL still install binaries.
Evidence: install.sh:160-183, install.sh:251-298, tests/installer.rs:286-392, tests/installer.rs:572-589

#### Scenario: Conflict and PATH rules
- **WHEN** installation examines destinations and PATH
- **THEN** differing binaries, incorrect aliases and differing snippets require consent; identical files do not, real directories fail, and writable profile directories use `/etc/profile.d/iroh-tunnel.sh` or macOS `/etc/paths.d/iroh-tunnel`

#### Scenario: PATH consent declined
- **WHEN** PATH addition is declined
- **THEN** binaries are installed and manual PATH guidance is printed

#### Scenario: Declined conflict
- **WHEN** a differing binary exists and the answer is "n"
- **THEN** the installer exits with "declined; nothing was changed." and no file is modified

#### Scenario: Reinstall same release
- **WHEN** the same release is already installed
- **THEN** no question is asked

#### Scenario: No terminal
- **WHEN** a question would be asked and no answer source is readable and `--yes` is absent
- **THEN** the answer is treated as no

#### Scenario: Directory already on PATH
- **WHEN** the bin directory is already on `PATH`
- **THEN** no PATH prompt appears

### Requirement: Unix installer replaces files via staging
The system SHALL stage every binary, alias and PATH snippet next to its destination before renaming any, replace an existing alias symlink (including one pointing at a directory), fail on write permissions before downloading, remove its temporary files on exit, and leave no staging files after success.
Evidence: install.sh:213-222, install.sh:300-334, tests/installer.rs:435, tests/installer.rs:483, tests/installer.rs:517

#### Scenario: Unwritable directory
- **WHEN** the bin directory (or the nearest existing parent) is not writable
- **THEN** the installer fails before any download and suggests sudo or `--bin-dir`

#### Scenario: Success leaves no staging files
- **WHEN** an install completes
- **THEN** no `.new.*` staging files remain in the bin directory

### Requirement: Installers never touch services
The system SHALL NOT register, start, stop or change any service from either installer, and SHALL say so on completion.
Evidence: install.sh:34, install.sh:343, install.ps1:27, install.ps1:317, tests/installer.rs:458, tests/installer_ps.rs:429

#### Scenario: Service state unchanged
- **WHEN** an install completes
- **THEN** the output states no service was registered or changed, and no service-related files are created

### Requirement: Windows installer installs three executables system-wide
The system SHALL install all three executable variants without aliases into `$env:ProgramFiles\iroh-tunnel` or `-BinDir`, using windows-amd64/arm64 zip archives and honoring version, consent and dry-run options.
Evidence: install.ps1:50-62, install.ps1:107-120, install.ps1:167-192, tests/installer_ps.rs:219, tests/installer_ps.rs:239, tests/installer_ps.rs:257

#### Scenario: Installer controls
- **WHEN** `-Version`, `-Yes`, `-DryRun` or installer environment overrides are supplied
- **THEN** version selection, consent and dry-run follow the shell installer equivalents, using `IROH_TUNNEL_BASE_URL`, `IROH_TUNNEL_TARGET` and `IROH_TUNNEL_PROMPT_INPUT`

#### Scenario: Default flow
- **WHEN** `install.ps1` runs against a release with all three zips
- **THEN** all three `.exe` files are installed and no alias files are created

#### Scenario: Unsupported platform
- **WHEN** PowerShell Core runs on a non-Windows host without a target override
- **THEN** the script fails and directs the user to `install.sh`

#### Scenario: Malformed version
- **WHEN** `-Version` does not look like `X.Y.Z`
- **THEN** the script fails and installs nothing

### Requirement: Windows installer verification, consent and PATH handling
The system SHALL verify archive SHA-256 before installation and ask before executable replacement or machine-PATH changes; missing answers mean no, and installation failure SHALL remove a directory created by this run.
Evidence: install.ps1:122-161, install.ps1:194-326, tests/installer_ps.rs:273-416, tests/installer_ps.rs:469

#### Scenario: PATH normalization and write failure
- **WHEN** machine PATH is updated
- **THEN** matching is case-insensitive with trailing slashes ignored, failed PATH writes are reported without failing the binary install, and `IROH_TUNNEL_PATH_FILE` can redirect PATH storage

#### Scenario: Executable replacement fails
- **WHEN** a rename fails because an executable is running
- **THEN** the error instructs the operator to stop the process

#### Scenario: Declined PATH change
- **WHEN** the directory is not on machine PATH and the answer is no
- **THEN** executables are installed and the script prints that PATH was not changed

#### Scenario: Checksum mismatch
- **WHEN** a zip digest differs from the checksum file
- **THEN** nothing is installed and existing files are kept

#### Scenario: Accepted PATH change
- **WHEN** the answer is yes
- **THEN** the directory is appended to the machine PATH exactly once

### Requirement: Build version derived from git tag
The system SHALL set the compile-time `APP_VERSION` to the version of a valid semver tag (optional `v` stripped) at HEAD, and otherwise to `<CARGO_PKG_VERSION>-dev`, including when there is no `.git`, the tag is not at HEAD, or the tag is not valid semver.
Evidence: build.rs:3-14, build.rs:32-62, .github/workflows/build.yml:50-59
Confidence: inferred

#### Scenario: Tagged build
- **WHEN** HEAD is exactly on tag `v1.2.3`
- **THEN** `APP_VERSION` is `1.2.3`

#### Scenario: Untagged or invalid tag
- **WHEN** HEAD has no tag, or the tag is `v1bad`
- **THEN** `APP_VERSION` is `<Cargo version>-dev`

### Requirement: Release build matrix and archives
The system SHALL build and archive each of the three executables for all eight release targets, with README and licenses in each role archive, and smoke-test version/help on non-zig targets.
Evidence: .github/workflows/build.yml:1-27, .github/workflows/build.yml:73-187, packaging/verify-dist.sh:144-208

#### Scenario: Supported targets and naming
- **WHEN** release archives are produced
- **THEN** targets cover Linux GNU amd64/arm64, Linux musl amd64/arm64/riscv64gc, Darwin arm64 and Windows amd64/arm64; names are `<exe>-<version>-<os>-<arch>[-musl].tar.gz` or Windows `.zip`

#### Scenario: Archive contents
- **WHEN** a role archive is inspected
- **THEN** it contains its own executable and no other role executable, in the format for its platform (PE32+, Mach-O arm64)

### Requirement: Linux packages are binary-only
The system SHALL ship co-installable binary-only role packages containing only a mode-0755 executable and its alias symlink; removing one SHALL NOT break other roles, and package-owned update SHALL leave binaries unchanged.
Evidence: packaging/nfpm.yaml:7-30, .github/workflows/package.yml:45-68, packaging/verify-dist.sh:75-142, packaging/verify-dist.sh:223-299

#### Scenario: Package targets
- **WHEN** Linux packages are built
- **THEN** deb targets are glibc amd64/arm64 and apk targets are musl amd64/arm64/riscv64, with `/usr/bin/<name>` and `/usr/bin/<alias>` only and no unit, config directory or maintainer script

#### Scenario: Exact members
- **WHEN** a package is listed
- **THEN** its members are exactly the binary and the alias link

#### Scenario: Install, run, remove
- **WHEN** one package is installed alone in a clean Debian or Alpine container
- **THEN** its alias runs `--help`, the package database owns both paths, `update` reports package ownership without modifying the binary, and removal leaves nothing behind

#### Scenario: Co-install
- **WHEN** all three packages are installed and one is removed
- **THEN** the other two aliases still run

### Requirement: Role container images
The system SHALL build one distroless image per executable for amd64/arm64, publish only on promotion, and direct in-container updates to redeploy instead of replacement.
Evidence: Dockerfile.release:14-28, Dockerfile.release.serve:7-17, Dockerfile.release.access:7-17, .github/workflows/docker.yml:35-125, packaging/verify-dist.sh:301-335

#### Scenario: Runtime and promotion tags
- **WHEN** role images are built and promoted
- **THEN** they use `gcr.io/distroless/cc-debian12`, only the role executable under `/usr/local/bin`, its entrypoint, `HOME=/tmp` and `IROH_TUNNEL_CONTAINER=1`; promotion pushes `ghcr.io/naicoi92/<image>` tags `v<ver>`, `<ver>`, major.minor and major, plus stable-only `latest`; rehearsals never push

#### Scenario: Image content
- **WHEN** a role image is inspected
- **THEN** its only binary is the role executable, it runs `--help`, and `update --yes` prints redeploy guidance

#### Scenario: Pre-release tag
- **WHEN** the version contains `-`
- **THEN** the `latest` tag is not applied

### Requirement: Source Dockerfile and compose demo
The system SHALL provide a top-level `Dockerfile` that builds `iroh-tunnel` from source with Rust 1.91 into a distroless runtime with `IROH_TUNNEL_CONTAINER=1`, and a `docker-compose.yml` demo that tunnels nginx through `serve` and `access` services, where `access` is in the `demo` profile and published only on `127.0.0.1:8080`, with example configs mounted read-only.
Evidence: Dockerfile:14-29, docker-compose.yml:1-41
Confidence: inferred

#### Scenario: Demo start
- **WHEN** `docker compose up web serve` runs
- **THEN** serve starts with the example config and `access` does not start unless its profile is requested

### Requirement: Signed, complete release set
The system SHALL produce signed Linux/Windows and Darwin checksum sets on version tags and reject missing, mismatched, unlisted or unexpected release artifacts and empty signature bundles.
Evidence: .github/workflows/release.yml:93-126, packaging/verify-release-set.sh:1-80, tests/release_artifacts.rs:104-190

#### Scenario: Signing identity and checksum names
- **WHEN** a `v*` release is signed
- **THEN** `checksums.txt` and `checksums-darwin.txt` receive keyless cosign `.sigstore.json` bundles under the release.yml identity and GitHub Actions issuer before complete-set verification

#### Scenario: Complete set
- **WHEN** all role archives, packages, both checksum files and bundles are present and consistent
- **THEN** the script prints an ok line and exits 0

#### Scenario: Defective set
- **WHEN** an artifact is missing, its digest differs, it is unlisted, or an extra file is present
- **THEN** the script reports a FAIL naming the file and exits non-zero

### Requirement: Signature verification of real release artifacts
The system SHALL verify the real signed bundles with the shipped `SigstoreVerifier` against the pinned issuer and release identity, and SHALL reject a wrong identity, wrong issuer, changed payload, swapped bundle, and missing or invalid verification material. These tests are `#[ignore]`d and run with `RELEASE_DIST` and `RELEASE_TAG` by the release and promote workflows.
Evidence: tests/release_artifacts.rs:1-12, tests/release_artifacts.rs:242-351, .github/workflows/release.yml:128-163

#### Scenario: Valid bundle
- **WHEN** a bundle is verified with the pinned issuer and `release.yml@refs/tags/<tag>` identity
- **THEN** verification succeeds

#### Scenario: Tampered input
- **WHEN** the payload is changed or the bundle belongs to another file
- **THEN** verification fails

### Requirement: Tag releases create drafts only
The system SHALL, on a `v*` tag push or manual dispatch, run lint/test, build, package, push-less image rehearsal, sign and verify, and then upload to a draft GitHub release (marked pre-release when the version contains `-`), re-running replaces assets of an existing draft, and it SHALL refuse to modify an already published release.
Evidence: .github/workflows/release.yml:1-62, .github/workflows/release.yml:165-204
Confidence: inferred

#### Scenario: Published release exists
- **WHEN** the tag's release is already published
- **THEN** the draft job exits with an error and changes nothing

### Requirement: Manual promotion publishes a verified draft
The system SHALL, via manually dispatched `promote.yml` with a `vX.Y.Z[-pre]` tag and an in-repository evidence URL, require the release to be a draft, re-verify the complete set and signatures, publish it, update the Homebrew cask for non-pre-release versions only, and push the three role images from the signed archives, verifying their checksums.
Evidence: .github/workflows/promote.yml:1-149, .github/workflows/docker.yml:66-86
Confidence: inferred

#### Scenario: Invalid inputs
- **WHEN** the tag is not `vX.Y.Z[-pre]` or the evidence URL is outside the repository
- **THEN** the workflow fails before doing anything

#### Scenario: Pre-release
- **WHEN** the tag contains `-`
- **THEN** the release is published but the Homebrew cask is not updated

#### Scenario: Not a draft
- **WHEN** the release for the tag is not a draft
- **THEN** the workflow fails

### Requirement: Homebrew cask rendering
The system SHALL render a Homebrew cask for `iroh-tunnel` that downloads the `darwin-arm64` tarball of the given version, requires arm64, links the `iroh-tunnel` binary, clears the quarantine attribute, creates the `it` symlink after install and removes it on uninstall.
Evidence: packaging/homebrew-cask.sh:1-42, .github/workflows/promote.yml:111-118
Confidence: inferred

#### Scenario: Render
- **WHEN** `homebrew-cask.sh 1.2.3 <sha>` runs
- **THEN** it prints a cask with that version, sha256 and the release download URL, and fails if either argument is missing

### Requirement: Pre-merge and rehearsal CI
The system SHALL run path-relevant PR checks behind an always-reporting `ci-ok` gate and rehearse build/package/images without publication on manual dispatch or relevant main pushes.
Evidence: .github/workflows/ci.yml:1-96, .github/workflows/checks.yml:1-48, .github/workflows/snapshot.yml:1-57
Confidence: inferred

#### Scenario: Relevant checks and failures
- **WHEN** code, installers, scripts or workflows change
- **THEN** relevant fmt, clippy, Linux/Windows tests, shellcheck and actionlint run, and any failed or cancelled job fails `ci-ok`; main build/packaging changes trigger the push-less snapshot rehearsal

#### Scenario: Docs-only PR
- **WHEN** a pull request changes no code or script paths
- **THEN** heavy jobs are skipped and `ci-ok` passes

### Requirement: Packaged release smoke test
The system SHALL verify signed release archive digests before forwarding real TCP and UDP traffic with production executables over public HTTPS relays on seven smoke-test runners.
Evidence: .github/workflows/release-smoke.yml:1-110, tests/release_smoke.rs:1-22, tests/release_smoke.rs:398

#### Scenario: Manual smoke invocation
- **WHEN** release-smoke.yml is dispatched for a release run with `RELEASE_SMOKE_BIN_DIR`
- **THEN** ignored release smoke tests execute extracted binaries and record digests, OS, architecture and commands

#### Scenario: Digest mismatch
- **WHEN** an archive's digest differs from the checksum file
- **THEN** the smoke job fails before running the executables
