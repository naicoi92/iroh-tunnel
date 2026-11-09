# Serve Readiness Waits for the Home Relay Implementation Plan

> Executed via `/opsx-apply`, which chooses the executor.
> Spec reference: this change's `specs/` and `design.md`.

**Goal:** The Serve GitHub Action completes only when its status file reports `node_id` and a non-empty `home_relay`, and explains a relay-less timeout.

**Architecture:** `readStatus` (adapter) exposes `home_relay`; the pure `isReady` (use case) owns the readiness rule and reports what it still awaits; `start.ts` passes that to `timeoutMessage` (failure formatting). Docs and CHANGELOG follow. All work is in `action-src/` (TypeScript, Bun), with the bundles in `serve/dist` and `access/dist` regenerated.

**Tech Stack:** TypeScript, Bun 1.4 (`bun test`, `bun run build`), `@actions/core`.

## Global Constraints

- Readiness deadline stays 60 s, poll 500 ms, max 120 polls (`start.ts` constants unchanged).
- No new action inputs or outputs; Access behaviour unchanged.
- The timeout message keeps its prefix `iroh-tunnel-<role> was not ready within 60 s.` and its `Last status: ...` section; a reason, when present, sits between them.
- Minimum tunnel version stays `0.6.0` (`action-src/build.ts`).
- Generated `serve/dist/*.js` and `access/dist/*.js` must match `bun run build` (`bun run check-dist`).
- Repo content in English; follow `~/.agents/skills/coding-standards/SKILL.md`.

## Review Focus

1. `home_relay` is an empty string `""` → treat as not registered (not ready). Pinned in Task 1 status and readiness tests.
2. `home_relay` missing from the JSON (older or half-written file) → not ready, no crash. Pinned in Task 1 status test.
3. Access status files have no `home_relay` → Access readiness unchanged. Existing Access tests in `readiness.test.ts` stay green; Task 1 adds no `home_relay` to Access fixtures.
4. Serve times out with no status file at all (no `node_id`) → generic timeout message without the relay reason. Pinned in Task 1 `start.test.ts`.
5. The tunnel exits while waiting for the relay → exit-code message, not the relay reason. Pinned in Task 1 by extending "child dies mid-poll" with a relay-pending fixture.

---

## Contracts

| Name | new / modified | Shape | public |
|---|---|---|---|
| `StatusSnapshot` (`action-src/src/adapters/status.ts`) | modified | `{ node_id: string; home_relay?: string \| null; services: { name: string; transports?: unknown[] }[] }`. `home_relay` is a non-empty string when the file has one, `null` when the file has the key with a non-string or empty value, and omitted when the key is absent | no |
| `Readiness` (`action-src/src/usecases/readiness.ts`) | modified | `{ ready: true } \| { ready: false; needsProbe?: boolean; awaiting?: 'home_relay' } \| { failed: string }` | no |
| `timeoutMessage` (`action-src/src/usecases/failure.ts`) | modified | `timeoutMessage(role: Role, status: unknown, log: string, secrets?: readonly string[], reason?: string): string`. When `reason` is set, it is inserted after `was not ready within 60 s.` as ` <reason>` before ` Last status:` | no |
| `NO_HOME_RELAY` (`action-src/src/usecases/failure.ts`) | new | `export const NO_HOME_RELAY = 'No home relay was registered; check the relay-url and relay-token inputs and outbound HTTPS access to the relay.'` | no |

The status file format and action inputs/outputs are unchanged (the action only reads the existing `home_relay` key).

---

## Task 1: Serve readiness and timeout reason — implements 1

**Covers:** 1.1, 1.2, 1.3, 1.4, 1.5
**Difficulty:** Normal
**Blocked by:** none

**Files:**
- Modify: `action-src/src/adapters/status.ts`
- Modify: `action-src/src/usecases/readiness.ts`
- Modify: `action-src/src/usecases/failure.ts`
- Modify: `action-src/src/usecases/start.ts`
- Test: `action-src/test/adapters/status.test.ts`, `action-src/test/usecases/readiness.test.ts`, `action-src/test/usecases/start.test.ts`
- Regenerate: `serve/dist/main.js`, `serve/dist/post.js`, `access/dist/main.js`, `access/dist/post.js`

