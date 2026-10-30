---
summary: Evidence for the Iroh 1.4.1 dependency upgrade (#101).
---

# Issue #101 evidence: Iroh dependency upgrade

## Selection

- Checked crates.io on 2026-10-30: `iroh` and `iroh-relay` latest stable is `1.4.1` (published 2026-10-21, not yanked). Previous: 1.4.0, 1.3.0, 1.2.0, 1.1.0.
- Before: `Cargo.toml` requested `1.1`, `Cargo.lock` recorded `iroh 1.1.0`.
- After: `iroh = "1.4.1"` (dependency and dev-dependency with `test-utils`) and `iroh-relay = "1.4.1"`; `cargo update -p iroh -p iroh-relay` moved `noq*` 1.2.0 → 1.4.0, `rustls` 0.23.41 → 0.23.46 and other transitive crates (see `Cargo.lock` diff).
- No source changes were required: the upgrade compiled without API changes and no supported behavior or platform was dropped.

## Verification (macOS, aarch64, Darwin 24.6)

| Command | Result |
|---|---|
| `cargo check --all-targets --all-features` | pass |
| `cargo clippy --all-targets --all-features -- -D warnings` | pass |
| `cargo test --all-features` | 122 unit + all integration tests pass (relay auth, status files, path logging, multiplex, quick run) |
| `cargo test --all-features --test serve_access_tunnel -- --ignored` | 4/4 pass over the real n0 relay network |

## Not covered

- `interop_0_4` (needs digest-pinned release 0.4 and current executables; see `tests/evidence/issue-85/README.md`): not run here.
- Windows and Linux: not run in this environment; covered by CI on the PR.
- UDP benchmarking and #97 final-artifact verification must rerun against the lockfile at their own commit; they are not part of this change.
