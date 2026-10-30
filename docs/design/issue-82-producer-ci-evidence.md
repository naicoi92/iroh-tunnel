# Issue #82 bounded producer CI validation

Baseline: `aaf3dcae706e60323ef138f7b442be844899bf0a`.
Live #82 body/comments and native blockers refreshed before implementation;
#81 is closed. This slice enables nonpublishing producer evidence, not release
publication or full #82 acceptance. Windows aliases and supplied-key behavior
remain open. Existing CI, snapshot and release workflows remain unchanged.

## Acceptance plan for PR-trigger rework

| Criterion | Test seam |
|---|---|
| PR synthetic merge SHA; manual exact SHA guard; PR selects both hosts | Workflow regression checks trigger, matrix, SHA wiring, both checkout refs and guard; hosted PR run required. |
| Fork safety | Read-only contents, no persisted credentials, secrets, PR-target trigger or publication; workflow static test and actionlint. |
| Isolated producer and fixture | Separate clean checkouts and output; Cargo target override takes precedence over config and ambient settings. Actual offline Cargo build checks config and environment; stale archive digest rejected. |
| Real archive provenance | Every extracted binary SHA-256 must equal corresponding checkout target triple build; producer checksum and full matrix still required. Real hosted producer run required. |
| Relay safety | Production without test-utils; fixture only isolated local relay, no fixture binary upload. Existing process fixture remains runtime seam. |

## Workflow and runnable commands

Pull requests run both Linux and macOS against GitHub's synthetic merge
`github.sha`; both clean checkouts use exactly that SHA. PRs from forks use
read-only contents permission and no persisted credentials, secrets, publishing
or `pull_request_target`. Concurrency is scoped by event and PR number or ref.
Manual dispatch retains required lowercase 40-character expected SHA matching
both event SHA and checkouts; platform selects linux, macos, or all.

Follow-up: trigger a draft PR; hosted Linux and macOS are **NOT RUN** until
then. Manual dispatch requires workflow availability on default branch. No push,
PR, merge or dispatch performed in this task.

Use separate clean same-SHA checkouts, initially absent target, and separate
fresh output directories. With producer tools installed, commands are:

```sh
export RUSTUP_TOOLCHAIN=1.91
CONFIG=.goreleaser.yaml # .goreleaser.macos.yaml on macOS arm64
uv run --no-project --python 3.12 --with PyYAML==6.0.3 python tests/release_aliases.py --production --config "$CONFIG" --output /tmp/iroh82-production-exact
uv run --no-project --python 3.12 --with PyYAML==6.0.3 python tests/release_aliases.py --config "$CONFIG" --output /tmp/iroh82-fixture-exact
```

Cargo receives checkout-local absolute `CARGO_TARGET_DIR` and no ambient
`CARGO_BUILD_TARGET_DIR`; environment override defeats `.cargo/config*`
`build.target-dir`. Initially existing or symlinked target fails. Each archive
binary is compared against the corresponding built target triple; checksums,
aliases and complete matrix remain mandatory. Fixture alone uses test-utils
and local-relay forwarding. Production executes all snapshot stages and never
runs test-feature forwarding. Logs only retained seven days.

## Historical execution (earlier code checkpoints)

Commands below record what actually ran at earlier checkpoints; their bare
Python invocations are historical, not current instructions. They do not
establish success for this rework or hosted runners.

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
Final-code checkpoint below supersedes local f449c3b result.

## Real local final-code producer + forwarding evidence

Code checkpoint: `2dc24a6e1edbdd8ace57b75928a38657e5b71d40`. Both runs used
separate detached clones of exact commit, clean tracked source, initially absent
default target; no Cargo cache copied. Same tools/host as above. Subsequent
commit changes this evidence document only; source hashes remain identified
rather than attributing these captures to documentation commit.

