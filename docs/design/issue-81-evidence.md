# Issue #81 rework evidence

## Contract and test plan (before fixes)

Baseline `bc943290d8393658851995fa99592c54eb071c2d`; merge integration
`7ad9bf52a4ae638472fb4cbdbd0da35f5e816d96` without reset completed as
`a0bf31b`. Required tracked design docs preserved. Live #81 body, comments and
native blockers refreshed 2026-10-29: open issue, no native blockers.
Both supplied independent reviews read; findings checked against source and live AC.

- CLI-QUICK-01 / #81 AC1, CL1: extend `tests/quick_run.rs` real-process
  fixed-target byte exchanges for long/short flags and numeric IPv4/IPv6.
  IPv6 target and listener tested separately so one failure cannot mask another.
- CLI-ERROR-01 / #81 AC2, CL1–2: process tests reject every explicit quick
  option mixed with role subcommands (including explicit default TCP), invalid
  address/port/peer/option, missing inputs and unsupported protocol. Assert
  role-specific corrective diagnostics, no working listener and unchanged
  existing config/key bytes. Existing `src/error.rs` and README classify config
  errors as exit 2; preserve that contract, not invent numeric redesign policy.
- IDENTITY-QUICK-01 / #81 AC2, CL2: both roles restart with distinct public
  identities; test empty homes and existing synthetic config/key preservation.
- SEC-NODEID-01 / #81 CL3: inspect operator docs/help beside successful quick
  session. Explicitly document public NodeId, target-owned access protection;
  no new authentication scheme or fabricated ACL test.
- CONTRIBUTING requires user-visible changelog entry; add under Unreleased.

TDD sequence: add process assertions, run against unchanged implementation and
record observed failures, then minimal fixes and rerun. Only isolated local relay
fixtures use existing insecure `test-utils` TLS override; no public relays or real
secrets. Production build omits test-utils. Byte exchange does not prove relay-only
transport; record observed status paths without assuming path selection.

## Finding verification and TDD observations

Pre-fix process run: `cargo test --test quick_run -- --nocapture`
(`/tmp/issue-81-red.log`, 5 passed, 2 failed). Mixed mode reproduced:
`serve --name echo config path` returned **0**, silently discarding `--name`.
Initial role-diagnostic assertion also failed on clap socket errors; assertion
was refined to accept role-specific `--target`/`--listen` context rather than
require redundant role names in every parser message.

Separate pre-fix checks:

- `cargo test --test quick_run quick_config_errors_use_existing_exit_classification -- --nocapture`
  failed: `serve -p udp` returned **1**, expected existing config class **2**.
- `cargo test --test quick_run invalid_quick_inputs -- --nocapture` failed:
  `serve --name echo --target 127.0.0.1:0` returned **1**, expected **2**;
  malformed address and out-of-range port already returned 2 via clap.
- `cargo test --test quick_run quick_operator_contract -- --nocapture` failed
  before README update: missing public-identity/target-protection explanation.
- Both IPv6 regressions **passed before fixes**, independently transferring
  `request` / `response` with IPv6 target and IPv6 listener. Review's claimed
  universal runtime IPv6 failure was **not reproduced** on this host: Tokio's
  string resolution falls back to OS address resolution, which accepts this
  unbracketed form here. No speculative IPv6 runtime change made; new regression
  coverage remains. Linux/Windows behavior still needs native runs.
- Changelog omission confirmed against CONTRIBUTING; fixed under Unreleased.

Fix uses clap's `args_conflicts_with_subcommands` on each role, which detects
explicit default `--protocol tcp` while allowing implicit default in configured
commands. Quick-only validation maps to existing `CliError::Config`; endpoint
and runtime errors are not blanket-classified as configuration errors. No
configured-run, wire protocol, dependency or TLS policy changes.

## Final worker execution (not independent acceptance)

Source: `126b03093d12a366f7a430f12b1abdf659a4c699`. Run finished
2026-10-29 09:07:54 UTC. macOS Darwin 24.6.0 arm64; Rust 1.99.1;
non-root uid 501, no elevation. Iroh/iroh-relay **1.1.0**, Tokio **1.53.0**,
local fixture `iroh::test_utils::run_relay_server()` from locked dependency.
`Cargo.lock` SHA256:
`61d2ad363c1f01415d18b96c49d9d70bbc85a2899aef1711a5511fae8f119699`.

Commands and observed results:

- `cargo fmt --check`: passed. Earlier intermediate check detected unformatted
  test additions; `cargo fmt` corrected them before final validation.
- `cargo test`: **131 passed, 4 ignored**, no failures (114 unit + 17 integration).
  Log `/tmp/issue-81-final-test.log`, SHA256
  `0f65cb14a2e3955f459bcb8fab0081f1ff67dd6aa8778477f0c5cb718bcc7445`.
