# Issue #97 evidence — packaged executables on the release candidate

Release candidate `v0.4.3-rc.3`, tag commit `129d39f692db3543998b4d9acf3926d8005f1acb`. This is a **draft prerelease**: nothing is published. It is partial evidence for RELEASE-MATRIX-01; it is not release acceptance (RELEASE-GATE-01 stays NOT_RUN).

## What ran

| Item | Value |
|---|---|
| Release run (build, sign, verify, draft) | [41782593181](https://github.com/naicoi92/iroh-tunnel/actions/runs/41782593181), all jobs success, including `verify signed artifacts` |
| Smoke run | [41786503461](https://github.com/naicoi92/iroh-tunnel/actions/runs/41786503461), 7 legs and `coverage summary` success |
| Harness commit | `7c2a433` (branch `test/97-smoke-role-logs`, PR #156); executables come only from the tag's `release-set` |
| Lockfile | `Cargo.lock` sha256 `cc66f0f9a330…` on Linux/macOS legs; `f7b2a1b53da5…` on Windows. The Windows value is the same file with CRLF line endings from the runner's checkout (appending CR to each line of `main`'s `Cargo.lock` reproduces it), not a different lockfile |
| Relay | public n0 relays (production executables accept only verified `https://` relays) |

Each leg downloaded the three role archives of its platform, checked each against the signed `checksums.txt` / `checksums-darwin.txt`, extracted them, confirmed `--version` contains `0.4.3-rc.3`, and ran four sessions: {all-in-one `iroh-tunnel serve|access`, standalone `iroh-tunnel-serve` / `iroh-tunnel-access`} x {TCP, UDP}. Each session forwarded exact request and response bytes through two separate processes with separate state directories. Evidence JSON per leg (archive digests, executable digests, user, elevation, commands) is the `smoke-evidence-<leg>` artifact of the smoke run (retention 90 days).

| Leg | Runner | User / elevated | Sessions | Reported path kinds |
|---|---|---|---|---|
| linux-amd64 (glibc) | ubuntu-22.04 | runner / no | 4 PASS | direct, relay |
| linux-arm64 (glibc) | ubuntu-22.04-arm | runner / no | 4 PASS | direct, relay |
| linux-amd64-musl | ubuntu-latest | runner / no | 4 PASS | direct, relay |
| linux-arm64-musl | ubuntu-24.04-arm | runner / no | 4 PASS | direct, relay |
| darwin-arm64 | macos-15 | runner / no | 4 PASS | direct, relay |
| windows-amd64 | windows-latest | runneradmin / yes | 4 PASS | direct, relay |
| windows-arm64 | windows-11-arm | runneradmin / yes | 4 PASS | direct, relay |

28 of 28 sessions passed, each on its first attempt (attempts = 1 per session).

## Findings from getting here

Runs 41784524957, 41785531176 and 41786010540 were failures of the harness, not of the product, and are kept as a record:

- Windows x64 and ARM64 exited with code 2: `could not resolve the LocalAppData folder`. The harness had overridden `USERPROFILE`, `APPDATA` and `LOCALAPPDATA`; the real profile is now kept. The packaged executables themselves work on both Windows architectures.
- `linux-arm64` once failed to compile the harness (`can't find crate for iroh`) from a restored Rust cache; the workflow now uses a per-run cache key.
- The first run had no role log in the evidence; the error now carries the log tail.

## Not verified (disclosed)

- **Path kind is observed, not forced.** `reported_path_kinds` lists what the status file saw (direct and relay candidates). The packaged executables cannot be pointed at a private relay, so a forced-relay or forced-direct run is not shown here; that is covered only by `tests/udp_tunnel.rs` with a test build.
- Loss, delay, congestion, path migration, NAT: not run.
- `linux-riscv64gc-musl` archives: not run (no hosted runner). macOS x64: no archive is released.
- Aliases (`it`, `its`, `ita`), short options, configured and service modes, installers, packages, images and `update` on these files: not covered by this smoke.
- Windows privileged account: the runner is `runneradmin` (elevated); a standard user was not tested here.
- The smoke compares archives with `checksums.txt` but does not verify the Sigstore signature itself; that is the `verify signed artifacts` job of the release run.
- Retries: up to 3 attempts per session are allowed; all 28 passed on the first.
