# install-release-packaging Specification

## Purpose

Define how releases are built, signed and delivered: the Unix and Windows installers, build version derivation, release archives, Linux packages, container images and the Homebrew cask. Updating an installed executable in place is owned by `self-update`.

## Requirements

### Requirement: Unix installer installs three role binaries with aliases
The system SHALL provide `install.sh`, which installs `iroh-tunnel`, `iroh-tunnel-serve` and `iroh-tunnel-access` into `/usr/local/bin` by default, each with a symlink alias (`it`, `its`, `ita`) that points at the binary by relative name.
Evidence: install.sh:47, install.sh:304-333
Confidence: inferred

#### Scenario: Default install
- **WHEN** `install.sh` runs against a release containing all three archives and the user confirms prompts
- **THEN** the three executables are installed and each alias runs the matching role binary

#### Scenario: Dry run
- **WHEN** `install.sh --dry-run` runs
- **THEN** it prints the target, download URL, destination directory and the three roles with aliases, and changes nothing

### Requirement: Unix installer platform detection
The system SHALL select the release target from the OS and CPU: Linux amd64/arm64 (glibc, or musl when `ldd` reports musl), riscv64 (musl build only), and macOS Apple silicon (`darwin-arm64`); it SHALL reject Intel macOS, other operating systems and unknown CPU architectures with an error. `IROH_TUNNEL_TARGET` overrides detection.
Evidence: install.sh:110-138
Confidence: inferred

#### Scenario: Intel macOS
- **WHEN** `install.sh` runs on a Darwin host that is not arm64
- **THEN** it fails and points at the Homebrew cask

#### Scenario: Target override
- **WHEN** `IROH_TUNNEL_TARGET=linux-arm64-musl` is set
- **THEN** the installer downloads archives for that target regardless of the host

### Requirement: Unix installer option and input validation
The system SHALL accept `--version`, `--bin-dir`, `--profile-dir`, `--yes`/`-y`, `--dry-run` and `--help`/`-h`, and SHALL reject unknown options, a missing option value, a non-absolute or unsafe-character `--bin-dir`/`--profile-dir`, and a `--version` that does not look like `X.Y.Z`. A leading `v` on the version is stripped.
Evidence: install.sh:82-106
Confidence: inferred

#### Scenario: Malformed version
- **WHEN** `--version` is given a value such as `abc`
- **THEN** the installer exits with an error and installs nothing

#### Scenario: Relative directory
- **WHEN** `--bin-dir` is a relative path
- **THEN** the installer exits with an error that the path must be absolute

### Requirement: Release resolution from the release host
The system SHALL download from `$IROH_TUNNEL_BASE_URL` (default `https://github.com/naicoi92/iroh-tunnel/releases`) at `/download/v<version>` for an explicit version or `/latest/download` otherwise, and for latest SHALL derive the version from the checksum file entry for the target. macOS uses `checksums-darwin.txt`; other targets use `checksums.txt`.
Evidence: install.sh:49-50, install.sh:190-230
Confidence: inferred

#### Scenario: Latest on Linux
- **WHEN** no `--version` is given
- **THEN** the version is read from the `iroh-tunnel-<version>-<target>.tar.gz` line of `checksums.txt`

#### Scenario: Latest has no archive for the target
- **WHEN** the checksum file lists no `iroh-tunnel` archive for the target
- **THEN** the installer fails with a message that the latest release has no archive for that target

### Requirement: Unix installer verifies checksums before changing anything
The system SHALL download every role archive and compare its SHA-256 with the checksum file, extract it, and require a regular (non-symlink) executable of the role name, all before touching the install directory; on any failure it SHALL exit non-zero and keep existing files. Only checksums are verified (no signature check), and the final output says so.
Evidence: install.sh:232-249, install.sh:343-344
Confidence: inferred

#### Scenario: Checksum mismatch
- **WHEN** one archive's digest differs from the checksum file
- **THEN** the installer reports a checksum mismatch, installs nothing and leaves existing files unchanged

#### Scenario: Release missing a role
- **WHEN** the checksum file does not list one of the three role archives
- **THEN** the installer fails and installs nothing

### Requirement: Unix installer asks consent before replacement and PATH change
The system SHALL ask before replacing differing install files or adding a PATH snippet, treat missing answers as no, and honor `--yes`; declining replacement SHALL change nothing, while declining PATH changes SHALL still install binaries.
Evidence: install.sh:160-183, install.sh:251-298
Confidence: inferred

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
Evidence: install.sh:213-222, install.sh:300-334
Confidence: inferred

