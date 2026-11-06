# iroh-tunnel GitHub Actions (Access + Serve) Implementation Plan

> Executed via `/opsx-apply`, which chooses the executor.
> Spec reference: this change's `specs/` (`github-actions-integration`, `ci-cd-pipeline`) and `design.md` (D1-D11).

**Goal:** Ship two JavaScript GitHub Actions — Access (`naicoi92/iroh-tunnel@v0`, alias `/access@v0`) and Serve (`naicoi92/iroh-tunnel/serve@v0`) — that start a job-scoped iroh-tunnel role from release binaries, return when it is usable, and stop it at job end; cover them in CI and release.

**Architecture:** One TypeScript package `action-src/` with a pure domain layer (requests, validation, config rendering, readiness rules), adapters (actions IO, release download, workspace, process, status file, log watcher, TCP probe) and two use cases (`start`, `stop`). Four entry points are bundled by `bun build` into committed `access/dist/` and `serve/dist/`; the root `action.yml` is generated from `access/action.yml`. CI adds an `action` PR job and action round trips in `e2e.yml`; release moves the major tag.

**Tech Stack:** Bun 1.4 (package manager, test runner, bundler), TypeScript (strict), Node 24 runtime, `@actions/core` 3.x, `@actions/exec` 3.x, `@actions/tool-cache` 4.x (all ESM), GitHub Actions YAML, actionlint 1.7.7.

## Global Constraints

