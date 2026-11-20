## Design Summary

Operators can install any published release, including release candidates such as `0.8.0-rc.1`, with `update --to VERSION`. Pinning a version older than the running one becomes an explicit act: it requires the new `--force` flag. Without `--force`, the command refuses the downgrade, exits with code 1, leaves the executable unchanged and tells the operator to add `--force`.

Version ordering follows Semantic Versioning precedence, so a release candidate sorts before its final release (`0.8.0-rc.1 < 0.8.0`), candidates order among themselves (`rc.1 < rc.2`), and a local `-dev` build sorts before the release of the same core (`0.7.0-dev < 0.7.0`).

Living requirements involved (`openspec/specs/self-update/spec.md` unless noted):

- "Latest release is the default and a version can be pinned": unchanged in intent. Its existing scenario "Explicit version" (`--to 9.9.8` while 9.9.9 is latest) stays valid only when the running version is not newer than 9.9.8; release-candidate pinning is made explicit with a scenario.
- "Already-current and newer versions are not changed": **altered**. Today it ignores pre-release suffixes when comparing and lets an explicit `--to` downgrade silently. After this change, comparison uses full semver precedence and an explicit `--to` older than the running version needs `--force`.
- "Version argument is validated before any request": the downgrade refusal also happens before any network request.
- `cli-entrypoints` "Update acts only on the invoked executable": **altered** to list the `--force` option.

Out of scope: automatic discovery of the newest release candidate (a `--pre`/channel option). `latest` keeps meaning the newest stable release.

## Alternatives Considered

### Alternative A: Pin-only with forced downgrade (chosen)
- **Approach**: rely on `--to` for release candidates and arbitrary versions; add `--force` to permit downgrades; adopt semver precedence.
- **Pros**: smallest surface; release location stays a plain base URL (`IROH_TUNNEL_BASE_URL` keeps working with the local test server); no unsigned API data influences selection.
- **Cons**: the operator must know the candidate's version string.
- **Why chosen**: covers the request with minimal new behaviour.

### Alternative B: Pre-release channel (`--pre`)
- **Approach**: query the GitHub releases API to find the newest pre-release and install it.
- **Pros**: no need to know the exact candidate version.
- **Cons**: depends on the GitHub API (rate limits, JSON parsing, a second release-location contract), breaks the single base-URL model used by tests and mirrors.
- **Why not chosen**: cost exceeds the need now; can be added later on top of A.

### Alternative C: Keep silent downgrade, only warn
- **Approach**: allow `--to` to downgrade as today but print a warning.
- **Pros**: no new flag, no breaking change.
- **Cons**: an accidental downgrade still happens; scripts cannot detect it.
- **Why not chosen**: the request explicitly asks for a guard flag.

## Agreed Approach

Alternative A. `--to` installs any published version (stable or pre-release). A target older than the running version is refused unless `--force` is given. `--force` only makes sense with `--to` and is rejected by the CLI without it. `--force` is independent from `--yes` (ownership override) and `--insecure` (verification bypass): it never bypasses ownership, container protection or verification.

## Key Decisions

- Refused downgrade is a failure: exit code 1, executable unchanged, message names the running and requested versions and suggests `--force`.
- The refusal is decided before any network request.
- Ordering uses full semver precedence (pre-release and `-dev` suffixes count); build metadata is ignored for ordering.
- Equal versions still report "already at <version>" with exit 0, with or without `--force` (`--force` does not reinstall).
- Without `--to`, behaviour stays: latest stable older than running reports "nothing to do" and suggests `--to`; the suggestion now also mentions `--force`.
- `--force` without `--to` is a usage error (exit 2).
- Breaking change for scripts that relied on `--to` downgrading silently; to be called out in the CHANGELOG.

## Open Questions

None.
