## MODIFIED Requirements

### Requirement: Latest release is the default and a version can be pinned
The system SHALL install the single version listed by the `latest` release when `--to` is absent, and SHALL install the named release when `--to VERSION` (with or without a leading `v`) is given, including a pre-release version such as `0.8.0-rc.1`. The `latest` release SHALL mean the newest stable release; pre-releases are installed only when named with `--to`. A named release older than the running version is subject to the downgrade guard.
Evidence: crates/host/src/update/update.rs, tests/update.rs

#### Scenario: Explicit version
- **WHEN** `update -i --to 9.9.8` is run while 9.9.9 is latest and the running version is older than 9.9.8
- **THEN** the installed executable is the 9.9.8 build

#### Scenario: Release candidate
- **WHEN** `update -i --to 9.9.9-rc.1` is run, that release is published and the running version is older than `9.9.9-rc.1`
- **THEN** the installed executable is the `9.9.9-rc.1` build

#### Scenario: Unknown version
- **WHEN** `--to` names a version that is not published
- **THEN** the command fails and the installed executable is unchanged

### Requirement: Version argument is validated before any request
The system SHALL reject a `--to` value that is not a Semantic Versioning 2.0.0 version (`MAJOR.MINOR.PATCH` with optional pre-release and build suffixes, optionally `v`-prefixed) with an "invalid version" error before making any network request.
Evidence: crates/host/src/update/update.rs, tests/update.rs

#### Scenario: Path traversal value
- **WHEN** `update --to ../x` is run
- **THEN** the command fails with "invalid version" without contacting the release server

#### Scenario: Pre-release value accepted
- **WHEN** `update --to v0.8.0-rc.1` is parsed
- **THEN** the requested version is `0.8.0-rc.1`

### Requirement: Already-current and newer versions are not changed
The system SHALL order versions by Semantic Versioning precedence, where a pre-release (including a `-dev` build) sorts before the release with the same core, pre-release identifiers compare as semver defines (so `0.8.0-beta.1 < 0.8.0-dev < 0.8.0-rc.1`), and build metadata is ignored. It SHALL report "already at <version>" with exit code 0 and without replacing when the target is the same normalized version string as the running version, also when `--force` is given. When `--to` is absent and the latest release is older than the running version, it SHALL report that there is nothing to do, suggest `--to <latest> --force`, exit 0 and leave the file unchanged. When the running version is not a valid semantic version, it SHALL NOT treat any target as older.
Evidence: crates/host/src/update/update.rs, src/render.rs, tests/update.rs

#### Scenario: Latest older than running
- **WHEN** running 0.1.0 and the latest release is 0.0.5
- **THEN** a message says nothing to do and suggests `--to 0.0.5 --force`, and the file is unchanged

#### Scenario: Dev build updates to its release
- **WHEN** running `0.7.0-dev` and the latest release is `0.7.0`
- **THEN** the `0.7.0` build is installed

#### Scenario: Release candidate updates to its release
- **WHEN** running `0.8.0-rc.2` and the latest release is `0.8.0`
- **THEN** the `0.8.0` build is installed

#### Scenario: Same version with force
- **WHEN** `update -i --to <running version> --force` is run and that release is published
- **THEN** stdout reports "already at <version>", exit code is 0 and the file is unchanged

#### Scenario: Unordered running version
- **WHEN** the running version is not a valid semantic version and `update -i --to 0.0.1` is run with 0.0.1 published
- **THEN** the 0.0.1 build is installed without `--force`

## ADDED Requirements

### Requirement: Explicit downgrade requires force
The system SHALL refuse `update --to VERSION` when VERSION is older than the running version unless `--force` is given: before any network request, with exit code 1, the executable unchanged and a message naming both versions and `--force`. Ownership and container decisions SHALL come first. `--force` SHALL NOT bypass verification, ownership or container protection.

#### Scenario: Downgrade refused
- **WHEN** `update --to 0.0.1` is run by a standalone executable whose running version is newer than 0.0.1
- **THEN** exit code is 1, stderr contains the running version, `0.0.1` and `--force`, the release server is not contacted and the file is unchanged

#### Scenario: Release candidate below running release refused
- **WHEN** running 0.8.0 and `update --to 0.8.0-rc.1` is run
- **THEN** the downgrade is refused and the file is unchanged

#### Scenario: Older release candidate refused
- **WHEN** running `0.8.0-rc.2` and `update --to 0.8.0-rc.1` is run
- **THEN** the downgrade is refused and the file is unchanged

#### Scenario: Newer release candidate installed
- **WHEN** running `0.8.0-rc.1` and `update -i --to 0.8.0-rc.2` is run with that release published
- **THEN** the `0.8.0-rc.2` build is installed without `--force`

#### Scenario: Forced downgrade
- **WHEN** running 0.7.0 and `update -i --to 0.6.0 --force` is run and 0.6.0 is published
- **THEN** the installed executable is the 0.6.0 build

#### Scenario: Force keeps verification
- **WHEN** `update --to 0.6.0 --force` is run without `--insecure` and the signature bundle is invalid
- **THEN** the update fails on the signature and the file is unchanged

#### Scenario: Force does not override ownership
- **WHEN** `update --to 0.6.0 --force` is run for a package-owned file without `--yes`
- **THEN** guidance is printed and the file is unchanged

#### Scenario: Container decision precedes the guard
- **WHEN** `update --to 0.0.1` is run without `--force` and `IROH_TUNNEL_CONTAINER=1`
- **THEN** stdout contains "redeploy", exit code is 0 and the file is unchanged
