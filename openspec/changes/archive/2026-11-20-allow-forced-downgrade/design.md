## Context

`update` lives in the `iroh-tunnel-host` crate (`crates/host/src/update/`). The use case `update(req, ctx)` takes an `UpdateRequest` (operator intent: `to`, `verification`, `overwrite`) and an `UpdateContext` (environment: base URL, current version, target, verifier, owner). The CLI (`src/cli.rs` `UpdateArgs`, `src/app.rs` `run_update_cli`) maps flags to the request and renders the `UpdateOutcome` (`src/render.rs`). Versions are strings: `parse_version` validates `--to` with a regex, and `is_older` compares only the numeric core. `APP_VERSION` is a git tag (`0.7.0`, `0.4.3-rc.4`) or `<cargo version>-dev` (`build.rs`).

## Goals / Non-Goals

**Goals:**
- Refuse an explicit downgrade unless the operator opts in with `--force`.
- Order versions by semver precedence everywhere `update` compares them.
- Keep the guard ahead of any network I/O and keep the executable unchanged on refusal.

**Non-Goals:**
- Pre-release discovery through the GitHub API.
- Changing ownership, container or verification policies.

## Decisions

**D1. Downgrade intent is a request policy enum.** Add `UpdateRequest::downgrade: DowngradePolicy { Refuse, Allow }`, mirroring `Verification` and `OverwritePolicy`, instead of a bare `bool`. It is operator intent, so it belongs in the request, not the context. Default mapping: `--force` → `Allow`, otherwise `Refuse`.

**D2. Semver via the `semver` crate.** Replace the regex in `parse_version` with `semver::Version::parse` (after stripping a leading `v`) and replace `is_older` with `Version::cmp_precedence` (build metadata ignored). Alternative: a hand-written pre-release comparator (~30 lines) — rejected; precedence rules for numeric vs alphanumeric identifiers are easy to get wrong and `semver` 1.0.28 is already in `Cargo.lock`. Parsing becomes slightly stricter (e.g. leading zeros, empty identifiers like `1.2.3-..` are rejected); all published tags are valid semver, and the error stays "invalid version".

**D3. Guard placement.** Inside `update()`, which the CLI calls only after `check_owner` allowed proceeding (so container/package outcomes win over the guard), after `--to` is parsed and before `http_client()` is built, if `downgrade == Refuse` and the requested version has lower precedence than `current_version`, fail. No outcome variant is added: a refused explicit request is an error (exit 1 through the generic `anyhow` path; not `CliError::Config`, which maps to 2). Message: `refusing to downgrade <exe> from <current> to <requested>; pass --force to install it anyway; the installed executable was not changed`.

**D4. Unparseable running version never blocks.** If `current_version` is not valid semver, no ordering is possible: the explicit target is installed and the `latest` path is not treated as older (same as today's `is_older` on garbage). `APP_VERSION` is validated as semver by `build.rs`, so this is defensive only.

**D5. Equality stays string equality on the normalized version.** "already at" compares the normalized requested/latest string with `current_version`, unchanged. `--force` does not reinstall. Precedence-equal versions differing only in build metadata are not published, so no special case.

**D6. `--force` requires `--to` at the CLI layer.** `#[arg(long, requires = "to")]` on `UpdateArgs::force`; clap produces the usage error (exit 2). The host library does not re-check this: `Allow` without `to` is harmless because the `latest` path never downgrades.

**D7. The `latest` hint mentions `--force`.** `render_update` for `NewerThanLatest` becomes `... (use --to <latest> --force to install it anyway)`, since following the old hint would now be refused.

## Risks / Trade-offs

- [Scripts using `--to` to roll back break] → CHANGELOG **Breaking** entry and the refusal message names `--force`.
- [Stricter version parsing rejects odd input that the regex accepted] → only non-semver strings are affected; they never matched a published tag anyway.
- [New runtime dependency] → `semver` is small, no transitive deps, already locked; user approved it.

## Migration Plan

Ship in the next release with the CHANGELOG entry. Rollback is a plain revert; no persisted state is involved.

## Open Questions

None.
