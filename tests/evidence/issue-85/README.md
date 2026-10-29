# Issue #85 — bounded TCP interoperability evidence

## Current review status

**Candidate evidence, not final acceptance.** Expanded real matrices passed 6/6
against each authentic v0.4.0 and v0.4.2 macOS arm64 release, using unmerged
#83 candidate runtime `a7a03be91716494eed29b8c20b350596589231f3` and harness
source commit `598887723bc46329a5a79a8fbdbf9a01c9808ed1` (script SHA256
`686e506805b4dda7b0b96282e827cc79685d999af4a7da17f15796bfef085886`).
Production binary SHA256 `eec3efa0cabacb420e203148025e5555304fbb93282c456ac1c5397a58a5883e`;
build and normal/build feature graph in `validation/candidate83-production-build.log`.
Runs: `runs/v0.4.0-candidate83/` and `runs/v0.4.2-candidate83/`. Each
variant recorded two overlapping channels, exact bytes in both rounds, expected
connection counts and two clean exits. Temporary OS directory prefixes replaced
with `/TMP/` and ANSI escapes removed; other captured content retained. Build
checkout path replaced with `<runtime-checkout>`. Public IDs are disposable.
First observer run failed because legacy Serve connection events were not
recognized; this was instrumentation error, not evidence of incompatibility.
Original failed run remains outside repository at
`/tmp/issue85-candidate83-0.4.0-a7a03be`. #83 remains unmerged; rerun against
final integrated runtime and independent Standards/Spec review remain required.
See [REWORK.md](REWORK.md) for original criterion mapping and validation boundary.

## Historical result and boundary

**Observed PASS, macOS arm64 only:** authentic published **v0.4.0** and **v0.4.2**
Access → current Serve, and current Access → each released Serve. Current means
source `5ad24007401d33fd6977fbb2d2b52894e7133333`, production debug build reporting
`iroh-tunnel 0.4.2-dev`, **not** a final redesigned release. No runtime, CLI,
shared config, dependency, protocol or release-policy changes made here.
Independent Standards/Spec acceptance review remains pending.

Each release matrix also exercises same-release old/old, current/current, and
old Access `multiplex = false` against old/current Serve (six variants each).
Each variant uses a real fixed TCP target and client, sends **1049 request bytes**
and returns **1048 distinct response bytes**, including NUL/non-UTF8/all 256 byte
values; assertions check exact bytes at target and client. Full hex transcripts,
commands/environment, timestamps, public IDs, artifact hashes, status and
kill/wait results are in `runs/*/results.json`. All successful role children
received SIGINT and exited **0**, no forced kill. This tests one TCP exchange per
variant, **not** shared-connection concurrency, reuse, lifecycle, UDP or #86.

| Run | Actual old Iroh (source lockfile) | Outcomes | Evidence |
| --- | --- | --- | --- |
| `v0.4.0-public` | 1.0.0 | 6/6 PASS | `runs/v0.4.0-public/` |
| `v0.4.2-public` | 1.1.0 | 6/6 PASS | `runs/v0.4.2-public/` |
| `v0.4.0-final` | 1.0.0 | 6/6 PASS, final harness | `runs/v0.4.0-final/` |

Baseline lock: Iroh/iroh-relay **1.1.0**, Tokio **1.53.1**.
Original metadata incorrectly said Tokio 1.53.0; correction checked against
tracked `Cargo.lock`, not a new build/run. Original provenance and checksum
manifest retained in `historical/44f8d08/`; historical logs/results untouched. See
`provenance.json` for source/tag SHAs, source lockfile hashes, release URLs,
GitHub API archive digests, published checksum lines, extracted binary hashes,
current build command, compiler and host. Published archives matched **both**
GitHub API digests and `checksums-darwin.txt`. These releases are unsigned
(macOS release recipe explicitly defers Cosign); checksum agreement identifies
published assets, not independent signed build attestation. Old dependency
versions come from tag lockfiles, not an unverifiable claim of embedded build
metadata. v0.4.1, other platform binaries and final release artifacts were not run.

