## Design Summary

The Serve GitHub Action currently completes its main step as soon as the
status file reports the node's `node_id`. It does not wait for relay
registration, because before issue #177 was fixed the status file never
reported `home_relay`. As a result, a client that dials the `node-id` output
right after the step completes may need to retry for a few seconds.

Issue #177 is fixed (#187): the serve status file now reports `home_relay`
within about one status flush interval (about 5 s) after the relay is
assigned. This change makes the Serve action wait until the status file
reports both `node_id` and a non-null `home_relay` before the step completes.
The 60-second budget stays the same. If no home relay is reported in time, the
step fails like any other readiness timeout, and its message states that no
home relay was registered.

Intent: when the Serve step completes, the node can be dialed through its relay
straight away. Embedded clients and Access peers in later steps then no longer
need a retry window just to cover relay registration.

Goals:
- Serve is ready only when the status file reports both `node_id` and `home_relay`.
- If no relay registers within 60 s, the step fails. It reports the last status
  and the end of the tunnel log, and says that the home relay was missing.
- Docs no longer tell callers to retry early dials because of relay
  registration.

Non-goals:
- No new action inputs or outputs. In particular there is no `home-relay`
  output and no input to opt out of waiting.
- No change to Access readiness or to the tunnel binary.
- No support for setups without a relay. The action has no way to disable
  relays: an empty `relay-url` means the n0 public relays.

Anti-goals:
- The step must not report success while the Serve node cannot be reached
  through any relay.

Living spec impact (`openspec/specs/github-actions-integration/spec.md`):
- **Replaces** the requirement "Serve readiness does not wait for a relay" and
  its scenario "home_relay null". The new requirement states that Serve
  readiness waits for the home relay.
- **Modifies** "Step returns only when the tunnel is usable": Serve is usable
  when its status file reports its NodeId and its home relay. The "Serve ready"
  scenario no longer relies on a client retrying.
- **Touches** "Start failures are reported with the tunnel log": a Serve
  timeout with no home relay is one of the readiness timeouts, and its message
  names the missing relay.

## Alternatives Considered

### Alternative A: Fail the step when no relay registers in 60 s
- **Approach**: readiness requires `node_id` and a non-null `home_relay`. At the
  deadline the step fails with the usual timeout report, which says that no
  home relay was registered.
- **Pros**: misconfigured relays (wrong URL, bad token, blocked egress) show up
  at the step that caused them; the step's success means the node can be
  dialed; no new API surface.
- **Cons**: a job that only needs direct connectivity and has a broken relay
  used to pass and now fails.

### Alternative B: Warn and continue on `node_id` only
- **Approach**: wait for `home_relay` up to 60 s, then log a warning and complete
  the step anyway.
- **Pros**: never breaks an existing workflow.
- **Cons**: adds up to 60 s to a failing setup and still leaves a tunnel that
  may not be usable; the real failure surfaces later in an unrelated step.
- **Why not chosen**: it hides the misconfiguration this change should expose.

### Alternative C: Fail by default, with an input to opt into warn
- **Approach**: A, plus a new input that switches to B.
- **Pros**: covers both preferences.
- **Cons**: grows the public action API for a case with no known user.
- **Why not chosen**: YAGNI; it can be added later if someone needs it.

## Agreed Approach

Alternative A, approved by the user. A relay that never registers almost
always means the client cannot dial either, so failing early with a clear
message beats a success that only looks like one. The action exposes no way to
disable relays, so no Serve step legitimately runs without a home relay.

## Key Decisions

- Serve readiness = status file has `node_id` and a non-null `home_relay`, within
  the existing 60 s budget (user-approved).
- Timeout without a home relay fails the step and the message says so
  (user-approved).
- No new inputs or outputs (agent's call, from YAGNI and the non-goals).
- Relay-disabled setups are out of scope because the action cannot express them
  (agent's call, from the input surface).
- Docs (`docs/github-actions.md`, Behavior and Embedded clients) drop the
  "retry for a few seconds because of relay registration" guidance (agent's
  call, from the issue).
- CHANGELOG notes the behaviour change under Unreleased (agent's call). The
  actions are not released yet, so this is not marked Breaking.

## Open Questions

None.