#### Scenario: Unwritable directory
- **WHEN** the bin directory (or the nearest existing parent) is not writable
- **THEN** the installer fails before any download and suggests sudo or `--bin-dir`

#### Scenario: Success leaves no staging files
- **WHEN** an install completes
- **THEN** no `.new.*` staging files remain in the bin directory

### Requirement: Installers never touch services
The system SHALL NOT register, start, stop or change any service from either installer, and SHALL say so on completion.
Evidence: install.sh:34, install.sh:343, install.ps1:27, install.ps1:317
Confidence: inferred

#### Scenario: Service state unchanged
- **WHEN** an install completes
- **THEN** the output states no service was registered or changed, and no service-related files are created

### Requirement: Windows installer installs three executables system-wide
The system SHALL install all three executable variants without aliases into `$env:ProgramFiles\iroh-tunnel` or `-BinDir`, using windows-amd64/arm64 zip archives and honoring version, consent and dry-run options.
Evidence: install.ps1:50-62, install.ps1:107-120, install.ps1:167-192
Confidence: inferred

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
Evidence: install.ps1:122-161, install.ps1:194-326
Confidence: inferred

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
The system SHALL set the compile-time `APP_VERSION` to the version of a valid semver tag (optional `v` stripped) at HEAD, and otherwise to `<CARGO_PKG_VERSION>-dev`, including when there is no `.git`, the tag is not at HEAD, or the tag is not valid semver. The release stage SHALL take the artifact version from the tag, so file names match `--version`.
Evidence: build.rs:3-14, build.rs:32-62, .github/workflows/release.yml:36-90 (preflight), .github/workflows/release.yml:92-218 (build)
Confidence: inferred

#### Scenario: Tagged build
- **WHEN** HEAD is exactly on tag `v1.2.3`
- **THEN** `APP_VERSION` is `1.2.3`

#### Scenario: Untagged or invalid tag
- **WHEN** HEAD has no tag, or the tag is `v1bad`
- **THEN** `APP_VERSION` is `<Cargo version>-dev`

### Requirement: Release build matrix and archives
The system SHALL build and archive each of the three executables for all eight release targets in the release stage, with README and licenses in each role archive, and smoke-test version/help on non-zig targets on their native runners.
Evidence: .github/workflows/release.yml:92-218 (build job, design.md D5). Archive contents beyond the native smoke check are not verified by CI (owner decision: tooling is not tested).
Confidence: inferred

#### Scenario: Supported targets and naming
- **WHEN** release archives are produced
- **THEN** targets cover Linux GNU amd64/arm64, Linux musl amd64/arm64/riscv64gc, Darwin arm64 and Windows amd64/arm64; names are `<exe>-<version>-<os>-<arch>[-musl].tar.gz` or Windows `.zip`

#### Scenario: Archive contents
- **WHEN** a role archive is inspected
- **THEN** it contains its own executable plus README and licenses, and no other role executable, in the format for its platform (PE32+, Mach-O arm64)

#### Scenario: Native smoke check
- **WHEN** a non-zig target is built
- **THEN** each executable runs `--help`, `iroh-tunnel --version` contains the release version, and a failure fails the release

### Requirement: Linux packages are binary-only
The system SHALL ship co-installable binary-only role packages containing only a mode-0755 executable and its alias symlink; removing one SHALL NOT break other roles, and package-owned update SHALL leave binaries unchanged.
Evidence: packaging/nfpm.yaml:7-30, crates/host/src/update/. Not verified by CI (owner decision: tooling is not tested).
Confidence: inferred

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
The system SHALL build one distroless image per executable for amd64/arm64 from the signed glibc release binaries and publish it on every release, and SHALL direct in-container updates to redeploy instead of replacement.
Evidence: Dockerfile.release:14-28, Dockerfile.release.serve:7-17, Dockerfile.release.access:7-17, .github/workflows/release.yml:426-515 (images job, design.md D8). Image contents are not verified by CI (owner decision: tooling is not tested).
Confidence: inferred

