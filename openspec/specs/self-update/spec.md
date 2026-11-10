# self-update Specification

## Purpose

Define the `update` command, which replaces only the invoked executable with a verified release: version selection, signature and checksum verification, atomic replacement, and refusal for managed installs and container images. Installer behaviour is owned by `install-release-packaging`.

## Requirements

### Requirement: Update replaces only the invoked executable
The system SHALL, on `update`, replace only the running standalone executable with the selected release's executable, leaving sibling files untouched, and SHALL print a notice to restart any running tunnel.
Evidence: src/update.rs:153-256, tests/update.rs:114-134, tests/update.rs:209-243, tests/update.rs:292-306

#### Scenario: Sibling file untouched
- **WHEN** `update` succeeds for `iroh-tunnel-serve` installed beside `iroh-tunnel-access`
- **THEN** only `iroh-tunnel-serve` has new content and the sibling is unchanged

#### Scenario: Restart notice
- **WHEN** an update completes
- **THEN** stdout reports "updated <path> from <old> to <new>" and tells the operator to restart any running tunnel

### Requirement: Latest release is the default and a version can be pinned
The system SHALL install the single version listed by the `latest` release when `--to` is absent, and SHALL install the named release when `--to VERSION` (with or without a leading `v`) is given.
Evidence: src/update.rs:165-168, src/update.rs:198-200, src/update.rs:404-420, tests/update.rs:137-156, tests/update.rs:182-197

#### Scenario: Explicit version
- **WHEN** `update -i --to 9.9.8` is run while 9.9.9 is latest
- **THEN** the installed executable is the 9.9.8 build

#### Scenario: Unknown version
- **WHEN** `--to` names a version that is not published
- **THEN** the command fails and the installed executable is unchanged

### Requirement: Version argument is validated before any request
The system SHALL reject a `--to` value that is not `MAJOR.MINOR.PATCH` (optionally `v`-prefixed, with optional pre-release/build suffix) with an "invalid version" error before making any network request.
Evidence: src/update.rs:287-297, tests/update.rs:200-206

#### Scenario: Path traversal value
- **WHEN** `update --to ../x` is run
- **THEN** the command fails with "invalid version" without contacting the release server

### Requirement: Release location is configurable
The system SHALL fetch releases from `https://github.com/naicoi92/iroh-tunnel/releases` unless `IROH_TUNNEL_BASE_URL` overrides the base URL, using `<base>/latest/download` or `<base>/download/v<version>`.
Evidence: src/update.rs:22-23, src/update.rs:131-134, src/update.rs:165-168, tests/common/release_server.rs:11-23, tests/update.rs:99-106

#### Scenario: Base URL override
- **WHEN** `IROH_TUNNEL_BASE_URL` points at a local server
- **THEN** the update downloads from that server

### Requirement: Release signature is verified by default
The system SHALL, unless `--insecure` is given, verify the checksums and Sigstore bundle against the project release.yml version-tag identity and GitHub Actions issuer before trusting them; failure SHALL leave the executable unchanged.
Evidence: src/update.rs:34-103, src/update.rs:172-196, src/update.rs:690-733, tests/update.rs:159-179, tests/update.rs:322-348, tests/update.rs:432-466

#### Scenario: Missing verification material
- **WHEN** the signature bundle is absent or invalid
- **THEN** the command exits with code 1, the error mentions the signature, and the old executable is intact

#### Scenario: Foreign signer
- **WHEN** the bundle is a genuine Sigstore bundle not signed by the project's release workflow
- **THEN** the update is rejected and the installed file is unchanged

#### Scenario: Wrong identity via verifier
- **WHEN** the verifier rejects the identity
- **THEN** the error contains the verifier message and the old executable is intact

### Requirement: Archive checksum is verified
The system SHALL compare the SHA-256 of the downloaded archive with the entry in the signed checksums file and, on mismatch or when the release does not list the archive, SHALL fail and leave the installed executable unchanged.
Evidence: src/update.rs:215-236, tests/update.rs:308-320

#### Scenario: Tampered archive
- **WHEN** the archive bytes differ from the listed checksum
- **THEN** the error contains "checksum mismatch" and the old executable is intact