```sh
export RUSTUP_TOOLCHAIN=1.91
export PATH=/tmp/iroh82-ci-tools/zig-macos-aarch64-0.13.0:$PATH
unset GITHUB_TOKEN GH_TOKEN HOMEBREW_TAP_GITHUB_TOKEN
# Production source has no fixture binaries, target initially absent.
cd /tmp/iroh82-ci-final-production-source
CARGO_TARGET_DIR=/tmp/iroh82-ci-stale-target python3 tests/release_aliases.py \
  --production --config .goreleaser.macos.yaml --output /tmp/iroh82-ci-final-production
# Different clean checkout, same SHA; target initially absent.
cd /tmp/iroh82-ci-final-fixture-source
python3 tests/release_aliases.py --config .goreleaser.macos.yaml \
  --output /tmp/iroh82-ci-final-fixture
```

Both passed. Production full snapshot in 4m14s: original hooks, three zigbuilds
without features, tar, checksum and generated cask; no publication. Fixture
snapshot built all three with explicit `--features=test-utils`, then selected
only macOS arm64 archive for actual TCP forwarding. Logs include producer
commands/cwd, exact SHA, feature graph and resolved digests:
`/tmp/iroh82-ci-final-{production,fixture}.log` (exit files both `0`).

| Artifact | SHA-256 |
|---|---|
| Production archive | `da0099d0c67019a6ec4634c01943bcd9246837af438476bcd47188526d3b0e4a` |
| Fixture archive (never distribute) | `b914581cb943167c70721e4e03ef041807f72aaf4fe65bb07b0c50efb2b9df80` |
| Production it / binary | `df774d40e4a45d42442ced9def5d9c81f4ad16d6bc559e5625112c634ce6dabe` |
| Production its / binary | `d2d28df98841c38aa4345d41028965efdbcb21e0f192689db5985f02258b9bb4` |
| Production ita / binary | `80e952d58e83092a60315831888fd0ad5754aa5eecc7773f709292592a719097` |
| Fixture it / binary | `bfd6735bbd80d53315b86d64ca41644f6921185b3a498c50fb4c4f54a22f51a8` |
| Fixture its / binary | `a188c6d815827dd991dd1d5f97937b1780456765f3aec9a3969143f0e4fbc792` |
| Fixture ita / binary | `c1ad8e33caf5c8bd76d4deb91e11bb0924f7ecf818ad297d44664f177a2e8ecd` |

All three default-target built binaries hash-identical to their actual extracted
archive members, per mode. Cargo Iroh release fingerprint: production
`[default, fast-apple-datapath, metrics, portmapper, tls-ring]`; fixture same plus
`test-utils`. Reverse normal/build feature graphs confirm split. Supplied stale
ambient target held literal `stale test-feature sentinel` at expected binary
path; SHA-256 `b00e85bde3d85b8ccf4b51285ef1d372d932d3d169665f7a09cc905a4d12e0b5`
unchanged and unequal every produced binary. Ambient target ignored, never
copied. This specifically checks observed GoReleaser mismatch failure mode.

Real fixture passed `produced_archive_aliases_forward_tcp_independently`: `it
serve` PID 83829 / `it access` PID 83831; `its` PID 83851 / `ita` PID 83852.
Both pairings transferred exact `matrix request` to target `127.0.0.1:58114`
and exact `matrix response` to client over isolated local relay. Only alias and
intended binary in each process PATH/directory. Test 1 passed, 0 ignored; final
line `alias forwarding PASS: 1 host-compatible archive(s)`. Never public relay.
Production deliberately reports forwarding NOT RUN.

Actual workflow guard script exercised locally against clean sibling disposable
clones: wrong SHA rejected, mismatched runner label rejected, preexisting target
sentinel rejected; matching SHA/native host shape passed. Not hosted-runner
execution. Exact-code Python regressions rerun: 7 passed, transcript
`/tmp/iroh82-ci-exact-python.log`. actionlint/shellcheck, both GoReleaser checks,
fmt and diff checks passed. Full Rust suite/clippy not rerun: no Rust edits;
existing required CI unchanged and real selected Rust producer test did run.

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
