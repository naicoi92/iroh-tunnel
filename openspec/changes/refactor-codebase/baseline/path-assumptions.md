# Path assumptions baseline

Hits for `src/`, `tests/`, `Cargo.toml`, `[package]`, `version =`, `include_str!`/`include_bytes!` outside the source tree.

| file | line | text | updated by task |
|---|---|---|---|
| Dockerfile | 16 | `COPY . .` (whole-context copy; still valid after the move) | Task 22 |
| .github/workflows/build.yml | 57 | `version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml \| head -n1)-dev"` | Task 21 |
| .github/workflows/cache-warm.yml | 17 | `- "Cargo.toml"` | Task 22 |
| .github/workflows/ci.yml | 46 | `- 'src/**'` | Task 22 |
| .github/workflows/ci.yml | 47 | `- 'tests/**'` | Task 22 |
| .github/workflows/ci.yml | 49 | `- 'Cargo.toml'` | Task 22 |
| .github/workflows/release-smoke.yml | 5 | `# TCP and UDP bytes through them (tests/release_smoke.rs). Each runner uploads` | Task 22 |
| .github/workflows/release-smoke.yml | 13 | `# proof is in tests/udp_tunnel.rs; this workflow records the path kinds the` | Task 22 |
| .github/workflows/release-smoke.yml | 62 | `# The harness (tests/release_smoke.rs) comes from the dispatched ref,` | Task 22 |
| .github/workflows/release.yml | 68 | `# which is what the updater pins (src/update.rs, #95).` | Task 20, Task 22 |
| .github/workflows/snapshot.yml | 23 | `- "Cargo.toml"` | Task 22 |
| .github/workflows/windows-service-e2e.yml | 12 | `- "src/service/**"` | Task 20, Task 22 |
| .github/workflows/windows-service-e2e.yml | 13 | `- "src/app.rs"` | Task 20, Task 22 |
| .github/workflows/windows-service-e2e.yml | 14 | `- "src/paths.rs"` | Task 20, Task 22 |
| .github/workflows/windows-service-e2e.yml | 15 | `- "src/ownership.rs"` | Task 20, Task 22 |
| .github/workflows/windows-service-e2e.yml | 16 | `- "src/status.rs"` | Task 20, Task 22 |
| .github/workflows/windows-service-e2e.yml | 17 | `- "tests/service_user_windows.rs"` | Task 22 |
| .github/workflows/windows-update-e2e.yml | 11 | `- "src/update.rs"` | Task 20, Task 22 |
| .github/workflows/windows-update-e2e.yml | 12 | `- "tests/update_zip_windows.rs"` | Task 22 |
| packaging/verify-dist.sh | 30 | `# forwarding for the same executables is also covered by tests/role_binaries.rs.` | Task 22 |
| packaging/verify-release-set.sh | 11 | `# tests/release_artifacts.rs.` | Task 22 |
| build.rs | 60 | `let cargo_version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(\|_\| "0.0.0".to_string());` | Task 21 |
| build.rs | 61 | `let app_version = tag_at_head().unwrap_or_else(\|\| format!("{cargo_version}-dev"));` | Task 21 |
| README.md | 10 | `[![rust](https://img.shields.io/badge/rust-1.91%2B-dea584?logo=rust)](Cargo.toml)` | Task 22 |
| README.md | 159 | `Requires Rust 1.91+ (matches 'rust-version' in 'Cargo.toml').` | Task 22 |
| README.md | 921 | `\| 'src/cli.rs'               \| clap CLI surface ('<role> <command>')                  \|` | Task 22 |
| README.md | 922 | `\| 'src/serve.rs'             \| serve role: publish local services into Iroh           \|` | Task 22 |
| README.md | 923 | `\| 'src/access.rs'            \| access role: dial a remote node_id to a local port     \|` | Task 22 |
| README.md | 924 | `\| 'src/role_run.rs'          \| shared run skeleton: dial with retry/backoff, watchers \|` | Task 20, Task 22 |
| README.md | 925 | `\| 'src/pipe.rs'              \| byte-copy pipes between Iroh streams and local sockets \|` | Task 22 |
| README.md | 926 | `\| 'src/endpoint.rs'          \| Iroh endpoint construction + transport tuning          \|` | Task 22 |
| README.md | 927 | `\| 'src/proto.rs'             \| ALPN protocol constants ('iroh-tunnel/{name}')         \|` | Task 22 |
| README.md | 928 | `\| 'src/udp.rs'               \| UDP framing and flow tables (QUIC DATAGRAM forwarding) \|` | Task 22 |
| README.md | 929 | `\| 'src/config.rs', 'src/config_cmd.rs' \| config model + 'config' subcommands           \|` | Task 22 |
| README.md | 930 | `\| 'src/service/'             \| service backends: 'systemd', 'launchd', BusyBox init   \|` | Task 20, Task 22 |
| README.md | 931 | `\| 'src/conn_path.rs'         \| peer connection-path reports for status files          \|` | Task 22 |
| README.md | 932 | `\| 'src/status.rs'            \| atomic per-role status-file writers ('StatusWriter')   \|` | Task 20, Task 22 |
| README.md | 933 | `\| 'src/status_cmd.rs'        \| '<role> status': read the file, render the table       \|` | Task 22 |
| README.md | 934 | `\| 'tests/'                   \| integration tests (network suite, '--ignored')         \|` | Task 22 |
| CONTRIBUTING.md | 25 | `'Cargo.toml') and the exact version CI builds with. New dependencies (or` | Task 22 |
| CONTRIBUTING.md | 38 | `The integration tests in 'tests/serve_access_tunnel.rs' are marked` | Task 22 |
| CONTRIBUTING.md | 68 | `\| 'src/cli.rs'               \| clap CLI surface ('<role> <command>')                  \|` | Task 22 |
| CONTRIBUTING.md | 69 | `\| 'src/serve.rs'             \| serve role: publish local services into Iroh           \|` | Task 22 |
| CONTRIBUTING.md | 70 | `\| 'src/access.rs'            \| access role: dial a remote node_id to a local port     \|` | Task 22 |
| CONTRIBUTING.md | 71 | `\| 'src/role_run.rs'          \| shared run skeleton: retry/backoff dial, disconnect watcher \|` | Task 20, Task 22 |
| CONTRIBUTING.md | 72 | `\| 'src/pipe.rs'              \| byte-copy pipes between Iroh streams and local sockets \|` | Task 22 |
| CONTRIBUTING.md | 73 | `\| 'src/endpoint.rs'          \| Iroh endpoint construction + transport tuning          \|` | Task 22 |
| CONTRIBUTING.md | 74 | `\| 'src/proto.rs'             \| ALPN protocol constants ('iroh-tunnel/{name}')         \|` | Task 22 |
| CONTRIBUTING.md | 75 | `\| 'src/config.rs', 'src/config_cmd.rs' \| config model + 'config' subcommands           \|` | Task 22 |
| CONTRIBUTING.md | 76 | `\| 'src/service/'             \| service backends: 'systemd', 'launchd', BusyBox/SysV init \|` | Task 20, Task 22 |
| CONTRIBUTING.md | 77 | `\| 'src/conn_path.rs'          \| peer connection-path reports for status files           \|` | Task 22 |
| CONTRIBUTING.md | 78 | `\| 'src/status.rs'            \| atomic 'serve-status.json' writer                      \|` | Task 20, Task 22 |
| CONTRIBUTING.md | 79 | `\| 'tests/'                   \| integration tests (network suite, '--ignored')         \|` | Task 22 |
| docs/self-hosted-relay.md | 12 | `and 'iroh-relay/src/main.rs' — cross-checked against release **v1.1.0** (re-verified` | none (upstream iroh-relay path, not this repo) |
| docs/self-hosted-relay.md | 166 | `('DEFAULT_CERT_RELOAD_INTERVAL', verified in 'iroh-relay/src/server/resolver.rs')` | none (upstream iroh-relay path, not this repo) |
| docs/agents/rust-coding-standards.md | 15 | `- **RS-06 — Preserve exit classification:** Preserve 'CliError' through context and conversion so 'src/main.rs' can downcast it: c` | Task 22 |
| docs/agents/rust-coding-standards.md | 33 | `- **RS-15 — Process and network coverage:** For CLI changes affecting arguments, exit codes, output, signals or lifecycle, exercis` | Task 22 |
| docs/agents/rust-coding-standards.md | 46 | `These rules select and adapt the stack-neutral coding-standards skill and Apollo GraphQL's MIT-licensed [rust-best-practices skill` | Task 22 |
| tests/quick_run.rs | 825 | `let readme = include_str!("../README.md");` | none (README.md stays at root) |