#### Scenario: Runtime and promotion tags
- **WHEN** role images are published for a release
- **THEN** they use `gcr.io/distroless/cc-debian12`, only the role executable under `/usr/local/bin`, its entrypoint, `HOME=/tmp` and `IROH_TUNNEL_CONTAINER=1`, and are pushed as `ghcr.io/naicoi92/<image>` with tags `v<ver>`, `<ver>`, major.minor and major, plus stable-only `latest`

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
The system SHALL produce `checksums.txt` (Linux and Windows archives and packages) and `checksums-darwin.txt` (Darwin archives) on version tags, require the expected set (three role archives for each of the eight targets, three .deb for each of two glibc targets, three .apk for each of three musl targets, and nothing else), check every listed file against them, and sign both keyless with cosign before any publication; each signature bundle SHALL be non-empty.
Evidence: .github/workflows/release.yml:286-379 (sign job, design.md D7)
Confidence: inferred

#### Scenario: Signing identity and checksum names
- **WHEN** a `v*` release is signed
- **THEN** `checksums.txt` and `checksums-darwin.txt` receive keyless cosign `.sigstore.json` bundles under the `release.yml@refs/tags/<tag>` identity and the GitHub Actions issuer

#### Scenario: Complete set
- **WHEN** every role archive and package is present and matches its checksum line
- **THEN** the sign job signs both checksum files and publication may start

#### Scenario: Defective set
- **WHEN** an expected file is missing, an unexpected file is present, a listed digest does not match, or a signature bundle is empty
- **THEN** the sign job fails naming the file, and nothing is published

### Requirement: Signature verification of real release artifacts
The system SHALL, in the release run and before any publication, verify each signature bundle with `cosign verify-blob` against the pinned issuer `https://token.actions.githubusercontent.com` and the exact identity `https://github.com/naicoi92/iroh-tunnel/.github/workflows/release.yml@refs/tags/<tag>` that the shipped updater pins, and SHALL fail the release when verification fails.
Evidence: .github/workflows/release.yml:286-379 (cosign verify-blob in the sign job, design.md D7); the shipped verifier's own accept/reject logic stays covered by unit tests in crates/host/src/update/
Confidence: inferred

#### Scenario: Valid bundle
- **WHEN** a bundle produced by this run is verified with the pinned issuer and identity
- **THEN** verification succeeds and publication may start

#### Scenario: Tampered input
- **WHEN** `cosign verify-blob` fails for any bundle (changed payload, wrong bundle, wrong identity or issuer)
- **THEN** the sign job fails and no publish job starts

### Requirement: Homebrew cask rendering
The system SHALL render a Homebrew cask for `iroh-tunnel` that downloads the `darwin-arm64` tarball of the given version, requires arm64, links the `iroh-tunnel` binary, clears the quarantine attribute, creates the `it` symlink after install and removes it on uninstall.
Evidence: packaging/homebrew-cask.sh:1-42, .github/workflows/release.yml:519-561 (homebrew job)
Confidence: inferred

#### Scenario: Render
- **WHEN** `homebrew-cask.sh 1.2.3 <sha>` runs
- **THEN** it prints a cask with that version, sha256 and the release download URL, and fails if either argument is missing

### Requirement: Windows installer protects the install directory it creates
The Windows installer SHALL, from an elevated session, leave an install directory that it creates (the default or a `-BinDir` path) writable only by SYSTEM, the Administrators group and TrustedInstaller, owned by the Administrators group, with no entry granting the installing user's own SID write access. It SHALL NOT change the owner or access of a directory that already existed.

#### Scenario: Fresh default install
- **WHEN** `install.ps1` creates `$env:ProgramFiles\iroh-tunnel` from an elevated session
- **THEN** the directory and the installed executables pass the Windows system-service binary trust check

#### Scenario: Parent grants write to Users
- **WHEN** `install.ps1` creates `-BinDir` from an elevated session under a parent that grants Users modify access
- **THEN** the created directory grants Users only read and execute, and no untrusted principal holds a write right

#### Scenario: Existing directory untouched
- **WHEN** `install.ps1` installs into a directory that already exists
- **THEN** its owner and access entries are unchanged

### Requirement: Non-elevated installs keep default directory access
The Windows installer SHALL, from a non-elevated session, create a new install directory with default access, as before this change. A binary installed there SHALL remain refused by the system-service binary trust check.

#### Scenario: Non-elevated install into a new directory
- **WHEN** `install.ps1 -BinDir <new dir under the user's profile>` runs from a non-elevated session
- **THEN** the install succeeds and the directory's owner is not changed to Administrators