#### Scenario: Archive not published
- **WHEN** the checksums file has no entry for the role archive
- **THEN** the update fails with "does not publish"

### Requirement: Insecure mode bypasses verification with a warning
The system SHALL, with `--insecure` (`-i`), print a warning to stderr, skip signature and checksum verification, and skip fetching the checksums file entirely when `--to` is also given.
Evidence: src/update.rs:158-163, src/update.rs:169-196, src/cli.rs:65-68, tests/update.rs:114-134, tests/update.rs:351-362, tests/update.rs:386-398

#### Scenario: Insecure latest
- **WHEN** `update --insecure` runs
- **THEN** stderr contains the `--insecure` warning and the latest release is installed

#### Scenario: Tampered payload with insecure
- **WHEN** the archive does not match its checksum and `--insecure` is set
- **THEN** the update still installs

#### Scenario: Pinned insecure without checksums
- **WHEN** `--insecure --to 9.9.9` is run and the release has no checksums file
- **THEN** the update succeeds

### Requirement: Already-current and newer versions are not changed
The system SHALL report "already at <version>" without replacing when the target version equals the running version, and SHALL NOT downgrade when the latest release is older than the running version (comparing the numeric core and ignoring a `-dev` suffix) unless `--to` is explicit.
Evidence: src/update.rs:201-214, src/update.rs:324-335, src/update.rs:653-659, tests/update.rs:364-384

#### Scenario: Latest older than running
- **WHEN** running 0.1.0 and the latest release is 0.0.5
- **THEN** a message says nothing to do and suggests `--to`, and the file is unchanged

### Requirement: Release asset naming and platform selection
The system SHALL select the archive `<exe>-<version>-<target>.tar.gz` (`.zip` for Windows targets) for the platform ids `darwin-arm64`, `windows-amd64`, `windows-arm64`, `linux-amd64[-musl]`, `linux-arm64[-musl]`, `linux-riscv64gc-musl`, use `checksums-darwin.txt` for Darwin targets and `checksums.txt` otherwise, and SHALL fail with a message pointing to the installer on other platforms.
Evidence: src/update.rs:337-390, tests/update.rs:37-84, tests/update_zip_windows.rs:87-103

#### Scenario: Unsupported platform
- **WHEN** the host matches no published target
- **THEN** the command fails saying no release is published and to use the installer

#### Scenario: Windows zip
- **WHEN** a `windows-amd64` or `windows-arm64` release is a `.zip` containing `<exe>.exe`
- **THEN** the installed `.exe` is replaced and siblings are unchanged

### Requirement: Archive extraction is bounded and single-entry
The system SHALL read exactly one regular-file entry named after the executable from the archive in memory, and SHALL fail without touching the installed file when the entry is missing, not a regular file, empty, or exceeds size limits (checksums 1 MiB, archive 256 MiB, executable 512 MiB).
Evidence: src/update.rs:24-27, src/update.rs:459-501, tests/update_zip_windows.rs:105-136

#### Scenario: Missing entry
- **WHEN** the zip lacks the expected `<exe>.exe`
- **THEN** the error contains "does not contain" and the old executable is intact

#### Scenario: Traversal entry name
- **WHEN** the archive entry is named `..\<exe>.exe`
- **THEN** it is not extracted and the old executable is intact

### Requirement: Replacement is atomic and preserves permissions
The system SHALL stage the new executable beside the target (exclusive creation, same directory), give it the existing file's permissions, and rename it over the target, leaving no staging file on success or failure and leaving the old executable if any step fails.
Evidence: src/update.rs:504-590, src/update.rs:596-651, src/update.rs:673-685

#### Scenario: Permissions preserved
- **WHEN** an executable with mode 0755 is replaced
- **THEN** the new file has mode 0755 and no staging file remains

#### Scenario: Missing target
- **WHEN** the target path does not exist
- **THEN** replacement errors and creates nothing

#### Scenario: Unique staging
- **WHEN** two staging files are created in the same directory
- **THEN** their names differ