**Interfaces:**
- Produces: `StatusSnapshot`, `Readiness`, `timeoutMessage`, `NO_HOME_RELAY` as in Contracts.

Commands run in `action-src/` unless a step says repo root.

- [ ] **Step 1: Failing status adapter test** in `test/adapters/status.test.ts`:

```ts
test('serve home_relay: string kept, null/empty/non-string normalized to null, absent omitted', async () => {
  await ready;
  const put = (o: object) => writeFile(join(dir, 'serve-status.json'), JSON.stringify({ node_id: 'n1', services: [], ...o }));
  await put({ home_relay: 'https://r.example./' });
  expect((await readStatus(dir, 'serve'))?.home_relay).toBe('https://r.example./');
  for (const home_relay of [null, '', 42]) {
    await put({ home_relay });
    expect((await readStatus(dir, 'serve'))?.home_relay).toBeNull();
  }
  await put({});
  expect(await readStatus(dir, 'serve')).not.toHaveProperty('home_relay');
});
```

- [ ] **Step 2: Run** `bun test test/adapters/status.test.ts`. Expected: FAIL (`home_relay` is undefined).

- [ ] **Step 3: Implement** in `src/adapters/status.ts`: extend `StatusSnapshot` per Contracts. In `readStatus`, when `'home_relay' in j`, set `home_relay` to the value if it is a non-empty string, else `null`.

- [ ] **Step 4: Run** `bun test test/adapters/status.test.ts`. Expected: PASS.

- [ ] **Step 5: Failing readiness and start tests (RED for the rule).**
  - In `test/usecases/readiness.test.ts`, replace `serve is ready iff node_id is present; home_relay null does not block` with:

```ts
test('serve: not ready without a status file', () => {
  expect(isReady(serve, undefined, '')).toEqual({ ready: false });
});

test('serve: node_id without home relay awaits the relay', () => {
  for (const home_relay of [null, undefined, '']) {
    expect(isReady(serve, { node_id: N, services: [], home_relay }, '')).toEqual({ ready: false, awaiting: 'home_relay' });
  }
});

test('serve: node_id and home relay -> ready', () => {
  expect(isReady(serve, { node_id: N, services: [], home_relay: 'https://r.example./' }, '')).toEqual({ ready: true });
});
```

  - In `test/usecases/start.test.ts`, change `serveReady` to set `{ node_id: NODE, home_relay: 'https://relay.example./', services: [] }` and add, in the happy-path `describe`:

```ts
test('serve waits for the home relay before completing', async () => {
  const w = newWorld(serveIn);
  w.status = { node_id: NODE, home_relay: null, services: [] };
  w.onSleep = (x, n) => n === 4 && serveReady(x);
  await runMain('serve', startDeps(w));
  expect(w.failed).toEqual([]);
  expect(w.polls).toBe(5);
  expect(w.outputs).toEqual({ 'node-id': NODE });
});
```

- [ ] **Step 6: Run** `bun test test/usecases/readiness.test.ts test/usecases/start.test.ts`. Expected: FAIL on `serve: node_id without home relay awaits the relay` and on `serve waits for the home relay before completing` (it completes on poll 1). `serve: node_id and home relay -> ready` already passes; it guards against regression.

