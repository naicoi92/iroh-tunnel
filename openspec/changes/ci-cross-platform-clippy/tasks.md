## 1. Per-OS lint in the pull-request and cache workflows

Blocked by: none

Files: `.github/workflows/ci.yml`, `.github/workflows/cache.yml`, `CONTRIBUTING.md`. Platforms: Linux, macOS and Windows runners (CI only; no Rust source changes).

- [x] 1.1 `ci.yml`: turn `lint` into an OS matrix (`lint (linux|macos|windows)`, `fail-fast: false`) running `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` on each OS; keep the rustfmt check and the rustdoc build only on Linux; keep the job id `lint` so `ci-ok` still needs it; update the header diagram (scenarios: Code change, Platform-only lint finding, A check fails, Broken rustdoc link, Mixed change; design D1-D3, D5)
- [x] 1.2 `cache.yml`: turn the `lint` warm-up into the same three-OS matrix with the same clippy command and `cache-key: lint`; update the header comment (scenario: Dependency bump merged; design D4)
- [x] 1.3 `CONTRIBUTING.md`: name the pull-request checks `lint (linux|macos|windows)` and `unit (linux|macos|windows)`, and say clippy must pass on every OS
- [x] 1.4 Verify statically: `actionlint` on the workflows passes; `openspec validate ci-cross-platform-clippy --strict` passes; the workflow inventory is unchanged (scenario: Workflow inventory); the expected clippy command passes locally with `cargo +1.91` for the host and for the Linux and Windows targets (record the cross-lint commands used)
- [ ] 1.5 Verify in CI: push the branch only with the user's explicit approval and confirm on the pull request that `lint (linux)`, `lint (macos)`, `lint (windows)` all run and pass and `ci-ok` passes; otherwise record this check as pending in the receipt