### Requirement: Permission failure uses a distinct exit code
The system SHALL report a permission-denied failure while staging or replacing as a permission error with exit code 3, with a hint (rerun with sufficient privileges such as sudo; on Windows, stop the running executable), and other failures exit 1.
Evidence: src/update.rs:299-319, src/update.rs:661-671, tests/update.rs:400-429

#### Scenario: Read-only install directory
- **WHEN** the install directory is not writable
- **THEN** exit code is 3 and the installed file is unchanged

### Requirement: Running tunnels are not stopped
The system SHALL NOT stop a running process of the old executable during update; on Unix the running process continues and the file at the path becomes the new version.
Evidence: src/update.rs:1-9, src/update.rs:253-254, tests/update.rs:209-243

#### Scenario: Live tunnel
- **WHEN** a tunnel runs from the executable being updated
- **THEN** the tunnel is still running after a successful update and stdout tells the operator to restart

### Requirement: Locked executable on Windows fails cleanly
The system SHALL, on Windows, report the OS error and keep the old executable when the target is locked by a running process, including the updater itself. The message SHALL add guidance that a system service using the executable must first be stopped from an elevated terminal, naming both concrete commands, `iroh-tunnel serve service stop --system` and `iroh-tunnel access service stop --system`, so that either can be pasted into PowerShell or cmd as is. No rollback is promised, and the updater SHALL NOT stop any process itself.

#### Scenario: Running image
- **WHEN** another process runs the target exe on Windows
- **THEN** the update fails and the exe remains usable

#### Scenario: Guidance for a system service
- **WHEN** the update fails because the executable is locked on Windows
- **THEN** stderr contains `iroh-tunnel serve service stop --system` and `iroh-tunnel access service stop --system` and no `<role>` placeholder, and no service or process was stopped by the updater

### Requirement: Ownership classification of the installed executable
The system SHALL classify the installed executable as Container when `IROH_TUNNEL_CONTAINER=1` (taking precedence over everything), Homebrew when its path contains `Caskroom`, `Cellar` or `homebrew`, Linux package when it sits directly in `/usr/bin`, and Standalone otherwise.
Evidence: src/update_owner.rs:11-84, src/update_owner.rs:120-170, tests/update_ownership.rs:165-188

#### Scenario: Container variable wins
- **WHEN** `IROH_TUNNEL_CONTAINER=1` and the path is `/usr/bin/iroh-tunnel`
- **THEN** the owner is Container

#### Scenario: Other container value
- **WHEN** `IROH_TUNNEL_CONTAINER=0` and the path is `/usr/local/bin/iroh-tunnel`
- **THEN** the owner is Standalone

#### Scenario: Linux package
- **WHEN** the executable is `/usr/bin/iroh-tunnel-serve`
- **THEN** the owner is the package `iroh-tunnel-serve`

### Requirement: Managed installs are updated through their owner
The system SHALL direct package/Homebrew installs to their owner and exit unchanged unless `--yes` is given; `--yes` SHALL warn before replacing only the invoked file, and `--insecure` alone SHALL NOT override ownership.
Evidence: src/update.rs:258-285, src/update_owner.rs:87-113, src/update_owner.rs:173-205, tests/update_ownership.rs:112-163

#### Scenario: Default
- **WHEN** a package-owned file is updated without `--yes`
- **THEN** guidance is printed and the file is unchanged

#### Scenario: Insecure only
- **WHEN** `--insecure` is given without `--yes` for a package-owned file
- **THEN** the file is unchanged

#### Scenario: Confirmed
- **WHEN** `--yes` is given for a package or Homebrew owned file
- **THEN** a warning is printed on stderr and only that file is replaced

### Requirement: Container images are never updated in place
The system SHALL, for a container-owned executable, print guidance to pull the new image and redeploy, exit successfully, and never replace the file, even with `--yes`.
Evidence: src/update.rs:258-285, src/update_owner.rs:104-113, tests/update_ownership.rs:146-152, tests/update_ownership.rs:165-188

#### Scenario: Real binary in a container
- **WHEN** `update -i --yes` runs with `IROH_TUNNEL_CONTAINER=1`
- **THEN** stdout contains "redeploy", exit status is 0 and the file is unchanged
