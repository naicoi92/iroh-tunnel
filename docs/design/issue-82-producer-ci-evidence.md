# Issue #82 bounded producer CI validation

Baseline: `aaf3dcae706e60323ef138f7b442be844899bf0a`.
Live #82 body/comments and native blockers refreshed before implementation;
#81 is closed. This slice enables nonpublishing producer evidence, not release
publication or full #82 acceptance. Windows aliases and supplied-key behavior
remain open. Existing CI, snapshot and release workflows remain unchanged.

## Acceptance plan (before implementation)

Approved seam: actual production GoReleaser snapshots, then actual producer
archives built with `test-utils` solely for the existing local-relay real-process
alias forwarding fixture. Structural regression tests supplement, not replace,
producer execution.

| Criterion | Observable test / evidence |
|---|---|
| Exact source and intended runner | Manual-only workflow checks immutable event SHA against caller-supplied 40-character expected SHA, checked-out HEAD, clean tracked source, OS and architecture before executing repository code. |
| Least privilege, bounded cost, fork safety | Only `workflow_dispatch`; `contents: read`, checkout credentials not persisted, no supplied secrets, no registry login or publishing steps. Explicit platform selection, concurrency cancellation, job/producer timeout and limited producer parallelism. |
| Actual production producer | Original Linux/macOS config with only `dist` changed; full `goreleaser release --snapshot --clean`, no skipped build/package/Docker/cask/before stages, no test feature. Rust 1.91 and Zig 0.13.0 match release workflow. Any missing credential or stage failure fails run; never substitute archive-only production PASS. |
| Complete archive structure | Production and fixture both require Linux GNU amd64/arm64 plus musl amd64/arm64/riscv64, or macOS arm64. Verify producer checksums, three regular binaries and relative aliases `it`, `its`, `ita`, extracted targets and resolved binary digests. |
| CLI-ALIAS-01 / #82 CL1, bounded runtime | Fixture invokes existing `produced_archive_aliases_forward_tcp_independently` over isolated local relay and actual TCP client/target, through `it serve`/`it access` and `its`/`ita`, each without other binaries installed. At least one host-compatible archive must run. Foreign/unknown OS/architecture/libc is NOT RUN, never runtime PASS. |
| No production/fixture contamination | Two workflow-owned clean checkouts at same SHA, each default Cargo target initially absent; distinct output dirs. Ambient CARGO_TARGET_DIR removed because GoReleaser Rust builder copies default target path. Never upload/distribute fixture executables. Production runtime limited to `--version`, not insecure relay forwarding. |

## Workflow and runnable commands

`producer-validation.yml` uses manual dispatch only, not every PR (15 Linux
release builds twice plus Docker/packages are expensive). No `pull_request_target`,
arbitrary checkout input, registry login, external secret or write permission.
Dispatch requires access to selected repository; fork PR code is never run with
upstream secrets. Fork-local dispatch, if enabled, has only that fork's read-only
context. Existing PR checks/protection and main snapshot/release remain untouched.
GitHub must already know the workflow on default branch before dispatch can work;
this task does not push, create PRs, merge or dispatch it.

Authorized follow-up after workflow is available (do not run release.yml):

```sh
SHA=$(git rev-parse HEAD)
BRANCH=$(git branch --show-current)
gh workflow run producer-validation.yml --repo naicoi92/iroh-tunnel \
  --ref "$BRANCH" -f expected_sha="$SHA" -f platform=all
```

Exact SHA must be 40 lowercase hex characters and equal dispatch event SHA AND
checked-out HEAD. Runner checks require Linux x86_64 or macOS arm64 before any
repository script runs. `platform=linux` defaults to single Linux job; `macos`
limits to macOS; `all` runs both. Cancellation replaces older same-ref/platform
runs. Job timeout 150m; each producer timeout 1h and parallelism 1. Production
checkout's Cargo target deleted only after successful inspection to bound disk
use before fixture build in second clean checkout. No shared Cargo cache. Logs only retained seven days; no fixture
executables uploaded. cargo-zigbuild and GoReleaser installation follow release's
current `--locked`/`latest` policy; logs record resolved versions rather than
claiming those unpinned tools are reproducible.

Equivalent local producer commands (fresh output required), from exact clean
checkout on matching host with Rust `1.91`, Zig `0.13.0`, cargo-zigbuild,
GoReleaser, Python 3.12+/PyYAML and Linux Docker/buildx/QEMU installed:

```sh
export RUSTUP_TOOLCHAIN=1.91
# Put Zig 0.13.0 on PATH, not system Zig of another version.
CONFIG=.goreleaser.yaml # .goreleaser.macos.yaml on macOS arm64
python3 tests/release_aliases.py --production --config "$CONFIG" --output /tmp/iroh82-production-exact
python3 tests/release_aliases.py --config "$CONFIG" --output /tmp/iroh82-fixture-exact
```

Run each command from its own clean checkout of identical committed SHA with
initially absent target/. First includes every production stage, changes only
dist. Second enables test-utils and skips before/homebrew/Docker/nFPM only for
fixture. GoReleaser 2.18.1 Rust builder honors CARGO_TARGET_DIR for compilation
but copies binaries from `<build.dir>/target/<triple>/release`; harness removes
ambient CARGO_TARGET_DIR and workflow's separate checkouts isolate default
producer/fixture target paths. Both validate complete archive matrix/checksums/aliases and
host-compatible alias versions. Only fixture forwards over local relay; zero
compatible runtime forwarding fails. Production final line explicitly says
`forwarding NOT RUN`. No fallback flags on production failure.