## Pre-run acceptance plan and live decision

Scope: TC-02, AC-TC-02, S16, `TCP-COMPAT-01`, #85 AC1–3. Agreed seam:
authentic released 0.4 executable and current production executable as separate
Serve/Access processes; actual loopback TCP client and fixed target. Old roles
use generated disposable configs; new roles use quick flags. Configs and keys
live only inside private temporary directories and are deleted after roles exit;
no raw key/token captured. Generated config syntax is in `tcp_compat.py`.

Before harness implementation, refreshed #85/#75 bodies, comments and native
blockers. `tracker-snapshot.json` preserves refresh: #85 OPEN, native blockers
#75 and #81 both CLOSED. Latest #75 resolution:
https://github.com/naicoi92/iroh-tunnel/issues/75#issuecomment-6489986383
supersedes earlier negotiation proposals: seek both-direction compatibility;
present any failure for user decision **before breaking release**. No negotiation
framework. No observed incompatible path here, so no breaking-release decision
was sought or invented. This evidence does not close #85 or #97.

| Acceptance | Real-process observation |
| --- | --- |
| #85 AC1 / TCP-COMPAT-01 | Released 0.4 Access → quick current Serve; quick current Access → released 0.4 Serve. Exact artifact versions, commands and bytes. |
| #85 AC2 | Distinguish exchange failure from fixture/build/startup failure; never call inconclusive compatibility PASS. Any incompatible real path requires user decision, not automatic release rejection. |
| #85 AC3 | No production/protocol changes. Old multiplex true/false and old/old + current/current controls. |

Rust rules: RS-01 (focused harness), RS-05 (actionable failures), RS-07/08/09
(blocking subprocess wait off Tokio workers, bounded I/O, owned kill/wait),
RS-10 (no key capture), RS-13/15 (real production processes, verified TLS),
RS-14 (public seam), RS-16 (locked validation; newer compiler not MSRV proof).

## Isolated fixture attempt, then verified-TLS public fallback

First tried loopback HTTP `iroh-relay` **1.1.0**, `RelayConfig::new`, TLS/QUIC
disabled, no certificate bypass. Both authentic old and production new processes
rejected configuration **before connecting**: `must be https://`. Historical
`runs/local-http-rejected/` preserves all six startup failures and original Cargo
output. Its intermediate harness called these `FAIL`; classify them as
**BLOCKED fixture selection**, not wire incompatibility. Reworked harness records
startup errors as `BLOCKED` (cleanup errors independently force FAIL). Do not treat this local attempt as a real tunnel.

Self-signed local HTTPS would require modifying authentic binaries or trust:
`src/config.rs` requires HTTPS; production `src/endpoint.rs` has no custom CA
CLI; locked `iroh-relay/src/tls.rs` defaults to embedded Mozilla roots. No system
trust modifications, runtime patches or insecure released rebuilds used. Final
opt-in harness therefore requires explicit relay; no unusable default fixture.

Fallback: `https://use1-1.relay.n0.iroh.link/`, same host for both roles. Only
published release binaries and separately built **production** current executable
contact it; Rust test wrapper creates **no endpoint and no network connection**.
Never supply Cargo's feature-unified/test-utils executable to public relay.
Current normal/build feature graph in `validation/production-features.txt` excludes
`test-utils`; old macOS tagged release recipe uses `cargo --release`, no such
feature. No TLS warning/bypass in successful role logs. TLS used built-in root
verification; no `-k`, insecure config or injected CA. `relay-curl-head.txt` shows
verified HTTPS endpoint responded HTTP 404 at `/` (404 is not tunnel failure).

