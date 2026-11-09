## Why

The Serve action completes as soon as the status file has a `node_id`, so a client that dials immediately may hit a node with no relay yet and must retry. That rule existed only because the status file never reported `home_relay` (#177). Since #187 it does, so the step can wait until the node is reachable through its relay.

## What Changes

**Serve readiness**
- From: Serve is ready when its status file reports `node_id`; it does not wait for relay registration.
- To: Serve is ready when its status file reports `node_id` and a non-null `home_relay`, within the existing 60 s limit.
- Reason: a completed Serve step should mean the node can be dialed through its relay right away.
- Impact: non-breaking for working setups (the step may take a few seconds longer). A Serve step whose relay never registers (wrong `relay-url`, bad `relay-token`, blocked egress) now fails after 60 s instead of passing. The actions are unreleased, so no released behaviour changes.

**Serve readiness timeout message**
- From: `was not ready within 60 s` with the last status and log tail.
- To: the same, plus a statement that no home relay was registered when `node_id` was present but `home_relay` was not.
- Reason: point the caller at the relay settings.
- Impact: non-breaking.

**Docs**
- From: `docs/github-actions.md` says Serve does not wait for relay registration and tells embedded clients to retry dials for a few seconds.
- To: it says Serve waits for its home relay, and drops the retry guidance tied to relay registration.

Non-goals: no new inputs or outputs (no `home-relay` output, no opt-out input); no change to Access readiness or to the tunnel binaries; no relay-disabled mode (the action cannot express one).

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `github-actions-integration`: "Step returns only when the tunnel is usable" (Serve usable condition), "Serve readiness does not wait for a relay" (replaced by a requirement that Serve waits for its home relay), and "Start failures are reported with the tunnel log" (Serve timeout without a home relay).

## Impact

- Code: `action-src/src/adapters/status.ts` (expose `home_relay`), `action-src/src/usecases/readiness.ts`, `action-src/src/usecases/failure.ts` / `start.ts` (timeout message); regenerated `serve/dist/main.js` and `access/dist/main.js`.
- Tests: `action-src/test/usecases/readiness.test.ts`, `start.test.ts`, `adapters/status.test.ts`, `fakes.ts` fixtures.
- Docs: `docs/github-actions.md`, `CHANGELOG.md` (Unreleased).
- Compatibility: requires a tunnel binary with the #177 fix; the action's minimum version (0.6.0) postdates it. Older binaries are already rejected by the version floor.
- Security: none; the status file is read-only input and no secrets are involved.
