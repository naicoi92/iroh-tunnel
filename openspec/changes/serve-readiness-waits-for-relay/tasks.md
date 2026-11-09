## 1. Serve readiness waits for the home relay

Blocked by: none

Owns: `action-src/src/adapters/status.ts`, `action-src/src/usecases/readiness.ts`, `action-src/src/usecases/start.ts`, `action-src/src/usecases/failure.ts`, `action-src/test/adapters/status.test.ts`, `action-src/test/usecases/readiness.test.ts`, `action-src/test/usecases/start.test.ts`, `serve/dist/main.js`, `access/dist/main.js` (regenerated). Platform-neutral TypeScript; Linux, macOS and Windows runners share this code path.

- [x] 1.1 `readStatus` exposes `home_relay` (non-empty string or `null`) in `StatusSnapshot` (design D1); test in `adapters/status.test.ts`
- [x] 1.2 `isReady` serve branch requires `node_id` and `home_relay`, returning `awaiting: 'home_relay'` while only `node_id` is present (design D2); covers scenarios "Home relay not yet reported", "Home relay reported"
- [x] 1.3 Serve timeout message adds the no-home-relay reason naming `relay-url` and `relay-token` (design D3); covers "Serve times out without a home relay", keeps "Readiness timeout", "Tunnel exits during start"
- [x] 1.4 Update existing serve fixtures in `start.test.ts` to report a home relay; covers "Serve ready"
- [x] 1.5 Regenerate `dist` (`bun run build`) and pass `bun run typecheck`, `bun test`, `bun run check-dist`

## 2. Docs and changelog

Blocked by: 1

Owns: `docs/github-actions.md`, `CHANGELOG.md`.

- [x] 2.1 `docs/github-actions.md` Behavior: Serve is ready once the status file reports a NodeId and a home relay; mention the timeout reason; remove the #177 retry note; covers "Docs describe Serve readiness"
- [x] 2.2 `docs/github-actions.md` Embedded clients: drop "retry the dial for a few seconds after Serve starts", keep "use the same relays"
- [x] 2.3 `CHANGELOG.md` Unreleased: note under the GitHub Actions entry that Serve waits for its home relay and fails after 60 s if none registers (not Breaking: actions unreleased)
- [x] 2.4 `bun run check-docs` passes

## 3. End-to-end verification

Blocked by: 1

Owns: no repo files (scratch only under `$TMPDIR`).

- [ ] 3.1 Build `iroh-tunnel-serve` from this commit and run `serve/dist/main.js` locally with `IROH_TUNNEL_ACTION_BIN_DIR`, default relays: step completes, `node-id` set, `serve-status.json` has non-null `home_relay`
- [ ] 3.2 Same with `relay-url: https://127.0.0.1:9/` (unreachable): step fails after 60 s with the no-home-relay reason, no `node-id` output; run the post step to stop the tunnel