## Execution status and validation

**Before:** baseline has no `producer-validation.yml`; existing snapshot only
runs on main. New production regression failed before implementation with
`error: unrecognized arguments: --production` (transcript
`/tmp/iroh82-producer-mode-red.log`).

**After, local static/regression checks:**

- `actionlint .github/workflows/producer-validation.yml`: passed, including
  shellcheck. Initial invalid runner-context usage corrected before commit.
- `goreleaser check --config .goreleaser.yaml` and macOS config: passed.
- `python3 -m unittest discover -s tests -p test_producer_validation.py`: 2 passed;
  synthetic GoReleaser boundary fixtures establish full production config except
  dist, no skipped stages, ambient target override removed, no forwarding and
  producer failure propagation. Synthetic stale-target sentinel cannot affect
  producer environment or extracted digest; sentinel remains unchanged. Not
  real snapshot evidence.
- `python3 -m unittest discover -s tests -p test_release_aliases.py`: 5 passed;
  complete matrices, rejection of missing/extra archives, libc/architecture
  selection and foreign-only run fails. Not real platform execution.
- `cargo +1.91 fmt --all -- --check` and `git diff --check`: passed. No Rust edit.
- Existing ci/snapshot/release workflows and production configs byte-unchanged
  versus baseline (`git diff --exit-code BASE -- <five paths>`).

## Target-path mismatch checkpoint

Implementation checkpoint `bca60ed50148f05093e3a2c5b1509778d5e0b52c` attempted
macOS production with CARGO_TARGET_DIR under output. Source inspection during
run found GoReleaser 2.18.1's Rust copy path ignores this override. That capture
(`/tmp/iroh82-ci-bca60ed-production.log`) was aborted; artifacts may have copied
preexisting default-target binaries and are **INVALID as exact-source/feature
proof**. No PASS claimed. This invalidates that attempt, not prior captures
that never used target override. Approved fix: workflow-owned separate clean
checkouts, no harness-created worktrees; clear ambient target override. Real
validation below must use clean disposable checkout of committed source.

## Real local production checkpoint

Fresh detached clone of `f449c3b57d9ead724395017d0d7a55b47d31fb96`, default
target initially absent; no tracked source changes. macOS 15.7.3 arm64 (not
GitHub macos-14), Rust 1.91.1, verified Zig 0.13.0, cargo-zigbuild 0.24.0,
GoReleaser 2.18.1, Python 3.14.3 / PyYAML 6.0.3. Credentials explicitly unset.

```sh
cd /tmp/iroh82-ci-f449c3b-production-source
RUSTUP_TOOLCHAIN=1.91 PATH=/tmp/iroh82-ci-tools/zig-macos-aarch64-0.13.0:$PATH \
  python3 tests/release_aliases.py --production --config .goreleaser.macos.yaml \
  --output /tmp/iroh82-ci-f449c3b-production
```

Passed full before hooks, all three production zigbuilds, tar/checksums and
generated Homebrew cask in 2m52s; no supplied token needed. Archive SHA-256
`2415c1ff1e8cbbf7e48496540e0d9f4b8ce0d4ab25b60b4e8d1fd9222f0e35ef`.
Resolved alias binary SHA-256: it
`8e2767913188c4bdc0e3d01fc58fbbf149093d0b69dc476d36ec3c9c37a48a20`,
its `2567af193450dd0e1d736988f4b84ee6fb2b896d025f3828f989973af0226164`,
ita `04f6ce5dd9bde3e6ff01c180046b8a1358bace3268a12f8abd76950d54842bb4`.
All aliases return intended binary name / `0.4.2-dev`. Tar members and extracted
relative links verified; config diff only dist; reverse Iroh feature graph
(`cargo +1.91 tree --locked -e normal,build,features -i iroh`) has no test-utils.
Transcript `/tmp/iroh82-ci-f449c3b-production.log`; graph
`/tmp/iroh82-ci-f449c3b-production-features.log`. This is real local production
snapshot evidence at stated checkpoint, not Linux or hosted-runner acceptance.
Later workflow-only source-path/logging changes require final-head checks.

Linux full production snapshot, complete GNU/musl/riscv64 structural matrix,
Docker/packages and native GNU forwarding remain **BLOCKED / NOT RUN** until
actual Linux runner executes exact committed source. musl/foreign architectures
are structural-only on glibc runner, never runtime PASS. macOS GitHub `macos-14`
execution also pending. Local host is macOS arm64; no Linux runtime coverage
inferred from cross-compilation. Historical evidence in
`issue-82-unix-alias-evidence.md` used different producer tool versions and does
not satisfy pinned producer gate. Independent Standards and Spec reviews remain
required outside this sole-writer task; no delegated review or claim of release
acceptance. Applicable constraints: RS-13 (test TLS boundary), RS-14/15 (real
producer/process seam), RS-16 (release toolchain); no Rust code changed.
