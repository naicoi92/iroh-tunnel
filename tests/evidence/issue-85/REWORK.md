# #85 review rework — acceptance plan and boundary

Baseline candidate: `44f8d082b8fc7a9aad6cad8c6c807dbfc465fba3`.
Runtime captured historically: `5ad24007401d33fd6977fbb2d2b52894e7133333`.

## Acceptance mapping

- `docs/design/redesign-acceptance.md:1137–1138`, TC-02 / AC-TC-02:
  runner must exercise two overlapping real TCP channels in every old/new and
  control variant. Distinct channel-tagged binary request/response payloads,
  checked exactly at target/client for two rounds. Keep first channel open
  while second exchanges bytes; close second before first's second round.
- TCP-01 where applicable: while both channels live, count Access and Serve
  connection-established log events. Require exactly one event on each side
  for current Access and old Access `multiplex=true`. Authentic old Access
  `multiplex=false` requires two per-channel connections, not redesigned
  behavior. Do not use Serve status peer-row count as connection count.
- RS-08/09/15: role cleanup owns SIGINT/wait with bounded kill fallback. Any
  premature exit, nonzero exit, forced kill, signal/wait error or failed
  temporary-directory cleanup forbids PASS. Regression tests inject failure
  and verify runner returns nonzero, including after successful exchange.
- RS-07/13: Rust wrapper waits off Tokio workers and invokes `uv run
  --no-project --offline python`; no Python project or dependencies needed.
  Wrapper starts no endpoint. Never use feature-unified binary on public relay.
- Preserve original capture and manifests; correct Tokio claim against
  `Cargo.lock` (1.53.1). Separate runtime source from harness source in reruns.

## Review-stage validation only

Red unit run saved in `validation/rework-red.log`; initial four tests failed before
helpers existed. Seven green isolated checks exercise actual loopback TCP sockets,
connection-count validation, and cleanup failure injection (including real disposable child exiting 7 on SIGINT). They establish
harness safety only, not old/new Iroh interoperability. Rust wrapper invalid-artifact
check returned expected failure (Cargo 101, runner BLOCKED) through uv. Locked
all-feature suite, all-target/all-feature Clippy and fmt passed; public-relay
tests remained ignored. Logs named `validation/rework-*` separate new validation. No public relay,
production endpoint, release artifact or real key used during this rework.

Live #85, #75 comments, #83 and #85 native blockers refreshed read-only during
rework: #85 OPEN, #75/#81 CLOSED; #75 still forbids new wire negotiation and
requires user decision on failed direction before breaking release. #83 runtime
must be integrated before fresh production build and expanded real matrix.

Historical `runs/` and original `validation/` captures remain unchanged.
`historical/44f8d08/` retains original provenance and checksum manifest;
original harness/README available verbatim at baseline commit. Updated root
manifest describes current files, not historical harness execution.

## Historical rework acceptance gap

Subsequent candidate run recorded in README and provenance; following text describes
the gap at rework time, not current candidate results.

**No new runtime compatibility PASS claimed.** Fresh, separately built,
verified-TLS production executable from #83-integrated runtime must run expanded
matrix against authentic 0.4 releases. Record exact runtime and harness refs,
lockfile/features/artifact hashes, both-direction bytes, connection counts and
successful controlled cleanup under new output paths. Independent Standards
and Spec reviews still required. #86 failure/drain semantics, final release
artifacts, supported platforms and full redesign #97 remain separate gates.
