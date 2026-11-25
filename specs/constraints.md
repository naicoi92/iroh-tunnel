# Constraints

These are the project-wide baseline for changes. Capability specifications give the detailed behavioral contracts. The CI commands below describe existing automation, not a claim that every check ran locally.

## Quality and security floor

- Do not add secrets to source, logs, test output or release artifacts.
- Do not weaken tests, assertions, validation, security controls or checks merely to make a change pass. Explain and review any necessary removal, suppression or exception.
- Do not replace required behavior with an unimplemented stub. Changes to observable behavior must be reflected in the relevant living specification and verified by tests.
- Preserve the supported platforms and Rust minimum version declared by the project unless an explicit change approves otherwise.
- Preserve transport security, identity handling and release-binary verification requirements described in the relevant specifications.

## Existing automated checks

| Scope | Check | Where |
| --- | --- | --- |
| Rust code | `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`; `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked` | PR CI for relevant changes; Clippy runs on Linux, macOS and Windows |
| Rust unit and doc tests | `cargo test --workspace --all-features --locked --lib --bins` and `cargo test --workspace --all-features --locked --doc` | PR CI for relevant changes on Linux, macOS and Windows |
| GitHub Actions | In `action-src`: `bun run typecheck`, `bun test`, `bun run check-dist`, `bun run check-docs` | PR CI for relevant changes |
| Scripts and workflows | ShellCheck, actionlint, PSScriptAnalyzer and the major-tag script test | PR CI for relevant changes |
| Integration and end-to-end | `cargo test --workspace --all-features --locked --test '*'`, plus platform and action-specific checks | Push to `main` and manual dispatch; release preflight requires successful main E2E |

See `.github/workflows/ci.yml` and `.github/workflows/e2e.yml` for exact conditions and setup. CI jobs are path-filtered where specified; this document does not assert branch-protection settings.

## Not yet established

No numeric coverage, performance or security-scanning thresholds have been agreed or measured. Do not present these dimensions as enforced until a threshold and a working check have been chosen and verified.
