## Context

The Serve action's start use case (`action-src/src/usecases/start.ts`) polls every 500 ms for up to 60 s. Each poll reads the role status file through `readStatus` (`adapters/status.ts`) and calls the pure `isReady` (`usecases/readiness.ts`). `readStatus` currently projects the file onto `{ node_id, services }` and drops `home_relay`. Since #187, `serve-status.json` carries `home_relay` (a URL string or `null`), refreshed on every status flush tick (about 5 s). The action requires tunnel version 0.6.0 or later (`action-src/build.ts`), which includes that fix.

## Goals / Non-Goals

**Goals:**
- Serve readiness requires `node_id` and a non-empty `home_relay`.
- A Serve timeout that had a `node_id` but no `home_relay` says so.

**Non-Goals:**
- Access readiness, poll cadence, deadline, probe logic, inputs and outputs are unchanged.
- No change to the Rust status writer.

## Decisions

**D1. `StatusSnapshot` gains an optional `home_relay?: string | null`.** `readStatus` keeps `home_relay` only when it is a non-empty string; otherwise it sets `null` (serve) or omits it. Access files have no `home_relay`, so the field stays optional and Access code ignores it. Alternative: a separate serve-only reader. Rejected because it would duplicate the parsing and the half-written-file tolerance for one field.

**D2. The readiness rule stays in `isReady`.** The serve branch becomes `node_id && home_relay`. It returns `{ ready: false, awaiting: 'home_relay' }` when `node_id` is present but `home_relay` is not, so the caller can explain a timeout without re-deriving the rule. Alternative: `start.ts` inspects `lastStatus.home_relay` itself. Rejected because it splits the readiness rule across two modules. `Readiness`'s not-ready variant gains an optional `awaiting?: 'home_relay'` beside the existing `needsProbe?`.

**D3. The timeout message names the missing relay.** `timeoutMessage` takes an optional reason suffix. `start.ts` remembers the last not-ready result and, when it is `awaiting: 'home_relay'`, appends: `no home relay was registered; check relay-url, relay-token and outbound HTTPS access to the relay.` The existing prefix `was not ready within 60 s` and the last status/log tail are kept, so existing assertions and callers' log searches still match. `lastStatus` is printed as the snapshot `readStatus` returns, which now includes `home_relay`, so the report shows `"home_relay":null`.

**D4. Exit and bind paths are untouched.** A tunnel that exits during start still fails with the exit-code message, whatever `home_relay` is.

These are internal TypeScript contracts within `action-src`; no public contract (action inputs/outputs, status file schema) changes.

## Risks / Trade-offs

- [Serve steps get up to about 5 s slower, from the flush interval after relay assignment] → Accepted; this is the dial-retry window callers previously handled themselves.
- [A Serve step with a broken relay that previously passed now fails] → Intended (anti-goal: no false success). The CHANGELOG and docs say so, and the message points at the relay inputs.
- [The relay is lost after readiness] → Out of scope; readiness is a start-time check.
- [`home_relay` reported but the relay is unreachable from the client side] → Not detectable by Serve; unchanged from today.

## Migration Plan

Ships with the not-yet-released actions (0.6.0). No migration is needed. Rollback is reverting the commit and regenerating `dist`.

## Open Questions

None.
