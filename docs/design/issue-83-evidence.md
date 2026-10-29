# Issue #83 — bounded legacy multiplex compatibility candidate

Base: `5ad24007401d33fd6977fbb2d2b52894e7133333`. This record covers only the
legacy configuration component, not issue closure or redesign release acceptance.
Source is the implementation commit containing this record; exact candidate SHA
and artifact digests are reported separately after committing and rebuilding.

## Decisions and scope

Live #83 body/comments/native dependencies and #68 body/comments refreshed on
2026-10-29. Native blocker #81 is closed and merged at the base. CFG-03 and TCP-01
confirm silent legacy-field ignoring and always-on connection reuse.

User clarification relayed by parent during implementation: **only app-owned
config saves remove `multiplex`; `config edit` delegates directly to the external
editor and must not rewrite its output.** This supersedes the broader save wording
for that editor path. No post-editor normalization or persistence-policy change.

Parent authorized the single `src/main.rs` initializer deletion needed when
removing `AccessService::multiplex`; #82 integration remains parent's responsibility.
Also authorized directly contradictory docs/comments and existing regression-test
updates. No CLI options, packaging, dependencies, lifecycle defaults, migration,
identity policy, protocol negotiation or other worktrees changed.

## AC matrix recorded before implementation

Initial plan recorded at `/tmp/issue-83-ac-plan.md` before writing tests. User
confirmed real-process seam; supporting local tests use public config boundaries.

| Requirement / acceptance case | Observable test and outcome |
|---|---|
| CFG-03, AC-CFG-04, S21–22 | `tests/config_multiplex_compat.rs`: public load/save ignores omitted/true/false/string/integer/float/date/array/inline-table/nested-table legacy values, drops field, retains settings/key on reload |
| CFG-03, app-owned saves | Real `config add`, `remove`, `keygen` commands strip all retained service fields while preserving comments, relay settings and key (except requested key rotation); inline services covered |
| CFG-03, identity save | Public generated-key persistence removes field for both roles; rejected mutation leaves file/key unchanged |
| User editor clarification | Actual `config edit` launches disposable Unix editor fixture; file matches editor output exactly, including service `multiplex = false` |
| Observable config parse/write errors | Actual `config keygen` process reports malformed TOML and read-only-file write failure; original file and key remain unchanged for these failure modes, and diagnostics do not contain the key |
| TCP-01, AC-TCP-03, #83 AC2, CFG-MULTIPLEX-01 | `tests/configured_multiplex.rs`: true/false/omitted, real configured Serve and Access processes, two overlapping TCP clients, exact distinct bidirectional bytes and follow-up, one Serve connection plus stream 2, no legacy warning |
| #83 AC1, S07, CFG-RUN-01 (supporting) | Same three variants save through actual config command, restart both processes, retain public identities/service/target, repeat byte exchange and connection-reuse observation |

Applicable Rust standards: RS-01/02 cohesive contract and updated docs; RS-03
remove obsolete flag rather than retain inert public API; RS-05 propagate I/O
errors; RS-08 child kill/wait and scoped local relay; RS-10 no key/config dumps;
RS-13 local-only insecure test feature; RS-14 public-boundary specification tests;
RS-15 configured process coverage; RS-16 Rust 1.91.1 gates.

## RED → GREEN evidence

1. `cargo test --locked --test config_multiplex_compat legacy_multiplex_values_are_ignored_and_removed_on_save -- --nocapture`
   - RED: exit 101, `save retained legacy field:` even with omitted input.
   - GREEN after removing schema field/runtime branch/required initializers: 1 passed.
   - Initial harness mistakenly used `toml::Value::from_str` on document text;
     corrected to `toml::from_str` before recording behavioral RED.
2. `cargo test --locked --test config_multiplex_compat config_commands_drop_legacy -- --nocapture`
   - RED: exit 101, `add retained legacy field`.
   - GREEN after stripping legacy service fields in common `write_doc`: 2 passed.
   - Intermediate fixture passed unsupported `--config` to `remove`; corrected
     to isolated existing test-config-root override, no CLI change.