- [ ] **Step 7: Implement** the serve branch of `isReady` in `src/usecases/readiness.ts`: no `node_id` → `{ ready: false }`; `node_id` and a truthy `home_relay` → `{ ready: true }`; otherwise `{ ready: false, awaiting: 'home_relay' }`. Update the `Readiness` type per Contracts and the doc comment (drop the #177 note).

- [ ] **Step 8: Run** `bun test test/usecases/readiness.test.ts test/usecases/start.test.ts`. Expected: PASS; Access tests unchanged.

- [ ] **Step 9: Failing timeout-reason tests (RED for the message).** In `test/usecases/start.test.ts`, failure `describe`:

```ts
test('serve timeout without a home relay names the relay inputs', async () => {
  const w = newWorld(serveIn);
  w.status = { node_id: NODE, home_relay: null, services: [] };
  await runMain('serve', startDeps(w));
  expect(w.polls).toBe(120);
  const m = w.failed[0]!;
  expect(m).toContain('not ready within 60 s');
  expect(m).toContain('No home relay was registered');
  expect(m).toContain('relay-url');
  expect(m).toContain('relay-token');
  expect(m).toContain('"home_relay":null');
  expect(w.outputs).toEqual({});
  expect(w.state['main-ok']).toBeUndefined();
});

test('serve timeout with no status file has no relay reason', async () => {
  const w = newWorld(serveIn);
  await runMain('serve', startDeps(w));
  expect(w.failed[0]).toContain('not ready within 60 s');
  expect(w.failed[0]).not.toContain('No home relay');
});
```

  - In the Access test `timeout after 120 polls: last status and log tail`, add `expect(m).not.toContain('No home relay');`.
  - Extend `child dies mid-poll`: set `w.status = { node_id: NODE, home_relay: null, services: [] }` before running and add `expect(w.failed[0]).not.toContain('No home relay');`.

- [ ] **Step 10: Run** `bun test test/usecases/start.test.ts`. Expected: only `serve timeout without a home relay names the relay inputs` FAILs (missing reason). The other additions pass already; they guard the paths that must not get the reason.

- [ ] **Step 11: Implement.** In `src/usecases/failure.ts`, add `NO_HOME_RELAY` and the optional `reason` parameter to `timeoutMessage` per Contracts. In `src/usecases/start.ts`, keep the last `Readiness` result from the poll loop, and at the deadline pass `NO_HOME_RELAY` as `reason` when that result was `awaiting: 'home_relay'`. Exit and bind paths are unchanged (design D4).

- [ ] **Step 12: Run** `bun run typecheck && bun test`. Expected: all pass.

- [ ] **Step 13: Regenerate and check bundles.** Run `bun run build && bun run check-dist`. Expected: `check-dist` reports no out-of-date files.

- [ ] **Step 14: Commit** (repo root):

```bash
cd "$(git rev-parse --show-toplevel)"
git add action-src/src action-src/test serve/dist access/dist
git commit -m "feat(action): wait for the home relay before Serve is ready (#183)"
```

---

## Task 2: Docs and changelog — implements 2

**Covers:** 2.1, 2.2, 2.3, 2.4
**Difficulty:** Easy
**Blocked by:** Task 1

**Files:**
- Modify: `docs/github-actions.md` (Behavior, Embedded clients)
- Modify: `CHANGELOG.md` (Unreleased → Added, GitHub Actions line)

Commands run from the repo root unless a step says otherwise.

- [ ] **Step 1: Behavior bullet.** Replace the readiness bullet in `docs/github-actions.md` with:

```markdown
- **Readiness (60 s limit).** Serve is ready when its status file has a
  `node_id` and a `home_relay`, so a client using the same relays can dial
  it as soon as the step completes. There is no `home-relay` output.
- **Timeout.** If the 60 s limit is reached, the step fails and prints the
  last status and the end of the log. When Serve had a NodeId but no home
  relay, the message says so: check `relay-url`, `relay-token` and outbound
  HTTPS access to the relay.
```

  This replaces the old Readiness and Timeout bullets. The #177 link goes away.

- [ ] **Step 2: Embedded clients.** Replace the last bullet with `- Use the same relays as the Serve step.`

- [ ] **Step 3: CHANGELOG.** Under `## [Unreleased]` → `### Added`, extend the GitHub Actions line with: ` The Serve action completes once its node has a home relay, and fails after 60 s if no relay registers.`

- [ ] **Step 4: Verify.** Run `cd action-src && bun run check-docs`, and from the repo root `rg -n "retry" docs/github-actions.md`. Expected: `check-docs: 2 files ok`, and no match about relay registration or dial retries.

- [ ] **Step 5: Commit**

```bash
git add docs/github-actions.md CHANGELOG.md
git commit -m "docs(action): Serve readiness waits for the home relay (#183)"
```

---

## Task 3: End-to-end verification — implements 3

**Covers:** 3.1, 3.2
**Difficulty:** Normal
**Blocked by:** Task 1

**Files:** none in the repo. Scratch files go in a fresh `mktemp -d` directory (`$S`).

Commands run from the repo root. Run the built Serve action the way the runner does: `@actions/core` reads `INPUT_<NAME>` env vars (uppercase, hyphens kept) and appends to the `GITHUB_OUTPUT` / `GITHUB_STATE` files in the `name<<delimiter` format; the post step reads state from `STATE_<name>` env vars (case kept).

- [ ] **Step 1: Build the binary.** Run `cargo build --release --locked --bin iroh-tunnel-serve`. Expected: success.

- [ ] **Step 2: Helpers.** In a bash scratch shell (no `set -e`), from the repo root:

```bash
new_ws() { S=$(mktemp -d); : > "$S/out"; : > "$S/state"; }
# GITHUB_STATE uses `name<<DELIM` / value / `DELIM` blocks; values here are single-line.
state_args() {
  local f=$1 name= line; STATE_ARGS=()
  while IFS= read -r line; do
    if [[ -z $name && $line == *'<<'* ]]; then name=${line%%<<*}
    elif [[ -n $name ]]; then STATE_ARGS+=("STATE_$name=$line"); name=; read -r _
    fi
  done < "$f"
}
run_main() {  # run_main [extra env...]
  env RUNNER_TEMP="$S" GITHUB_OUTPUT="$S/out" GITHUB_STATE="$S/state" \
    IROH_TUNNEL_ACTION_BIN_DIR="$PWD/target/release" \
    INPUT_NAME=echo INPUT_TARGET=127.0.0.1:18080 INPUT_PROTOCOL=tcp \
    "$@" node serve/dist/main.js
}
run_post() { state_args "$S/state"; env RUNNER_TEMP="$S" "${STATE_ARGS[@]}" node serve/dist/post.js; }
tunnel_pid() { state_args "$S/state"; printf '%s\n' "${STATE_ARGS[@]}" | sed -n 's/^STATE_pid=//p'; }
bun action-src/scripts/echo-server.ts 18080 & ECHO=$!
trap '[[ -n $S ]] && run_post; kill $ECHO' EXIT
```

  `run_post` passes every saved state key (`workspace`, `pid`, `role`, `main-ok`) as separate quoted arguments; `env` accepts `STATE_main-ok=1` verbatim.

- [ ] **Step 3: Happy path (3.1).** `new_ws; run_main; RC=$?; echo "exit=$RC"`. Expected: `exit=0`, the log line `iroh-tunnel-serve is ready (node-id …)`, `$S/out` contains `node-id`, and `find "$S" -name serve-status.json -exec cat {} +` shows a non-null `home_relay`. Then `PID=$(tunnel_pid); run_post; kill -0 "$PID"` must fail (process gone), then `S=`.

- [ ] **Step 4: Unreachable relay (3.2).** `new_ws; run_main 'INPUT_RELAY-URL=https://127.0.0.1:9/'; RC=$?; echo "exit=$RC"`. Expected: after about 60 s, `exit=1`, an `::error::` line containing `not ready within 60 s` and `No home relay was registered`, and no `node-id` in `$S/out`. Then `PID=$(tunnel_pid); run_post; kill -0 "$PID"` must fail, then `S=`. This step needs no outbound access, so it always runs locally.

- [ ] **Step 5: Record** commands, exit codes and key log lines in `apply.md` evidence.

If the machine cannot reach the n0 relays, Step 3 (3.1) may instead be evidenced by `gh workflow run e2e.yml --ref <branch>` (only with the user's approval to push). Its Serve steps use the default relays and a round trip. That workflow does not exercise 3.2; if Step 4 cannot run locally, record 3.2 as not verified.

---

## Scenario → test

- Access ready → existing `start.test.ts` "access tcp: probes exactly once, then waits for transports" (unchanged)
- Serve ready → `start.test.ts` "serve: sets node-id and main-ok, state saved in order" (with relay fixture) + Task 3 Step 3
- Readiness timeout → `start.test.ts` "timeout after 120 polls: last status and log tail" (Access, asserts no relay reason)
- Serve times out without a home relay → `start.test.ts` "serve timeout without a home relay names the relay inputs" + Task 3 Step 4
- Tunnel exits during start → `start.test.ts` "exit code 2 fails immediately with class and log tail"
- Listen port taken → `start.test.ts` "start fails fast on bind error" (unchanged)
- Home relay not yet reported → `readiness.test.ts` "serve: node_id without home relay awaits the relay" + `start.test.ts` "serve waits for the home relay before completing"
- Home relay reported → `readiness.test.ts` "serve: node_id and home relay -> ready"
- Docs describe Serve readiness → Task 2 Step 4 (`check-docs` + `rg` review of `docs/github-actions.md`)