- Paths: Access metadata `access/action.yml`; root `action.yml` generated from it; Serve metadata `serve/action.yml`; bundles `access/dist/{main,post}.js`, `serve/dist/{main,post}.js`; sources/tests `action-src/`.
- `runs`: `using: node24`, `post-if: always()`.
- Inputs exactly: Access `name`, `node-id`, `listen` (required), `protocol`, `relay-url`, `relay-token`, `version`; Serve `name`, `target` (required) + the same four; plus one internal input `job-status` on both (default `${{ job.status }}`, documented as "Internal; do not set", omitted from consumer docs). Outputs exactly: `node-id` on both. Serve readiness = status file has `node_id` (no relay wait; issue #177).
- Readiness limit 60 s, poll 500 ms; Unix stop: SIGTERM, SIGKILL after 5 s.
- Executables: `iroh-tunnel-access`, `iroh-tunnel-serve`, invoked as `<exe> run -c <config>`; child env adds `IROH_TUNNEL_STATE_DIR=<ws>/state`, `NO_COLOR=1`, and on Unix `HOME=<ws>/home`.
- Release URL: `https://github.com/naicoi92/iroh-tunnel/releases/download/v<ver>/`; archives `<exe>-<ver>-<os>-<arch>[-musl].tar.gz` / `.zip`; checksum file `checksums.txt` (Linux, Windows) or `checksums-darwin.txt` (macOS).
- CI-only hook env `IROH_TUNNEL_ACTION_BIN_DIR`; never documented for consumers.
- No change to `src/`, `crates/`, `tests/`, `Cargo.*`. No secret-key input. Workflow text, docs and commits in English.
- `actions/checkout` keeps `persist-credentials: false`; no `${{ }}` interpolated directly into `run:` (use `env:`).

## Review Focus

1. **Access listener fails to bind but the process keeps running** (verified: logs `ERROR failed to bind <addr>`, does not exit) → step must fail at once naming `listen`; pinned by Task 4 test `start fails fast on bind error`.
2. **Relay token or key leaking into logs** via the error path that prints the config or log tail → mask token, never print the config; Task 4 test `no secret in any output` covers start-failure and stop paths.
3. **Windows process left running** because detached children survive the runner's step → post step must kill and verify; Task 3 test `stop kills after grace` plus e2e post-step liveness check (Task 6).
4. **Unspecified listen address probe** (`0.0.0.0`, `[::]`) failing on Windows → probe loopback of the same family; Task 3 test `probe maps unspecified to loopback`.
5. **Committed bundle or root `action.yml` stale** after a source change → `check-dist` in CI; Task 5 step and Task 6 `action` job.

---

## Contracts

| Name | new / modified | Shape | public |
|---|---|---|---|
| Access action path | new | `naicoi92/iroh-tunnel@<ref>` and `naicoi92/iroh-tunnel/access@<ref>` (identical) | yes |
| Serve action path | new | `naicoi92/iroh-tunnel/serve@<ref>` | yes |
| `access/action.yml` | new | `name: iroh-tunnel`; `description`; `branding: {icon: link, color: blue}`; inputs `name`(req), `node-id`(req), `listen`(req), `protocol`(default `tcp`), `relay-url`(default `""`), `relay-token`(default `""`), `version`(default `""`), internal `job-status` (default `${{ job.status }}`, description `Internal; do not set.`); outputs `node-id`; `runs: {using: node24, main: dist/main.js, post: dist/post.js, post-if: always()}` | yes |
| root `action.yml` | new (generated) | byte-identical to `access/action.yml` except `runs.main: access/dist/main.js`, `runs.post: access/dist/post.js`; first line comment `# Generated from access/action.yml by action-src/build.ts; do not edit.` | yes |
| `serve/action.yml` | new | `name: iroh-tunnel-serve`; inputs `name`(req), `target`(req), `protocol`, `relay-url`, `relay-token`, `version`, internal `job-status` (as Access); output `node-id`; `runs` as above with `dist/` | yes |
| Input `relay-url` | new | newline-separated `https://` URLs; empty = n0 public relays | yes |
| Input `version` | new | `X.Y.Z` \| `vX.Y.Z` \| either with `-pre` \| `latest` \| empty = build-time version | yes |
| Env `IROH_TUNNEL_ACTION_BIN_DIR` | new | directory containing `iroh-tunnel-access[.exe]` / `iroh-tunnel-serve[.exe]`; skips download | no (CI-only) |
| Env `IROH_TUNNEL_ACTION_ERROR_FILE` | new | when set, the main step also writes its failure message to this path; used only by `e2e.yml` to assert the Windows limit message | no (CI-only) |
| `type Role` | new | `'access' \| 'serve'` | no |
| `type Protocol` | new | `'tcp' \| 'udp'` | no |
| `type HostPort` | new | `{ host: string; port: number; family: 'ipv4' \| 'ipv6' \| 'name' }` (host stored without brackets) | no |
| `type AccessRequest` | new | `{ role: 'access'; name: string; nodeId: string; listen: HostPort; protocol: Protocol; relayUrls: string[]; relayToken?: string; version: VersionSpec }` | no |
| `type ServeRequest` | new | `{ role: 'serve'; name: string; target: HostPort; protocol: Protocol; relayUrls: string[]; relayToken?: string; version: VersionSpec }` | no |
| `type TunnelRequest` | new | `AccessRequest \| ServeRequest` | no |
| `type VersionSpec` | new | `{ kind: 'default' } \| { kind: 'latest' } \| { kind: 'exact'; version: string }` (version without `v`) | no |
| `class InputError` | new | `extends Error { readonly input: string }` | no |
| `parseAccess(raw: RawInputs): AccessRequest` / `parseServe(raw: RawInputs): ServeRequest` | new | `RawInputs = (name: string) => string` (trimmed `core.getInput`); throw `InputError` | no |
| `renderConfig(req: TunnelRequest): string` | new | TOML: optional `[node]` with `relay_urls = [...]` and `relay_token = "..."`; one `[[services]]` with `name`, `protocol`, `host`, `port` (+ `node_id` for access); never `secret_key` | no |
| `type ReleaseTarget` | new | `{ os: 'linux' \| 'darwin' \| 'windows'; arch: 'amd64' \| 'arm64'; musl: boolean; archiveExt: '.tar.gz' \| '.zip'; checksumFile: 'checksums.txt' \| 'checksums-darwin.txt' }` | no |
| `parseHostPort(s: string, opts: { allowName: boolean }): HostPort` | new | throws `InputError`-compatible `Error`; caller sets `input` | no |
| `parseVersion(s: string): VersionSpec`, `compareVersions(a: string, b: string): number` | new | semver order; pre-release lower than its release | no |
| `class UnsupportedPlatformError` | new | `extends Error { readonly supported: string[] }` | no |
| `isMusl(): boolean` | new | Linux only; `process.report.getReport().header.glibcVersionRuntime === undefined` | no |
| `resolveTarget(platform: string, arch: string, isMusl = false): ReleaseTarget` | new | throws `UnsupportedPlatformError` listing `linux-amd64, linux-arm64, darwin-arm64, windows-amd64, windows-arm64` | no |
| `archiveName(exe: string, version: string, t: ReleaseTarget): string` | new | `${exe}-${version}-${os}-${arch}${musl ? '-musl' : ''}${ext}` | no |
| `BUILD_VERSION`, `MIN_VERSION` | new | string constants injected by `build.ts` (`define`); `BUILD_VERSION` = `[workspace.package] version` in root `Cargo.toml`; `MIN_VERSION` = literal in `build.ts`, the first release that ships the actions (the next release after 0.5.0, confirmed with the owner at apply time) | no |
| `interface ActionsIO` | new | `input(name): string; setOutput(k, v): void; setSecret(v): void; saveState(k, v): void; getState(k): string; info/warning(msg): void; group<T>(title, fn: () => Promise<T>): Promise<T>; setFailed(msg): void; jobFailed(): boolean` (`jobFailed` = `input('job-status')` is `failure` or `cancelled`, or state `main-ok` is absent; the runner re-evaluates `${{ job.status }}` for the post step) | no |
| `interface Installer` | new | `install(role: Role, spec: VersionSpec): Promise<{ exePath: string; version: string }>`; `createInstaller({ releaseRoot, target, cache: ToolCache, fetch: typeof fetch, binDir?: string })` where `releaseRoot` = `https://github.com/naicoi92/iroh-tunnel/releases` (downloads from `${releaseRoot}/download/v<ver>/`, `latest` resolved from the redirect of `${releaseRoot}/latest`), `ToolCache = { find, downloadTool, extractTar, extractZip, cacheDir }` subset of `@actions/tool-cache` | no |
| `interface Workspace` | new | `{ root, stateDir, homeDir, configPath, logPath; writeConfig(toml: string): Promise<void> }` (mode `0o600`); `createWorkspace(runnerTemp: string): Promise<Workspace>`; `removeWorkspace(root: string): Promise<void>` (no-op when missing) | no |
| `interface Launcher` | new | `spawn(exe: string, args: string[], env: Record<string,string>, logPath: string): { pid: number; exitCode(): number \| null }` (exitCode only valid in the main process, which owns the child); `isAlive(pid: number): boolean`; `stop(pid: number, graceMs: number): Promise<'exited' \| 'killed' \| 'still-alive'>` (production grace 5000) | no |
| `readStatus(stateDir, role): Promise<StatusSnapshot \| undefined>` | new | `StatusSnapshot = { node_id: string; services: { name: string; transports?: unknown[] }[] }` from `<role>-status.json` | no |
| `scanLog(text: string, listen: HostPort): LogSignals` | new | `{ bound: boolean; bindError?: string }`; bound iff a line contains `listening for local clients listen_addr=<addr>` or `listening for local udp clients listen_addr=<addr>`; bindError iff a line contains `failed to bind <addr>:` or `failed to bind udp <addr>:` (`<addr>` = `listen` rendered as `host:port`, IPv6 bracketed as the CLI prints it). Never matches `access endpoint ready, listening for local clients` (`run_access.rs:137`) or `failed to bind iroh endpoint` (`endpoint/create.rs:48`) | no |
| `probe(addr: HostPort, timeoutMs: number): Promise<void>` | new | TCP connect then close; `0.0.0.0` → `127.0.0.1`, `::` → `::1` | no |
| `isReady(req, status, log): Readiness` | new | `Readiness = { ready: true } \| { ready: false; needsProbe?: boolean } \| { failed: string }` per D7 | no |
| `runMain(role: Role, deps: StartDeps): Promise<void>` | new | parses inputs with `parseAccess`/`parseServe` from `deps.io.input`, then calls `start`; on `InputError` calls `setFailed` naming the input and returns before any other dep is used | no |
| `start(req: TunnelRequest, deps: StartDeps): Promise<void>` / `stop(deps: StopDeps): Promise<void>` | new | `StartDeps = { io: ActionsIO; installer: Installer; createWorkspace: (runnerTemp: string) => Promise<Workspace>; launcher: Launcher; readStatus: typeof readStatus; readLog: (path: string) => Promise<string>; probe: typeof probe; clock: Clock; platform: NodeJS.Platform; runnerTemp: string; env: Record<string, string \| undefined>; writeFile: (path: string, text: string) => Promise<void> }`; `StopDeps = { io; launcher; readStatus; readLog; removeWorkspace; platform }`; `Clock = { now(): number; sleep(ms: number): Promise<void> }` | no |
| Saved state keys | new | `workspace` (root path) saved right after the workspace is created; `pid` and `role` saved right after spawn; `main-ok` (`'1'`) saved only on success | no |
| `ci.yml` filter `action` + job `action` | new | paths `action.yml`, `access/**`, `serve/**`, `action-src/**`, `.github/workflows/ci.yml`, `Cargo.toml` (bundles embed its version); job runs `bun install --frozen-lockfile`, `bun run typecheck`, `bun test`, `bun run check-dist` in `action-src/`; `ci-ok.needs` gains `action` | no |
| `e2e.yml` steps + job `e2e-action-container` | new | see Task 6; `e2e-ok.needs: [e2e, e2e-action-container]` | no |
| `release.yml` job `action-tag` | new | `needs: [preflight, github-release]`, `if: needs.preflight.outputs.prerelease != 'true'`, `permissions: contents: write`, moves `vX` | no |

---

## Task 1: Toolchain and package skeleton — implements 1.1, 1.2, 1.3, 1.4, 1.5

**Files:**
- Create: `action-src/package.json`, `action-src/tsconfig.json`, `action-src/bun.lock` (by `bun add`), `action-src/build.ts`, `action-src/scripts/check-dist.ts`, `action-src/src/version.ts`, `action-src/test/smoke.test.ts`
- Modify: `.gitattributes` (append generated entries)

**Interfaces:** Produces `BUILD_VERSION`, `MIN_VERSION`, scripts `typecheck`, `test`, `build`, `check-dist`.

- [ ] **Step 1:** In `action-src/`: `bun init -y`, then set `package.json` `{ "name": "iroh-tunnel-action", "private": true, "type": "module", "scripts": { "typecheck": "tsc --noEmit", "test": "bun test", "build": "bun run build.ts", "check-dist": "bun run scripts/check-dist.ts" } }`; `bun add @actions/core@^3 @actions/exec@^3 @actions/tool-cache@^4`; `bun add -d typescript @types/node @types/bun`.
- [ ] **Step 2:** `tsconfig.json`: `strict: true`, `module: "ESNext"`, `moduleResolution: "Bundler"`, `target: "ES2023"`, `types: ["node", "bun"]`, `noEmit: true`, include `src`, `test`, `build.ts`, `scripts`.
- [ ] **Step 3:** `src/version.ts`: `declare const __BUILD_VERSION__: string; declare const __MIN_VERSION__: string; export const BUILD_VERSION = typeof __BUILD_VERSION__ === 'string' ? __BUILD_VERSION__ : '0.0.0-dev'; export const MIN_VERSION = ...same pattern...`.
- [ ] **Step 4:** `build.ts(outRoot = repo root)`: read `version` from the `[workspace.package]` table of root `Cargo.toml` (regex anchored after that header); `MIN_VERSION` is a literal in `build.ts` (see Contracts); `Bun.build` four entries (`src/entry/access-main.ts` → `access/dist/main.js`, `access-post` → `access/dist/post.js`, `serve-main`/`serve-post` → `serve/dist/`), `target: 'node'`, `format: 'esm'`, `minify: false`, `define` both constants; then generate root `action.yml` from `access/action.yml` (string replace of the `main:`/`post:` lines only, prefix the generated-file comment). Accept an `--out <dir>` flag so check-dist can build elsewhere.
- [ ] **Step 5:** `scripts/check-dist.ts`: build into a temp dir with `--out`, byte-compare the six generated files with the committed ones, print the differing paths and exit 1 on any difference.
- [ ] **Step 6:** `.gitattributes`: `access/dist/** linguist-generated=true`, `serve/dist/** linguist-generated=true`, `/action.yml linguist-generated=true`.
- [ ] **Step 7:** `test/smoke.test.ts`: `expect(BUILD_VERSION).toBeString()`. Run `bun install --frozen-lockfile && bun run typecheck && bun test` in `action-src/` — Expected: 1 pass, typecheck clean.
- [ ] **Step 8:** Commit: `build(action): add the bun TypeScript package for the GitHub Actions`

---

## Task 2: Domain — implements 2.1, 2.2, 2.3, 2.4

**Files:** Create `action-src/src/domain/{inputs,hostport,version,config,errors}.ts`; tests `action-src/test/domain/{inputs,hostport,version,config}.test.ts`.

**Interfaces:** Produces `Role`, `Protocol`, `HostPort`, `AccessRequest`, `ServeRequest`, `TunnelRequest`, `VersionSpec`, `InputError`, `parseAccess`, `parseServe`, `renderConfig`.

- [ ] **Step 1: failing tests `hostport.test.ts`:** `parseHostPort('127.0.0.1:65535', {allowName:false})` → `{host:'127.0.0.1',port:65535,family:'ipv4'}`; `'[::1]:5432'` → `{host:'::1',port:5432,family:'ipv6'}`; `'postgres:5432'` with `allowName:true` → family `name`; throws for `'127.0.0.1:0'`, `'127.0.0.1:70000'`, `'::1:5432'`, `'localhost:15432'` with `allowName:false`, `':80'`, `'host'`.
- [ ] **Step 2: failing tests `version.test.ts`:** `''` → default; `'latest'` → latest; `'0.6.0'` and `'v0.6.0'` → exact `0.6.0`; `'v0.6.1-rc.1'` → exact `0.6.1-rc.1`; throws for `'0.6'`, `'v'`, `'latest2'`; `compareVersions('0.5.9','0.6.0') < 0`, pre-release lower than its release.
- [ ] **Step 3: failing tests `inputs.test.ts`** (each error asserts `err.input`): Access minimal (`name: db`, 64-hex node-id, `listen: 127.0.0.1:15432`) parses with protocol `tcp`, `relayUrls: []`; missing `node-id` → `input === 'node-id'`; missing `target` for Serve → `'target'`; `name: My_DB` → `'name'` and message lists `a-z, 0-9, -`; name of 64 bytes → `'name'`; node-id length 10 → `'node-id'`; 52-char base32 accepted; `protocol: sctp` → `'protocol'` with message containing `tcp` and `udp`; `relay-url: relay.example.com` and `http://relay.example.com` → `'relay-url'` with `https://` in message; two `https` lines → two entries in order; `relay-token: "abc\n"` → `'abc'`; `relay-token: "a\nb"` → `'relay-token'` and the message does not contain `a` or `b` values; `version: 0.6` → `'version'`; Access raw map containing `target` → ignored (parser never reads it; assert reader was not called with `'target'`).
- [ ] **Step 4:** Implement `hostport.ts`, `version.ts`, `errors.ts`, `inputs.ts` per contracts (name regex `^[a-z0-9-]{1,63}$`; node-id `^[0-9a-f]{64}$` or `^[a-z2-7]{52}$`, case-insensitive per `dto.rs:30-36`).
- [ ] **Step 5: failing tests `config.test.ts`:** Serve `{name:'db', target: postgres:5432}` renders `[[services]]\nname = "db"\nprotocol = "tcp"\nhost = "postgres"\nport = 5432\n` and no `[node]`; IPv6 target renders `host = "::1"` (verified: binary accepts unbracketed host in config); Access renders `node_id`; relays + token render `[node]\nrelay_urls = ["https://a", "https://b"]\nrelay_token = "t"`; output never contains `secret_key`; TOML string escaping of `"` and `\` in token.
- [ ] **Step 6:** Implement `renderConfig`.
- [ ] **Step 7:** Run `bun test test/domain && bun run typecheck` — Expected: all pass; `grep -R "adapters\|@actions" src/domain` — Expected: no match.
- [ ] **Step 8:** Commit: `feat(action): parse and validate action inputs and render the role config`

---

## Task 3: Adapters — implements 3.1, 3.2, 3.3, 3.4, 3.5, 3.6

**Files:** Create `action-src/src/adapters/{target,release,io,workspace,launcher,status,logscan,probe}.ts`; tests `action-src/test/adapters/*.test.ts`.

**Interfaces:** Consumes `Role`, `VersionSpec`, `HostPort`, `BUILD_VERSION`, `MIN_VERSION`. Produces `ReleaseTarget`, `resolveTarget`, `archiveName`, `Installer`, `ActionsIO`, `Workspace`, `Launcher`, `readStatus`, `scanLog`, `probe`.

- [ ] **Step 1: failing tests `target.test.ts`:** `('linux','x64',false)` → `linux/amd64` glibc `.tar.gz` `checksums.txt`; `('linux','x64',true)` → musl; `('linux','arm64',false)`; `('darwin','arm64')` → `checksums-darwin.txt`; `('win32','x64')` → `windows/amd64` `.zip`; `('win32','arm64')`; `('darwin','x64')` and `('linux','riscv64')` throw listing the five supported targets; `archiveName('iroh-tunnel-serve','0.6.0',musl amd64)` → `iroh-tunnel-serve-0.6.0-linux-amd64-musl.tar.gz`. Implement `target.ts`; musl detection `isMusl()` = `process.report.getReport().header.glibcVersionRuntime === undefined` on Linux.
- [ ] **Step 2: failing tests `release.test.ts`** with a local `Bun.serve` fake release host (`releaseRoot` injected, serving `/download/v<ver>/...` and `/latest` → 302 `/tag/v<ver>`): default spec installs `BUILD_VERSION`; `latest` resolves via the `releases/latest` redirect to the newest stable; exact below `MIN_VERSION` → error naming the minimum, no HTTP request made; 404 → error naming the version and URL; digest mismatch → error naming the archive and nothing extracted (assert extract fake not called); second install of same role+version uses the cache (fake cache hit, zero HTTP); `IROH_TUNNEL_ACTION_BIN_DIR` set → returns `<dir>/iroh-tunnel-access[.exe]` with zero HTTP; missing executable in that dir → error.
- [ ] **Step 3:** Implement `release.ts` with `@actions/tool-cache` (`find`, `downloadTool`, `extractTar`/`extractZip`, `cacheDir`) and `node:crypto` SHA-256; checksum line format `<sha256>  <file>`.
- [ ] **Step 4: failing tests `io.test.ts`:** `setSecret` is called before any `info` that could include the token (spy order); `group` wraps output in `core.startGroup/endGroup`. Implement `io.ts` over `@actions/core`.
- [ ] **Step 5: failing tests `workspace.test.ts`:** creates `<tmp>/iroh-tunnel-<random>/{state,home}`, two calls give different roots; `writeConfig` mode is `0o600` on Unix (skip on Windows); `removeWorkspace` deletes the tree and is a no-op when missing. Implement `workspace.ts`.
- [ ] **Step 6: failing tests `launcher.test.ts`** using a tiny Node fixture script as the child: spawn writes stdout+stderr to the log file and the child gets `IROH_TUNNEL_STATE_DIR`, `NO_COLOR=1` and (Unix) `HOME`; `stop` returns `exited` when the child handles SIGTERM; returns `killed` for a child ignoring SIGTERM after ~5 s (use injected grace of 200 ms in test); `isAlive` false after stop. Implement `launcher.ts` with `child_process.spawn(..., { detached: true, stdio: ['ignore', fd, fd], windowsHide: true })` + `unref()`; Windows stop via `process.kill(pid)`; if still alive after the grace period, `exec.exec('taskkill', ['/PID', pid, '/T', '/F'])` (this is the `@actions/exec` use).
- [ ] **Step 7: failing tests `status.test.ts`, `logscan.test.ts`, `probe.test.ts`:** `readStatus` returns undefined when the file is missing or half-written JSON; `scanLog` detects `listening for local clients listen_addr=127.0.0.1:15432` and `ERROR failed to bind 127.0.0.1:15998: Address already in use` (strings from the verified binary output), and does NOT treat `access endpoint ready, listening for local clients` as bound nor `failed to bind iroh endpoint` as a bind error, nor a bind error for a different address; `probe({host:'0.0.0.0',port:p})` connects to a server on `127.0.0.1:p` (test "probe maps unspecified to loopback"); probe rejects on refused connection within timeout. Implement the three modules.
- [ ] **Step 8:** Run `bun test test/adapters && bun run typecheck` — Expected: all pass.
- [ ] **Step 9:** Commit: `feat(action): add release, process, workspace and status adapters`

---

## Task 4: Use cases start and stop — implements 4.1, 4.2, 4.3, 4.4, 4.5

**Files:** Create `action-src/src/usecases/{start,stop,readiness,failure,run-main}.ts`; tests `action-src/test/usecases/{start,stop,readiness}.test.ts`, `action-src/test/fakes.ts`; Modify `action-src/src/adapters/io.ts` (`jobFailed`), `action-src/test/adapters/io.test.ts`.

**Interfaces:** Consumes all Task 2/3 contracts. Produces `isReady`, `start`, `stop`, saved state keys.

- [ ] **Step 1: failing tests `readiness.test.ts`:** Serve ready iff `node_id` is present (a `home_relay: null` does not block); Access TCP: not bound → not ready; bound and `transports` empty → `needsProbe: true`; bound and non-empty → ready; Access UDP bound → ready; any `bindError` → `failed` containing the address.
- [ ] **Step 2:** Implement `readiness.ts`.
- [ ] **Step 3: failing tests `start.test.ts`** with fakes (fake clock advancing 500 ms per poll): state `workspace` is saved before install and `pid`/`role` right after spawn, so they exist on every failure after spawn (bind error, timeout, early exit) and `stop()` can clean up; happy Serve sets outputs and `main-ok=1`; happy Access TCP calls the prober exactly once then sets `node-id`; UDP Access logs `peer connection not checked` and sets `node-id`; `runMain` with an invalid input → `setFailed` naming the input and installer, workspace and launcher never called; child exits with code 2 → fails immediately with `exit code 2 (config)` and log tail; exit code 6 → message contains `already running for this runner user`, `system service`, and on `platform==='win32'` `one Serve and one Access step per job on Windows`; bind error → fails at the first poll naming `listen` ("start fails fast on bind error"); 120 polls without ready → fails with `not ready within 60 s`, last status and log tail; no outputs set on any failure; with `env.IROH_TUNNEL_ACTION_ERROR_FILE` set, the failure message is written there via `writeFile` ("error file written when env set").
- [ ] **Step 4:** Implement `start.ts` and `runMain` (`setSecret(relayToken)` first; write the failure message to `env.IROH_TUNNEL_ACTION_ERROR_FILE` when set; exit classes map: 1 general, 2 config, 3 permission, 4 iroh, 5 service, 6 conflict per `docs/monitoring.md`), `failure.ts` (message builders, log tail = last 40 lines).
- [ ] **Step 5: failing tests `stop.test.ts`:** no saved state → success, no calls; saved state → prints status inside a group, calls `launcher.stop`, prints the log only when `jobFailed()` is true, removes the workspace, and fails when stop returns `still-alive`; `io.test.ts`: `jobFailed()` true for `job-status` `failure` and `cancelled`, or when `main-ok` is missing; false for `success`.
- [ ] **Step 6:** Implement `stop.ts`; job failure comes from `io.jobFailed()` (internal `job-status` input, re-evaluated by the runner for the post step, or missing `main-ok`).
- [ ] **Step 7: failing test "no secret in any output":** fixture config containing `secret_key = "<base64>"` written by the fake child into the workspace config; run start-failure, timeout and stop paths with `relay-token: tok-123`; assert neither the key nor `tok-123` appears in any captured IO call except `setSecret`.
- [ ] **Step 8:** Run `bun test && bun run typecheck` — Expected: all pass.
- [ ] **Step 9:** Commit: `feat(action): start and stop the tunnel with readiness and diagnostics`

---

## Task 5: Entry points, metadata, bundles — implements 5.1, 5.2, 5.3, 5.4, 5.5

**Files:** Create `action-src/src/entry/{access-main,access-post,serve-main,serve-post,deps}.ts`, `action-src/scripts/local-smoke.ts`, `access/action.yml`, `serve/action.yml`, generated `access/dist/*`, `serve/dist/*`, `action.yml`; test `action-src/test/metadata.test.ts`.

**Interfaces:** Consumes `start`, `stop`, `parseAccess`, `parseServe`, adapters. Produces the public contracts `access/action.yml`, `serve/action.yml`, root `action.yml`.

- [ ] **Step 1: failing test `metadata.test.ts`** (parse YAML with `Bun.YAML.parse`): root and `access/action.yml` differ only in `runs.main`/`runs.post` lines and the generated comment; both declare exactly the Access inputs/outputs of Global Constraints (including internal `job-status` with default `${{ job.status }}`); `serve/action.yml` declares exactly the Serve ones; all three have `using: node24`, `post-if: always()`; root has `name: iroh-tunnel`, `branding.icon`, `branding.color`; `access/action.yml` does not declare `target`, `serve/action.yml` does not declare `listen`/`node-id`.
- [ ] **Step 2:** Write `access/action.yml` and `serve/action.yml` per Contracts (input descriptions one line each; `relay-token` description says to pass a secret; internal `job-status` input last).
- [ ] **Step 3:** Entry modules: `main` = `runMain('access' | 'serve', realStartDeps())` with top-level `try/catch` → `setFailed`; `post` = `stop(realStopDeps())`. `realStartDeps` lives in `src/entry/deps.ts`.
- [ ] **Step 4:** Run `bun run build` then `bun test test/metadata.test.ts` and `bun run check-dist` — Expected: pass, `check-dist` prints nothing.
- [ ] **Step 5:** Local smoke (macOS/Linux dev box, manual): `cargo build --locked --bin iroh-tunnel-serve --bin iroh-tunnel-access`; run `node access/dist/main.js` style harness `action-src/scripts/local-smoke.ts` that sets `INPUT_*`, `RUNNER_TEMP`, `GITHUB_OUTPUT`, `GITHUB_STATE`, `IROH_TUNNEL_ACTION_BIN_DIR=target/debug` for Serve then Access (and converts each `GITHUB_STATE` file line `k=v` into env `STATE_k=v` before running the matching post script), checks the workspace config mode is still `600` after the binary saved its key ("Key file permissions"), runs one UDP datagram round trip through a UDP Serve/Access pair ("UDP service"), against a local echo server, does a round trip, then runs both post scripts — Expected: round trip bytes equal, both posts report stopped, no `iroh-tunnel-` process left (`pgrep -f iroh-tunnel-` empty).
- [ ] **Step 6:** Commit: `feat(action): add the Access and Serve action entry points and metadata`

---

## Task 6: CI coverage — implements 6.1, 6.2, 6.3, 6.4, 6.5

**Files:** Modify `.github/workflows/ci.yml`, `.github/workflows/e2e.yml`; create `action-src/scripts/roundtrip.ts`, `action-src/scripts/echo-server.ts` (used by e2e).

**Interfaces:** Consumes the action paths, `IROH_TUNNEL_ACTION_BIN_DIR`. Produces `ci.yml` filter/job `action`, e2e steps, job `e2e-action-container`.

- [ ] **Step 1:** `ci.yml`: add filter `action` with the five paths plus `Cargo.toml` (it feeds `BUILD_VERSION`, so a version bump re-runs `check-dist`); docs paths stay out of the filter so "Docs-only change" keeps skipping every check (`check-docs` therefore runs only when action paths change; accepted); job `action` (`needs: changes`, `if: needs.changes.outputs.action == 'true'`, `ubuntu-latest`, checkout, `oven-sh/setup-bun@v2` with `bun-version: 1.4.4`, `working-directory: action-src` steps `bun install --frozen-lockfile`, `bun run typecheck`, `bun test`, `bun run check-dist`); `ci-ok.needs: [changes, lint, unit, scripts, action]`; header diagram row `action (bun typecheck + test + check-dist)`.
- [ ] **Step 2:** `action-src/scripts/roundtrip.ts <host:port> <payload>`: connect, write payload, half-close, read to EOF, exit 0 iff equal; plus `echo-server.ts <port>` (TCP echo) run in background by the workflow.
- [ ] **Step 3:** `e2e.yml` job `e2e`, after the existing suite, steps (all OS, every `run:` with `shell: bash`, echo server started with `nohup ... &`): `cargo build --release --locked --bin iroh-tunnel-serve --bin iroh-tunnel-access`; set `IROH_TUNNEL_ACTION_BIN_DIR=${{ github.workspace }}/target/release` via `$GITHUB_ENV`; setup bun; start echo server on 18080; `- id: serve1 uses: ./serve with: {name: echo, target: 127.0.0.1:18080}`; `- uses: ./ with: {name: echo, node-id: ${{ steps.serve1.outputs.node-id }}, listen: 127.0.0.1:19080}`; `bun action-src/scripts/roundtrip.ts 127.0.0.1:19080 hello-$RUNNER_OS`.
- [ ] **Step 4:** Linux/macOS (`if: matrix.name != 'windows'`): second echo on 18081, `- id: serve2 uses: ./serve` `name: echo2`, `uses: ./access` with its node-id on 19081, round trip on 19081 and again on 19080 (both listeners live), and a step with `env: A: ${{ steps.serve1.outputs.node-id }}, B: ${{ steps.serve2.outputs.node-id }}` asserting `[ "$A" != "$B" ]`. Windows (`if: matrix.name == 'windows'`): `- id: second uses: ./access continue-on-error: true` (same inputs as the first Access), with `listen: 127.0.0.1:19082` (a free port, so exit 6 is the only possible failure) and step `env: IROH_TUNNEL_ACTION_ERROR_FILE: ${{ runner.temp }}/second-error.txt`, then a step asserting `steps.second.outcome == 'failure'` and that the file contains `one Serve and one Access step per job on Windows`. The error-file write is implemented and unit-tested in Task 4 Step 4 (`failure.ts`).
- [ ] **Step 5:** In the Linux cell of `e2e`, after the build, `actions/upload-artifact@v4` name `action-bins`, paths `target/release/iroh-tunnel-serve` and `target/release/iroh-tunnel-access`. New job `e2e-action-container`: `needs: e2e`, `runs-on: ubuntu-latest`, `container: ubuntu:24.04`, `services: { web: { image: nginx:alpine } }`; steps (shell bash): `apt-get update && apt-get install -y --no-install-recommends ca-certificates curl git`; `actions/checkout@v7` with `persist-credentials: false`; `actions/download-artifact@v4` name `action-bins` to `bins/`; `chmod +x bins/*` (artifacts drop modes); `echo "IROH_TUNNEL_ACTION_BIN_DIR=$GITHUB_WORKSPACE/bins" >> "$GITHUB_ENV"`; `- id: web uses: ./serve with: {name: web, target: web:80}`; `uses: ./ with: {name: web, node-id: ${{ steps.web.outputs.node-id }}, listen: 127.0.0.1:18090}`; `curl -fsS http://127.0.0.1:18090/ | grep -q 'Welcome to nginx'`. `e2e-ok.needs: [e2e, e2e-action-container]`.
- [ ] **Step 6:** Run `actionlint` (1.7.7) — Expected: no output. Run `git diff --stat -- tests crates src Cargo.toml Cargo.lock` — Expected: empty. `bun test` — Expected: pass.
- [ ] **Step 7:** Commit: `ci: test the GitHub Actions on pull requests and end to end on main`

---

## Task 7: Release major tag — implements 7.1, 7.2

**Files:** Modify `.github/workflows/release.yml`, `.github/workflows/ci.yml` (`scripts` job step); create `.github/scripts/move-major-tag.sh`, `.github/scripts/move-major-tag.test.sh`.

**Interfaces:** Consumes `needs.preflight.outputs.version`, `prerelease`. Produces job `action-tag`.

- [ ] **Step 1: failing check** `move-major-tag.sh --dry-run <tag> <tag-list-file>`: (tag-list file: one tag per line) prints `move v0 -> v0.6.0` for lines `v0.5.0`, `v0.6.0`; prints `skip: v0.5.9 is not the highest stable v0 release (v0.6.0)` for tag `v0.5.9` with lines `v0.5.0`, `v0.6.0`, `v0.5.9`; prints `skip: pre-release` for `v0.6.1-rc.1`. Write these three cases plus "API failure prints the manual command and exits 1" (stub `gh` as a shell function returning 1) as a bash test `.github/scripts/move-major-tag.test.sh`; add `bash .github/scripts/move-major-tag.test.sh` to the `scripts` job in `ci.yml`.
- [ ] **Step 2:** Implement `move-major-tag.sh <tag>` with functions `highest_stable <major> < tag-list` (one tag per line on stdin, drop tags containing `-`, `sort -V | tail -1`) and `move_ref`, the `gh api` calls of Step 3; `--dry-run <tag> <file>` reads the tag list from `<file>` (one per line) and only prints the decision; on API failure print `move it by hand: git tag -f v${major} ${tag}^{commit} && git push -f origin refs/tags/v${major} (this push re-triggers release.yml, whose preflight rejects v${major}; ignore that run)` and exit 1. `set -euo pipefail`.
- [ ] **Step 3:** `release.yml` job `action-tag`: `needs: [preflight, github-release]`, `if: needs.preflight.outputs.prerelease != 'true'`, `permissions: contents: write`, no checkout; `env: GH_TOKEN: ${{ github.token }}, GH_REPO: ${{ github.repository }}, TAG: ${{ github.ref_name }}`; run the script, which uses the REST API via `gh api` instead of git: list tags `gh api --paginate repos/$GH_REPO/git/matching-refs/tags/v${major}. --jq '.[].ref' | sed 's#^refs/tags/##'`, resolve the tag commit `gh api repos/$GH_REPO/commits/$TAG --jq .sha`, then `gh api -X PATCH repos/$GH_REPO/git/refs/tags/v${major} -f sha=<sha> -F force=true`, falling back to `gh api -X POST repos/$GH_REPO/git/refs -f ref=refs/tags/v${major} -f sha=<sha>` when the ref does not exist yet. Keeps the repo-wide `persist-credentials: false` rule (no checkout at all). A `GITHUB_TOKEN` ref update does not trigger `release.yml`.
- [ ] **Step 4:** Run `bash .github/scripts/move-major-tag.test.sh` — Expected: 4 passed. `shellcheck .github/scripts/*.sh` and `actionlint` — Expected: no output.
- [ ] **Step 5:** Commit: `ci(release): move the major action tag after a stable release`

---

## Task 8: Documentation — implements 8.1, 8.2, 8.3, 8.4, 8.5, 8.6

**Files:** Create `docs/github-actions.md`, `action-src/scripts/check-docs.ts`; modify `README.md` (new section after "Use cases"), `docs/README.md` (Guides row), `CONTRIBUTING.md` (new "GitHub Actions" section + Releasing bullets), `CHANGELOG.md` (`## [Unreleased]`), `action-src/package.json` (script `check-docs`), `.github/workflows/ci.yml` (`action` job step `bun run check-docs`).

**Interfaces:** Consumes the public contracts.

- [ ] **Step 1:** `docs/github-actions.md` sections (the internal `job-status` input is not documented): Access (example to a private service), Serve (job service example; `container:` example with `target: postgres:5432`), Two services, Own relay (`relay-url` + `relay-token: ${{ secrets.IROH_RELAY_TOKEN }}`), Serve + Access in one job, Inputs/outputs tables (per role), Behaviour (60 s readiness; Serve does not wait for relay registration, so embedded clients should retry dials for a few seconds (issue #177); Access opens and closes one connection to trigger the dial; job-end stop; status always printed, log printed when the job failed), Versions (`@v0`, `@vX.Y.Z`, `version`, `latest`), Platforms (Linux amd64/arm64 incl. musl containers, macOS arm64, Windows amd64/arm64), Windows limit, Embedded clients (dial `node-id`, ALPN `iroh-tunnel/<name>`, one bidirectional stream per TCP connection, raw bytes, same relays).
- [ ] **Step 2:** README "GitHub Actions" section: one Access and one Serve snippet, link to the page; `docs/README.md` row `[GitHub Actions](github-actions.md) | Access and Serve steps for CI/CD`.
- [ ] **Step 3:** CONTRIBUTING: "GitHub Actions" section (`action-src/`, `bun install`, `bun test`, `bun run build` and commit `access/dist`, `serve/dist`, `action.yml`; `MIN_VERSION` value and when to change it); Releasing bullets: in the release-bump PR, after bumping `Cargo.toml`, run `bun run build` in `action-src/` and commit the bundles (CI `check-dist` fails otherwise); use bun 1.4.4; `v0` moves automatically after a stable release; tick "Publish this Action to the GitHub Marketplace" when publishing each release (accept the Marketplace agreement on the first).
- [ ] **Step 4:** CHANGELOG `## [Unreleased]` → `### Added` → `- GitHub Actions: Access (naicoi92/iroh-tunnel) and Serve (naicoi92/iroh-tunnel/serve) for CI/CD.`
- [ ] **Step 5:** `check-docs.ts`: extract every fenced `yaml` block (parse with `Bun.YAML.parse`; walk the value recursively and check every object with a `uses` starting `naicoi92/iroh-tunnel`, mapping `@`-root and `/access` to `access/action.yml`, `/serve` to `serve/action.yml`) in `docs/github-actions.md` and README that contains `naicoi92/iroh-tunnel`, parse it, and assert each `with:` key is declared in the matching metadata file and is not `job-status`; run it in the `action` CI job (`bun run check-docs`). Run it — Expected: pass.
- [ ] **Step 6:** Run `openspec validate github-actions-serve-access --strict` — Expected: valid. Commit: `docs: document the GitHub Actions`
- [ ] **Step 7 (needs owner permission to push/merge):** on the PR, `action` job green; after merge, `e2e (linux|macos|windows)` and `e2e-action-container` green with round trips and post-step `stopped` lines; record run URLs in `apply.md`.

---

## Scenario → test

**github-actions-integration**
- Access through the root path → e2e `e2e` step `uses: ./` round trip (Task 6 Step 3)
- Access through the alias path → e2e Linux/macOS `uses: ./access` round trip (Task 6 Step 4); `metadata.test.ts` root ≡ access
- Serve path → e2e `uses: ./serve` (Task 6 Step 3)
- Root metadata → `metadata.test.ts` "root has name, branding, node24, post-if"
- Serve metadata → `metadata.test.ts` "serve runs node24 with post"
- Root metadata mirrors the access directory → `metadata.test.ts` "root differs only in runs paths"; `check-dist`
- Minimal Access step → `inputs.test.ts` "access minimal defaults"; e2e round trip
- Missing required Access input → `inputs.test.ts` "missing node-id"; `start.test.ts` "validation error never installs"
- Input of the other role → `inputs.test.ts` "access ignores target"; `metadata.test.ts` "access does not declare target"
- Several Access services → e2e Linux/macOS round trips on 19080 and 19081 after both Access steps started (Task 6 Step 4)
- Serve in front of a runner-mapped job service → e2e Serve on `127.0.0.1:18080` (Task 6 Step 3)
- Serve in a container job with a hostname target → e2e `e2e-action-container` (Task 6 Step 5); `config.test.ts` hostname host
- Missing required Serve input → `inputs.test.ts` "missing target"
- UDP service → `config.test.ts` "udp protocol rendered"; `readiness.test.ts` "udp access bound is ready"; `local-smoke.ts` UDP datagram round trip (Task 5 Step 5)
- Two relays → `inputs.test.ts` "two relay lines in order"; `config.test.ts` "relays and token"
- Invalid service name → `inputs.test.ts` "name My_DB"
- Valid input → `inputs.test.ts` "access minimal defaults"
- Malformed NodeId → `inputs.test.ts` "node-id length 10"
- Unknown protocol → `inputs.test.ts` "protocol sctp"
- Relay URL not https → `inputs.test.ts` "relay-url without scheme / http"
- Relay token with a trailing newline → `inputs.test.ts` "token trimmed"
- Multi-line relay token → `inputs.test.ts` "multi-line token rejected without value"
- Malformed version → `version.test.ts` "0.6 rejected"
- Port out of range → `hostport.test.ts` "port 0 and 70000"
- Port boundary accepted → `hostport.test.ts` "port 65535"
- IPv6 address → `hostport.test.ts` "bracketed ipv6 / unbracketed rejected"
- Hostname not allowed for listen → `hostport.test.ts` "localhost listen rejected"
- Concurrent jobs → `workspace.test.ts` "two workspaces differ" + e2e two Serve steps get different node-ids (Task 6 Step 4 assert step); the three OS cells of `e2e` run concurrently, each with its own node-ids
- Key file permissions → `workspace.test.ts` "config mode 0600"; `local-smoke.ts` mode still `600` after the binary saved its key (Task 5 Step 5)
- No key leaks → `start.test.ts` "no secret in any output"
- Key removed at job end → `stop.test.ts` "removes workspace"
- Own relay with a token → `io.test.ts` "setSecret before use"; `config.test.ts` "relays and token" (no e2e: the CI relay is self-signed, which production binaries reject)
- Default relays → `config.test.ts` "no [node] without relays"
- Default version → `release.test.ts` "default installs BUILD_VERSION"
- Release not found → `release.test.ts` "404 names version and URL"
- Version too old → `release.test.ts` "below MIN_VERSION, no request"
- Latest → `release.test.ts` "latest resolves newest stable"
- Checksum mismatch → `release.test.ts` "digest mismatch, nothing extracted"
- Cached binary → `release.test.ts` "second install uses cache"
- Unsupported runner → `target.test.ts` "darwin x64 and linux riscv64 rejected"
- Alpine container job → `target.test.ts` "linux x64 musl"
- Ubuntu runner → `target.test.ts` "linux x64 glibc"
- Later steps use the tunnel → e2e round trip runs in a later step than the actions (Task 6 Step 3)
- No outputs on failure → `start.test.ts` "no outputs on any failure"
- Access ready → `readiness.test.ts` "bound + transports"; `start.test.ts` "probe once then ready"
- Serve ready → `readiness.test.ts` "serve ready with node_id"; e2e Access round trip right after the Serve step
- home_relay null → `readiness.test.ts` "serve ready with node_id even when home_relay is null"
- UDP Access step → `start.test.ts` "udp logs not checked"
- Readiness timeout → `start.test.ts` "not ready within 60 s"
- Tunnel exits during start → `start.test.ts` "exit code 2 fails immediately"
- Listen port taken → `start.test.ts` "start fails fast on bind error"; `logscan.test.ts` "failed to bind line"
- Normal stop → `stop.test.ts` "stops and succeeds"; `launcher.test.ts` "stop exited"
- Main step failed before starting → `stop.test.ts` "no state is a no-op"
- Successful job → `stop.test.ts` "status printed, log not printed"
- Failed job → `stop.test.ts` "log printed when job failed"
- Two Serve steps on Linux → e2e Linux second Serve step (Task 6 Step 4)
- Concurrent jobs on one self-hosted runner user → `workspace.test.ts` + `launcher.test.ts` "child env HOME and STATE_DIR per workspace" (isolation mechanism); not exercised on a shared self-hosted runner (none in CI) — noted in verify
- Second Serve step on Windows → `start.test.ts` "exit 6 on win32 names the limit"; e2e asserts the same message for a second Access step on Windows (same lock mechanism; the Serve wording is unit-tested only)
- Access and Serve together on Windows → e2e Windows Serve + Access round trip (Task 6 Step 3)
- System service on a self-hosted Linux runner → `start.test.ts` "exit 6 already running message"
- Copy-paste adoption → `check-docs.ts` README examples use declared inputs only
- Docs page examples → `check-docs.ts` + review of section list (Task 8 Step 1)
- Embedded client contract → review of `docs/github-actions.md` "Embedded clients" section (Task 8 Step 1)

**ci-cd-pipeline**
- Code change → unchanged `lint`/`unit` jobs; actionlint (Task 6 Step 6)
- Installer script change only → unchanged `scripts` job; actionlint
- Workflow change only → unchanged `scripts` job; actionlint
- Action change only → `ci.yml` filter `action` + job `action` (Task 6 Step 1); verified on the PR run (Task 8 Step 7)
- Version bump → `Cargo.toml` in the `action` filter (Task 6 Step 1); observed on the next release-bump PR
- Mixed change → filter includes `.github/workflows/ci.yml`; verified on this change's own PR (it touches ci.yml and action-src, so lint, unit and action all run)
- Docs-only change → unchanged; `action` filter excludes `docs/**`
- A check fails → `ci-ok.needs` includes `action` (Task 6 Step 1)
- Superseded push → unchanged concurrency group
- Suite contents → `git diff --stat -- tests` empty (Task 6 Step 6)
- Script lint → unchanged `scripts` job; `move-major-tag.sh` covered by `git ls-files '*.sh'`; `move-major-tag.test.sh` run in `scripts` (Task 7 Step 1)
- Action tests → `action` job runs `bun test` with fakes only (Tasks 2-5)
- Container job on Linux → `e2e-action-container` (Task 6 Step 5)
- Actions on every OS → `e2e` action steps on the matrix (Task 6 Step 3)
- Action regression → `e2e-ok.needs` and action steps without `continue-on-error` (Task 6 Steps 3, 5)
- No release download in CI → `IROH_TUNNEL_ACTION_BIN_DIR` set; `release.test.ts` "BIN_DIR makes zero HTTP"
- Alias path on Linux and macOS → Task 6 Step 4
- Windows limit asserted → Task 6 Step 4 Windows branch
- Stable release → `move-major-tag.test.sh` case 1
- Pre-release → `move-major-tag.test.sh` case 3; job `if:`
- Release job failed → `needs: github-release` (job skipped when it fails)
- Older patch released later → `move-major-tag.test.sh` case 2
- Tag move fails → `move-major-tag.test.sh` "API failure prints the manual command and exits 1" (Task 7 Step 1)
- Marketplace step → CONTRIBUTING Releasing bullet (Task 8 Step 3)
