## Design Summary

**Intent.** The CI/CD pipeline has grown into 13 workflows where the same work
runs in several places, auxiliary tooling is tested more heavily than the
application, and stage boundaries (test, build, release) blur. A typical PR
waits about 16 minutes, of which roughly 12 minutes are spent in two test files
that exercise the installer scripts, not the application. The owner wants a
pipeline where every job has one clear purpose, unrelated jobs run in parallel,
and nothing redundant runs.

**Guiding principle (owner).** Tests exist to test the application. Auxiliary
tooling (installer scripts, workflow files, packaging scripts) is almost always
correct and must not cost more CI time than the application itself; it gets
cheap static linting only.

**Measured baseline (PR run 41971651188).**

| Item | Time |
|---|---|
| `test (ubuntu)` job, critical path | 16 min |
| `tests/installer.rs` | 438 s |
| `tests/installer_ps.rs` | 275 s |
| All other integration tests + unit tests | about 200 s |
| Compile | 47 s |
| `test (windows)` | 8 min |
| Separate `windows-service-e2e` / `windows-update-e2e` | 8 min / 4 min, recompiled from scratch, not part of `ci-ok` |
| `cache-warm` on main | 15 min, reruns the whole suite only to write a cache |

**Target shape — three stages, each with one job.**

```
TEST    (pull request)   lint || test linux || test windows || script lint  -> ci-ok
CACHE   (push to main)   compile only, to keep PR caches warm
RELEASE (v* tag)         confirm the commit passed ci-ok
                         -> build all targets -> package -> sign -> verify signature
                         -> publish: GitHub Release || container images || Homebrew tap
```

Release is deployment: a tag push ends with the artifacts live on GitHub
Releases, ghcr.io and the Homebrew tap, with no manual promotion step.

**Goals**
- PR feedback driven only by application tests; target roughly 4-5 minutes
  wall clock for a typical PR (estimate, to be measured after the change).
- One workflow per stage; no job repeated in another stage.
- Independent jobs inside a stage run in parallel.
- A tag push publishes automatically once build, sign and signature
  verification succeed.

**Non-goals**
- Changing application behaviour, the CLI, the update mechanism or the
  signing identity that `update` pins.
- Changing the set of release targets, packages or images.
- Adding new test tooling (for example `cargo-nextest`).

**Anti-goals (would make this a failure even if it "works")**
- A release whose signatures the shipped `update` command rejects.
- Moving the slow tests to another stage instead of removing them (same cost,
  later feedback).
- Dropping application test coverage to gain speed.

## Alternatives Considered

### Alternative A: Keep every test, split and path-filter the slow ones
- **Approach**: Move installer tests into their own job that runs only when
  installer files change; keep the Windows e2e workflows, snapshot, smoke and
  promote.
- **Pros**: No coverage removed; smaller change.
- **Cons**: Keeps testing auxiliary tooling; keeps 13 workflows and the blurry
  stage boundaries; release still needs manual smoke and promotion.
- **Why not chosen**: The owner rejected testing auxiliary tooling at all; it
  does not fix redundancy or the stage boundaries.

### Alternative B: Make the installer tests cheap (stub archives) and keep them
- **Approach**: Keep `installer.rs` / `installer_ps.rs` but build their
  fixture archives once from tiny stub executables.
- **Pros**: Coverage of the installer scripts kept at low cost.
- **Cons**: Still maintains a test suite for tooling; does not address the
  other redundant workflows.
- **Why not chosen**: Contradicts the guiding principle; static linting of the
  scripts is judged sufficient.

### Alternative C: Application-only tests, three clean stages, automatic publish (chosen)
- **Approach**: Remove tests and verification scripts that target auxiliary
  tooling, lint scripts statically, fold Windows service lifecycle into the
  existing Windows test job, replace the cache-warm rerun with a compile-only
  job, and turn the tag pipeline into a straight build-sign-publish run.
- **Pros**: Shortest PR path; one purpose per workflow; fewer workflows to
  maintain; release is one unattended run.
- **Cons**: Installer and packaging regressions are found by users, not CI; no
  manual stop before publication; the main spec must be rewritten.
- **Why chosen**: See Agreed Approach.

## Agreed Approach

Alternative C. It is the only option that applies the owner's principle
(tests are for the application), removes duplicated runs instead of moving
them, and makes the release stage end in deployment. The accepted cost is that
installer, packaging and CI files are guarded only by static linting, and that
a bad tag is fixed by publishing a new tag.

## Key Decisions

All decided by the owner in the exploration thread:

1. **Remove auxiliary tests and scripts**: the installer script tests (Unix
   and PowerShell), the workflow/Dockerfile content tests, the CI-content
   checks in the CLI layout tests, the release-set completeness tests, the
   packaged-release smoke test, and the two packaging verification scripts.
2. **Keep static linting** for `install.sh` (shellcheck), `install.ps1`
   (PSScriptAnalyzer) and the workflows (actionlint). These take seconds.
3. **Windows service lifecycle** stays, as part of the existing Windows test
   job (it reuses that compile); the two separate Windows e2e workflows go.
4. **Cache warming** on main compiles only (tests are built, not run, plus the
   lint pass); it no longer reruns the suite.
5. **Tag releases** do not rerun lint/test; they check that the tagged commit
   already passed `ci-ok`.
6. **Release is deployment**: on a tag, after build, sign and signature
   verification, publish automatically to GitHub Releases, ghcr.io and the
   Homebrew tap. Draft releases, the manual promote workflow and the manual
   release-smoke workflow are removed.
7. **Signature verification** after signing uses a fast command-line check
   with the same issuer and identity that `update` pins, instead of compiling
   the Rust test suite.
8. **Rehearsal workflow** (`snapshot`) is removed; the tag pipeline is the
   only place release artifacts are built.

## Open Questions

None blocking. Left to design:
- How the tag pipeline confirms the tagged commit passed `ci-ok`.
- Whether the Windows ARM64 test leg is kept (default: dropped; x64 covers
  the Windows code paths).
- Which docs (README badge, CONTRIBUTING CI section) change with the
  workflows.