Every successful Access connection-established log explicitly observes
`via relay=https://use1-1.relay.n0.iroh.link/`; captured status reports active
relay where available. These observations establish relay active at handshake
and snapshot, **not forced relay-only for entire transfer**. Direct path and
separate-host/NAT behavior remain untested. Public relay server software version
not exposed/pinned; network availability affects reruns. Self-contained source,
commands and captured evidence do not make external infrastructure deterministic.

## Reproduce safely

Requirements: pinned runtime and harness sources, Cargo/Rust, uv with available
Python 3 (historical run used 3.14), GitHub CLI authenticated for asset/issue reads, macOS arm64 to run these
exact assets. Use fresh output directories; runner refuses overwrite. No sudo,
install, service changes, push, PR or merge needed.

```sh
# From harness checkout; pin actual harness commit, not runtime baseline.
HARNESS_DIR="$PWD"
HARNESS_REF=$(git rev-parse HEAD)
printf 'harness_ref=%s\n' "$HARNESS_REF"
# Historical runtime replay ONLY. This baseline has no #85 harness.
RUNTIME_REF=5ad24007401d33fd6977fbb2d2b52894e7133333
# For new acceptance replace RUNTIME_REF with reviewed #83-integrated commit.
# Do not call baseline replay current acceptance.
git worktree add --detach /tmp/issue85-runtime "$RUNTIME_REF"
mkdir -p /tmp/issue85-artifacts/v0.4.0 /tmp/issue85-artifacts/v0.4.2
for v in 0.4.0 0.4.2; do
  gh release download "v$v" --repo naicoi92/iroh-tunnel \
    --pattern "iroh-tunnel-$v-darwin-arm64.tar.gz" \
    --pattern checksums-darwin.txt --dir "/tmp/issue85-artifacts/v$v"
  gh release view "v$v" --repo naicoi92/iroh-tunnel \
    --json tagName,targetCommitish,name,publishedAt,assets,url,body \
    > "/tmp/issue85-artifacts/v$v/release.json"
  (cd "/tmp/issue85-artifacts/v$v" && \
    grep "iroh-tunnel-$v-darwin-arm64.tar.gz" checksums-darwin.txt | shasum -a 256 -c && \
    tar -xzf "iroh-tunnel-$v-darwin-arm64.tar.gz")
done
# Compare archive API digest in release.json to shasum, as recorded in provenance.json.
(cd /tmp/issue85-runtime &&
  git rev-parse HEAD && shasum -a 256 Cargo.lock &&
  CARGO_TARGET_DIR=/tmp/iroh-85-prod-target cargo build --locked --bin iroh-tunnel &&
  CARGO_TARGET_DIR=/tmp/iroh-85-prod-target \
    cargo tree --locked --edges normal,build,features -i iroh)
cd "$HARNESS_DIR"
# STOP if graph includes test-utils. Record rebuilt digest; hashes may vary by compiler/path.
shasum -a 256 /tmp/iroh-85-prod-target/debug/iroh-tunnel
```

Historical artifact pins below identify original capture, **not** automatically
trusted digests for rebuilt input. Independently verify and record rebuilt
production digest/features before replacing `IROH_INTEROP_CURRENT_SHA256`.
Record `RUNTIME_REF` and `HARNESS_REF` alongside new results. New script records
its own SHA256. New output paths below avoid overwriting historical evidence:

