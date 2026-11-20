## Why

`update --to VERSION` already installs any published release, including release candidates, but it downgrades silently: a typo or a stale script can roll an executable back with no guard. Version comparison also ignores pre-release suffixes, so `0.8.0-rc.1` and `0.8.0` look equal. Making downgrades explicit and ordering versions by semver lets operators try release candidates and roll back on purpose, safely.

## What Changes

**Explicit downgrade**
- From: `update --to X` installs X even when X is older than the running version.
- To: `update --to X` refuses when X is older than the running version, exits 1 with the executable unchanged and suggests `--force`; `update --to X --force` installs it.
- Reason: a downgrade must be a deliberate act.
- Impact: **Breaking** for scripts that rely on `--to` downgrading; they must add `--force`.

**Version ordering**
- From: only the numeric `MAJOR.MINOR.PATCH` core is compared; pre-release and `-dev` suffixes are ignored.
- To: Semantic Versioning precedence (`0.8.0-rc.1 < 0.8.0-rc.2 < 0.8.0`, `0.7.0-dev < 0.7.0`); build metadata is ignored.
- Reason: release candidates and dev builds must order correctly for the downgrade guard.
- Impact: non-breaking for the `latest` path (a `-dev` or candidate build already updates to the release of the same core); it decides the explicit guard, e.g. `--to 0.8.0-rc.1` while running `0.8.0` is a downgrade. Pre-release identifiers compare as semver defines, so `0.8.0-beta.1 < 0.8.0-dev < 0.8.0-rc.1`.

**New `--force` option**
- `update --force` is only valid together with `--to` (usage error, exit 2, otherwise). It does not bypass ownership (`--yes`), container protection or verification (`--insecure`), and does not reinstall the running version.

**Release candidates**
- Pinning a pre-release with `--to` is documented and covered by a scenario. `latest` keeps meaning the newest stable release.

Non-goals: discovering the newest release candidate automatically (a `--pre` or channel option); changes to installers.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `self-update`: version pinning covers pre-releases; version ordering uses semver precedence; an explicit older target needs `--force`.
- `cli-entrypoints`: `update` gains the `--force` option.

## Impact

- Code: `crates/host/src/update/` (`types.rs` request, `update.rs` version parsing/ordering and the downgrade guard), `src/cli.rs` (`UpdateArgs`), `src/app.rs` (request mapping), `src/render.rs` (the "newer than latest" hint).
- Dependency: `semver` 1.x becomes a runtime dependency of `iroh-tunnel-host` (already in `Cargo.lock` as a build dependency of the root crate).
- Tests: `tests/update.rs`, `tests/update_ownership.rs`, `tests/update_zip_windows.rs` construct `UpdateRequest` and need the new field.
- Docs: `docs/cli.md`, `docs/install.md`, `CHANGELOG.md` (Breaking entry).
- Security: the guard runs before any network request; verification and ownership checks are unchanged.
