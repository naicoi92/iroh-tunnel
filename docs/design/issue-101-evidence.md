# Issue #101 evidence: Iroh dependency upgrade

Covers the dependency upgrade only. It is not redesign release acceptance; #97 owns that.

## Selection

- Checked crates.io on 2026-10-30: `iroh` and `iroh-relay` latest stable is `1.4.1` (published 2026-10-21, not yanked). Previous: 1.4.0, 1.3.0, 1.2.0, 1.1.0.
- Before: `Cargo.toml` requested `1.1`, `Cargo.lock` recorded `iroh 1.1.0`.
- After: `iroh = "1.4.1"` (dependency and dev-dependency with `test-utils`) and `iroh-relay = "1.4.1"`, both caret requirements; `Cargo.lock` holds the exact resolution. `cargo update -p iroh -p iroh-relay` moved `noq*` 1.2.0 to 1.4.0 and `rustls` 0.23.41 to 0.23.46, replaced the `hickory-*` DNS stack with `n0-dns-resolver`, and removed `moka`, `uuid` and other transitive crates (see the `Cargo.lock` diff). The lockfile keeps a single `rustls` and a single `tokio`.
- Compatibility constraints: MSRV stays 1.91 (no new language or std features were used; CI builds on Linux and Windows). The release wire protocol is unchanged (`iroh-tunnel/{name}` ALPN); no source changes were required, and no supported behavior or platform was dropped.

## Verification (macOS arm64, Darwin 24.6)

| Command | Result |
|---|---|
| `cargo check --all-targets --all-features` | pass |
| `cargo clippy --all-targets --all-features -- -D warnings` | pass |
| `cargo test --all-features` | 122 unit tests and all non-ignored integration tests pass (relay auth, status files, path logging, multiplex, quick run) |
| CI on the PR (Linux and Windows lint and test) | pass |

Local runs did not use `--locked`; the lockfile was freshly updated and CI resolves from it.

## Release 0.4 interoperability on Iroh 1.4.1

Harness: `tests/evidence/issue-85/tcp_compat.py`, the script the ignored `interop_0_4` test wraps. Public relay `https://use1-1.relay.n0.iroh.link/`, macOS arm64.

- Current executable: `cargo build --locked --bin iroh-tunnel` in a separate target directory (`iroh-tunnel 0.4.2-dev`, Iroh 1.4.1), SHA256 `cb63de702f07de5c27b7438d26b835c96191299e35658bed07752d8b13c5b1f5`. `cargo tree --locked --edges normal,build,features -i iroh` contains no `test-utils`, so this production binary verifies relay TLS.
- Released executables (archive checksums matched `checksums-darwin.txt`; digests equal the issue-85 provenance): v0.4.0 `431d9f4d14effd398617b3d62adcf182a86533395d3cfac4426a31361d51c9f5`, v0.4.2 `df09e20c808331f4f387dcf4233aa4eb7a2af94204042c991e0f7a5f226b836c`.
- Result: 6/6 PASS against each release (old/old, current/current, old Access to current Serve, current Access to old Serve, and old Access `multiplex = false` against old and current Serve). Raw run logs are kept outside the repository by the repository owner's decision.
- Limits as in issue-85: relay path only, one macOS arm64 host, v0.4.1 not run, not final release artifacts.

## Not covered

- The four ignored public-relay tests in `tests/serve_access_tunnel.rs` are **blocked** under RS-15: the available test-feature binary disables relay TLS verification, so they must not run against public relays. An earlier local run of them is withdrawn and not counted as evidence.
- Status, relay-auth and path-logging behavior is covered only by the `cargo test` results above, without committed logs.
- Linux and Windows runtime behavior beyond CI, v0.4.1, direct (non-relay) paths, and DNS behavior of the new `n0-dns-resolver` stack in restricted networks.
- UDP benchmarking and #97 final-artifact verification must consume the lockfile at their own commit and rerun affected evidence.