```sh
export IROH_INTEROP_CURRENT_BIN=/tmp/iroh-85-prod-target/debug/iroh-tunnel
export IROH_INTEROP_CURRENT_SHA256=7368f6cfa4f722d704d97c2f8c38d56f9fd4456eff2c86b5ee84ef7d45115a4a
export IROH_INTEROP_RELAY=https://use1-1.relay.n0.iroh.link/
export IROH_INTEROP_OLD_BIN=/tmp/issue85-artifacts/v0.4.0/iroh-tunnel
export IROH_INTEROP_OLD_SHA256=431d9f4d14effd398617b3d62adcf182a86533395d3cfac4426a31361d51c9f5
export IROH_INTEROP_OUTPUT="$PWD/tests/evidence/issue-85/runs/v0.4.0-expanded-new"
CARGO_TARGET_DIR=/tmp/iroh-85-test-target \
  cargo test --locked --test interop_0_4 -- --ignored --nocapture

export IROH_INTEROP_OLD_BIN=/tmp/issue85-artifacts/v0.4.2/iroh-tunnel
export IROH_INTEROP_OLD_SHA256=df09e20c808331f4f387dcf4233aa4eb7a2af94204042c991e0f7a5f226b836c
export IROH_INTEROP_OUTPUT="$PWD/tests/evidence/issue-85/runs/v0.4.2-expanded-new"
uv run --no-project --offline python tests/evidence/issue-85/tcp_compat.py
```

Use **new** output paths on rerun. uv invocation and Rust wrapper
execute identical script. Rust test ignored by default: downloading/running
external artifacts and public network must be explicit. `results.json` records
actual per-process argv and environment. Target/client code, payload literals,
timeouts and generated config templates are in script. Exit 0 requires all
variants PASS; any startup/config/artifact problem is not compatibility PASS.

Isolated harness validation (no external artifacts or network):

```sh
uv run --no-project --offline python -m unittest discover \
  -s tests/evidence/issue-85 -p 'test_tcp_compat.py' -v
```

Runner now requires two channel-tagged requests (1050 bytes each) and distinct
responses (1049 bytes each), each checked twice per channel. Both upstream TCP
sockets stay open across first round; second closes before first completes.
One Access and one Serve connection-established event required for shared
variants; old Access false requires two. Status peer rows are not proof of
connection sharing. Missing/changed observability fails closed. Cleanup must
SIGINT and wait for exit 0 on every still-live role; forced kill or premature
exit fails even after byte exchange. These checks passed against the unmerged #83 candidate, not a final integrated runtime.
Isolated tests cannot substitute for that real run.

## Historical validation, red evidence and review limits

Evidence-only TDD adaptation: no runtime fix to force green. Invalid-artifact
run returned 1 with `BLOCKED`/FileNotFoundError; historical first local fixture
run failed as above; then actual production both-direction matrices passed.
`runs/invalid-artifact-final/` repeats original harness negative path. Historical
pre-final captures are labelled; `v0.4.0-final` used original candidate harness
at `44f8d08`, not reworked two-channel harness.
No production behavior changed during any run.

Locked fmt/Clippy/full tests and final opt-in test logs in `validation/`.
Captured log text normalizes line endings and strips trailing whitespace/blank
EOF lines for Git hygiene; content, timestamps and outcomes otherwise unchanged.
`SHA256SUMS.json` hashes committed evidence files; provenance separates historical and reworked harness sources.
Full suite: **134 passed, 5 ignored** (four legacy public-relay tests safely
ignored, plus this explicit-artifact test). Compiler **1.99.1** on macOS 15.7.3
arm64, uid **501**, non-root; this does not establish Rust 1.91 MSRV or native
Linux/Windows runtime coverage. No external bot/delegated reviews run. Independent
Standards and Spec reviews remain required before acceptance; own inspection
checked source bounds, no secret capture and no runtime/protocol diff.

Protocol comparison: `git diff v0.4.0 5ad2400 -- src/proto.rs src/pipe.rs
src/endpoint.rs` empty. Access/Serve diffs add quick entrypoints/path rendering,
not wire negotiation; `proto.rs` retains `iroh-tunnel/{name}` ALPN. This is
supporting inspection, not substitute for recorded real-process exchanges.
No incompatibility trade-off observed. Release decision remains with user if
future dependency/final-artifact/platform results fail. Full supported-matrix,
final artifacts, direct/forced relay, separate hosts, v0.4.1, concurrency and
long-lived sessions remain verification gaps; #97 still owns release acceptance.