3. Runtime switch removed with schema because it references the removed field.
   New process test first passed on changed code. To establish regression
   sensitivity, temporarily restored only `src/access.rs`, `src/config.rs`,
   `src/main.rs` from pinned base in this same worktree, ran test, restored
   changed files using shell exit trap. No other worktree modified.
   - `cargo test --locked --test configured_multiplex configured_false -- --nocapture`
   - Baseline RED: exit 101, `expected one reused service connection`, observed
     `left: 2`, `right: 1`; two `peer connected via` lines and `stream=1` twice.
   - Initial test assumed target accept order matched client order; separate
     legacy QUIC handshakes exposed this assumption. Fixture now identifies
     exact distinct requests independently of accept order.
   - GREEN: `cargo test --locked --test configured_multiplex -- --nocapture`:
     all 3 variants pass, both initial and saved/restarted sessions.
   - This is a retrospective runtime regression check, not a claim that runtime
     process test was written before the coupled schema/runtime implementation.

Actual logs retained locally at `/tmp/issue-83-{red-config,green-config,red-commands,green-commands,red-runtime-baseline,green-runtime,final-targeted,tests-msrv,clippy-msrv,production-build}.log`.

## Validation observed

Environment: macOS 15.7.3 build 24G419, Darwin arm64; non-root user `naicoi`
(uid 501). `rustc 1.91.1 (ed61e7d7e 2025-11-07)` installed for MSRV checks.
Cargo.lock unchanged, SHA-256
`61d2ad363c1f01415d18b96c49d9d70bbc85a2899aef1711a5511fae8f119699`;
Iroh and iroh-relay 1.1.0, noq 1.2.0.

- `cargo +1.91.1 fmt --all -- --check`: passed.
- `cargo +1.91.1 clippy --all-targets --all-features --locked -- -D warnings`: passed.
- `cargo +1.91.1 test --all-features --locked`: passed, 140 tests; 4 public-relay
  tests ignored. Includes updated access path-logging expectation for legacy false.
- `cargo +1.91.1 test --all-features --locked --test config_multiplex_compat --test configured_multiplex -- --nocapture`:
  passed, 6 config tests + 3 process tests. Final targeted log records commands,
  loopback relay/target/listener addresses, public peer IDs, connection-path lines.
- `cargo +1.91.1 build --release --locked`: passed, production build without
  `test-utils`. Production verified-TLS runtime not exercised.
- `git diff --check`: passed.

Example observed process path, 2026-10-29T12:52:54Z: false variant initial
session logs `peer connected via relay=https://127.0.0.1:50969/`, one connection
with streams 1 and 2; same target `127.0.0.1:50972` reached after restart at
12:52:57Z. All three variants showed loopback relay on establishment. Subsequent
path migration is possible; this is not forced-relay-only or direct-path proof.
Requests: `first request`, `other request`; replies: `first response`,
`other response`; first channel follow-up: `still open` echoed exactly.

## Review rework: parse and write failures

Tests added after independent review, against candidate `ab1fd7b`: both passed
on existing behavior before changelog or implementation changes; no behavioral
RED is claimed. `malformed_config_reports_parse_error_and_preserves_file` checks
exit 2, parse diagnostic, unchanged file and no key in output.
`config_write_error_is_reported_and_readonly_file_preserved` checks exit 1,
write diagnostic, unchanged legacy file/key and no key in output. The latter
uses Unix read-only mode with a preflight check; privileged users who bypass
permissions explicitly skip it. It exercises failure opening an existing file,
not a partial write or a general atomicity guarantee. Windows write-failure
injection is not covered. Applicable standards RS-05, RS-10, RS-11, RS-13,
RS-14 and RS-15. External-editor contract remains unchanged.

On macOS arm64, non-root uid 501, Rust 1.91.1: each new test passed
individually against existing behavior; `cargo +1.91.1 fmt --all -- --check`,
`cargo +1.91.1 clippy --all-targets --all-features --locked -- -D warnings`,
`cargo +1.91.1 test --all-features --locked` (142 passed, 4 public-relay tests
ignored), and targeted `cargo +1.91.1 test --all-features --locked --test
config_multiplex_compat --test configured_multiplex` (8 + 3 passed) all passed.
No public relay tests were run with disabled TLS verification.

## Limits and review gate

Independent Standards and Spec reviews still required on exact candidate by
parent; no review delegation performed. No PR/push/merge. Linux/Windows runtime,
Windows editor fixture, 0.4 interoperability (#85), lifecycle/failure/scale (#86),
production verified TLS and final-artifact release acceptance (#97) not claimed.
Existing public-relay tests were not run because test feature disables certificate
verification. New tests only use disposable generated identities and local relay.

Test readiness retries only bounded address-in-use races. Collision/failure cleanup
in this new configured harness not fault-injected; children killed/reaped by RAII.
Configured E2E uses debug test-feature binary rather than distributed release
artifact. Application-managed serialization may reformat config as before;
concurrent/atomic persistence policy intentionally unchanged.
