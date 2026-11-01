# Issue #141 evidence — publish role images from promote.yml

Baseline `b364f32` (origin/main). No native blockers at pickup. Prerequisites located: `docker.yml` with `push`/`version` inputs and three Dockerfiles (`.github/workflows/docker.yml`, `Dockerfile.goreleaser*`); signed `*-linux-{amd64,arm64}.tar.gz` and `checksums.txt` verified by `promote.yml` (#100, `packaging/verify-release-set.sh`).

## Decision

A gated `images` job in `promote.yml`, `needs: publish`, so it shares the release gate (manual dispatch, evidence URL in this repository, draft-only, re-verified set). It calls `docker.yml` with `push: true`, `release_tag` and `permissions: packages: write`; no other job gains `packages`.

The images are built from the **published, signed glibc archives**, not from fresh build artifacts: `docker.yml` (when `release_tag` is set) downloads `<role>-<version>-linux-{amd64,arm64}.tar.gz` and `checksums.txt` from the release, checks sha256 against the signed checksum list, and extracts the binary. Dockerfiles are taken from the tag. The images therefore contain the exact released bytes. Tags: `vX.Y.Z`, `X.Y.Z`, `vX.Y`, `vX`, and `latest` for non-prerelease versions (unchanged `docker.yml` logic). Each matrix job writes `ghcr.io/naicoi92/<image>@sha256:…` to its step summary as the recorded digest.

Tag pushes (`release.yml`) still only rehearse (`push: false`); `ci.yml`/`snapshot.yml` rehearsals are unchanged (`release_tag` empty).

## Acceptance

| AC | Evidence | Status |
|---|---|---|
| After promotion the three role images exist on ghcr.io | `images` job, matrix of 3 | CONFIGURED, not run (pushing is public) |
| Same gate as release | `needs: publish`; runs only after the gated dispatch | CONFIGURED |
| `packages: write` for that job only | job-level permission on `images`; workflow default stays `contents: read` | PASS (static) |
| Digests recorded | step summary per image | CONFIGURED |

## Verification

- `actionlint .github/workflows/*.yml`: clean.
- Staging step logic rehearsed locally on a synthetic release (checksum OK, extraction for both arches).
- Not run end to end: needs a real promotion (public). Limits: no registry push, no `docker buildx` run of the promote path; verify on the first promotion and record digests here.
- Review (Standards and Spec, independent, base `b364f32` head `7879149`): no hard violations; both verdicts "OK with notes". Fixed: stale header in `docker.yml`, stale statements in `issue-100-evidence.md`. Archive layout checked: `build.yml` packs the binary as a top-level member, as the staging `tar` expects.
- **Open, retry gap:** `images` runs after `publish`. If it fails, "Re-run failed jobs" of the same run re-runs only `images` (the release is already public and `publish` stays succeeded). A fresh dispatch of the same tag stops at the draft check. An images-only entry point is not added (it would be a second, separately gated path); decide if wanted.
- Digests live in each job's step summary; copy them into this file after the first promotion, as the run logs expire.
- Rehearsal path (`release_tag` empty) on head `5a19d44`: `ci` run 41331560321 green; `snapshot` run [41331778315](https://github.com/naicoi92/iroh-tunnel/actions/runs/41331778315) green (9 builds, 3 `docker` image jobs not pushed, package). The promote path (`release_tag` set) is not exercised by CI.