- `cargo test --test quick_run -- --nocapture`: **11 passed**, no ignored tests,
  no IPv6 skip. Log `/tmp/issue-81-final-e2e.log`, SHA256
  `86f9c35e3049c9bcd04c65b314101ac6c9edea0cefcdbd55c7615de0606ac8a6`.
  Tests launch actual `target/debug/iroh-tunnel` subprocesses, TCP target/client,
  isolated HOME/state paths. Full invocation arguments and public status
  snapshots appear in log; no secret/key/token values logged.
- `cargo build --release --no-default-features`: passed. Production normal/build
  dependency graph checked using
  `cargo tree -e features --no-default-features --edges normal,build`; no
  `test-utils` feature. Release `access --help` and `serve -p udp` smoke checks
  passed (latter exit 2). Production TLS bypass not introduced or enabled.
- `git diff --check`: passed.

Artifact SHA256:

| Artifact | Digest |
|---|---|
| real-process E2E binary, test-utils enabled, never distribute | `b48193f909185e16b3216756e2941a02703d41b505dfce020e71014db5bb8129` |
| production release, no test-utils | `7ff2c5e1cccb3651a54e7875a98c06262171d015c87cad0411bb3aa9908a7656` |

### CLI-QUICK-01 observed variants

Each actual client sent `request` (7 bytes), fixed target asserted exact request,
returned `response` (8 bytes), client asserted exact response. Dynamic fixtures:

| Flags | Fixed target | Access listener | Result |
|---|---|---|---|
| long, protocol omitted | `127.0.0.1:57712` | `127.0.0.1:57713` | exact bytes both ways |
| short, explicit `-p tcp` | `127.0.0.1:57698` | `127.0.0.1:57700` | exact bytes both ways |
| long, protocol omitted | `[::1]:57699` | `127.0.0.1:57701` | exact bytes both ways |
| short, explicit `-p tcp` | `127.0.0.1:57706` | `[::1]:57707` | exact bytes both ways |

Serve arguments: `serve --name echo --target ADDRESS --relay-url LOCAL_RELAY`
(or `-n`, `-t`, `-p tcp`). Access:
`access --name echo --listen ADDRESS --node-id PUBLIC_SERVE_ID --relay-url LOCAL_RELAY`
(or `-n`, `-l`, `-N`, `-p tcp`). Each roundtrip uses its own local relay and
target. Status after 6 seconds showed both local relay and direct transports
marked active. This proves **neither relay-only nor direct-only byte routing**;
no path-forcing/performance claim. Only loopback relay URLs supplied, no public
relays or real credentials.

### CLI-ERROR-01 / IDENTITY-QUICK-01 observations

Both roles reject malformed/out-of-range/zero-port addresses, bad names,
unsupported protocol, bad relay scheme, unknown option, missing arguments;
Access additionally rejects invalid/missing peer ID. Expected corrective
argument or constraint present in stderr, config class exit 2. All explicit
long/short quick options, including explicit default TCP and relay URL, reject
both `config path` and `run --config EXISTING_FIXTURE` with `cannot be used with`.
Implicit defaults still allow configured `config path`. Invalid/mixed process
exit bounded at 5 seconds; no status file or lingering listener observed.

Synthetic valid role configs contain a known test-only key and loopback relay;
key file also present. No user secrets accessed. Tests compare every existing
file's exact bytes and complete relative-path/byte snapshots before/after
failure and quick restarts, preventing unnoticed new key files as well as
replacement. Empty homes remain empty across independent restarts of both roles.
Representative restart public IDs (both empty/existing-home variants passed):

- Serve `065677a5f919fdb7a2c11366b6993ea5746e5628a7a83eba0725d5fd7167187b`
  → `8b7c0369133b26ec8cdb7bebefe9e376ef07f804b5568a7cb942842ebda7c2f0`.
- Access `b6ee99f3158063ac9d22e41f2cc35350af270a87f4ce23b8cfdfe671e11a610e`
  → `100940887fc849f975bc3a5c0cd6da85f14175b3cc9e9c098fa799c02266074a`.

### SEC-NODEID-01 documentation assertion

README quick-run section: “NodeId is public identity, not target authorization”;
“Target services must enforce their own access protection”. Real `access --help`
labels NodeId “public identity, not authorization; protect the target service”.
Process documentation assertion passed alongside successful byte sessions. No
ACL/authentication behavior invented or claimed.

## Parent integration verification

Integration fast-forwarded to the reviewed candidate `c5b3ecbba54a6f4aa30002b360ca323d0b34f6d4`. Independent Standards and Spec reviews found no remaining issues; neither claimed cross-platform or production-TLS acceptance. Parent reran on this exact source in the integration worktree on macOS arm64:

- `cargo fmt --all -- --check` and `cargo clippy --locked --all-targets --all-features -- -D warnings`: passed.
- `cargo test --locked --all-features`: 131 passed, four pre-existing Internet-dependent tests ignored.
- `cargo test --locked --test quick_run -- --nocapture`: 11 passed, including separate IPv6 target and listener cases.
- `cargo build --locked --release --no-default-features`: passed; normal/build feature graph excludes `test-utils`.
- Lockfile SHA-256 `61d2ad363c1f01415d18b96c49d9d70bbc85a2899aef1711a5511fae8f119699`; combined verification log SHA-256 `acdf117ce9ec2946d0449179a0d83473a4576e1c4be864a966960964c75babe6`; feature graph log SHA-256 `cd2d3e96d6764a4e9fee41c44be45b7d4f4a0bc396f4d0aa7e0df3106a4cd6bf`.

A prior parent test invocation accidentally ran in the original checkout; its results are excluded from #81 evidence. Slice acceptance applies only to the tested macOS environment and test-feature process path. Final-artifact, production TLS and Linux/Windows evidence remain open under #97; this does not mark the full catalog as passed.

## Limits and reviewer disposition

- Independent parent review still required; shared acceptance catalog remains
  NOT_RUN until independently accepted final-artifact evidence. This is worker
  observation, not release acceptance or issue closure.
- Linux, Windows x64/ARM64 unavailable in this worktree; only macOS arm64 runtime
  tested. IPv6 happened to work unchanged here; portability not inferred.
- Four existing Internet/n0-relay tests intentionally ignored by default. Not
  forced: test-utils disables TLS verification globally; user requires local
  fixtures only. Existing local relay auth/status tests passed.
- Production release built and smoke-tested, but full production TLS roundtrip
  not run against local self-signed fixture. Actual forwarding evidence uses
  real test-feature-enabled executable, not production-distribution artifact.
- No 0.4 interoperability, standalone binaries, UDP, ownership, service/install,
  update, forced relay/direct or release-matrix claims; these remain other issues.
- Existing untracked `.codegraph/` left untouched, never staged. No push or issue
  edits/closure performed. Documentation-only evidence commit follows code SHA.

## Follow-up isolation/readiness contract

- All quick-run subprocesses use an absolute test-feature config root and isolated
  XDG/HOME. `config path` verifies both role defaults point into that root;
  preservation snapshots seed those exact files. Production without test-utils
  ignores the test override and retains OS config resolution.
- Access process readiness requires its post-bind listener log, not status or
  pre-bind endpoint log. Retry only explicit address-in-use bind failures, with
  bounded attempts and child cleanup. Other bind failures and timeouts fail.
  Existing same-target and independent IPv6 process roundtrips remain intact.

### Follow-up worker validation

macOS arm64, Rust 1.99.1; RS-01, RS-05, RS-08, RS-11, RS-13–16.
`cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features
--locked -- -D warnings`, and `cargo test --locked --test quick_run` passed
(13 tests, including both IPv6 cases). Initial runs exposed ANSI-colored log
fields and unbracketed IPv6 listener output; readiness matching was corrected
and rerun. `cargo build --locked --no-default-features` passed; with an absolute
`IROH_TUNNEL_TEST_CONFIG_DIR` set, production `serve config path` returned
`HOME/Library/Application Support/iroh-tunnel/serve.toml`, not the override.
No native Linux/Windows, Rust 1.91, or verified-TLS production forwarding run.
Collision retry branch not forced in this run. Independent reviews still required.

### Forced collision follow-up

Acceptance mapping (RS-08, RS-09, RS-13–16): a real isolated Access process
first binds an occupied IPv4 loopback port; its address-in-use log triggers
retry, the failed child is killed and its `wait` result observed, and the next
Access process reports its post-bind listener and accepts a TCP connection on a
different port. The reservation remains held throughout. Each attempt retains
the existing 30-second deadline and eight-attempt bound; process drop handles
cleanup even on error. This tests the actual retry and cleanup path, not a mock.

macOS arm64, Rust 1.99.1: `cargo fmt --all -- --check`,
`cargo clippy --all-targets --all-features --locked -- -D warnings`, and
`cargo test --locked --test quick_run -- --nocapture` passed (14 tests).
Representative log: occupied `127.0.0.1:53537`, OS error 48; retry
`127.0.0.1:53556`; first child PID 58500 wait status 9, successfully reaped.
Full test log `/tmp/issue81-quick-run.log` SHA-256
`d44a089c7b6da2b3f75a823f907097fd5896af4d11e789db164607c9432ea4c5`.
An initial Clippy run found unnecessary unwrap and a guard across await; both
were corrected before the passing run.

Non-collision bind failure coverage remains blocked: no portable deterministic
way was established to cause a real TCP bind to fail for a reason other than
address-in-use without assuming host interface, privileges, socket policy, or
platform-specific error behavior. No simulated error is claimed as process
coverage. Native Linux/Windows, Rust 1.91 and full-suite gates remain unverified
in this follow-up; no production TLS or release acceptance is implied.
