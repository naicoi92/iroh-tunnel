# Contributing to iroh-tunnel

Thanks for contributing! Bug reports, fixes, and features are welcome.

- [Bugs, questions, and security](#bugs-questions-and-security)
- [Development setup](#development-setup)
- [Code quality gates](#code-quality-gates)
- [Project layout](#project-layout)
- [Pull requests](#pull-requests)
- [Compatibility rules](#compatibility-rules)
- [Licensing](#licensing)

## Bugs, questions, and security

- **Bugs and feature requests:** open a
  [GitHub issue](https://github.com/naicoi92/iroh-tunnel/issues). For bugs,
  include the output of `iroh-tunnel --version`, your OS, and logs captured
  with `-vv` (or `RUST_LOG=iroh_tunnel=debug`).
- **Security vulnerabilities:** do **not** open a public issue — follow
  [SECURITY.md](SECURITY.md).

## Development setup

You need **Rust 1.91 or newer** — this is the crate's MSRV (`rust-version` in
`Cargo.toml`) and the exact version CI builds with. New dependencies (or
version bumps) must keep building on 1.91; when a bump would raise the MSRV,
pin instead (for example, `vergen-gitcl` is held at 9.x because 10.x requires
rustc 1.95).

```sh
git clone https://github.com/naicoi92/iroh-tunnel && cd iroh-tunnel
cargo build
cargo test --all-features
```

### Network integration tests

The integration tests in `tests/serve_access_tunnel.rs` are marked
`#[ignore]` because they dial the real n0 relay network over the Internet.
`cargo test` alone runs the offline unit tests. Run the online suite
explicitly when touching serve/access behavior:

```sh
cargo test --all-features -- --ignored         # one test takes ~70 s
```

## Code quality gates

CI runs exactly these three commands on every pull request, in a `lint` and a
`test` job. Run them locally before pushing:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

`clippy -D warnings` means zero warnings — fix or suppress with a comment
explaining why.

## Project layout

| Path                       | Purpose                                                |
|----------------------------|--------------------------------------------------------|
| `src/cli.rs`               | clap CLI surface (`<role> <command>`)                  |
| `src/serve.rs`             | serve role: publish local services into Iroh           |
| `src/access.rs`            | access role: dial a remote node_id to a local port     |
| `src/role_run.rs`          | shared run skeleton: retry/backoff dial, disconnect watcher |
| `src/pipe.rs`              | byte-copy pipes between Iroh streams and local sockets |
| `src/endpoint.rs`          | Iroh endpoint construction + transport tuning          |
| `src/proto.rs`             | ALPN protocol constants (`iroh-tunnel/{name}`)         |
| `src/config.rs`, `src/config_cmd.rs` | config model + `config` subcommands           |
| `src/service/`             | service backends: `systemd`, `launchd`, BusyBox/SysV init |
| `src/status.rs`            | atomic `status.json` writer                            |
| `tests/`                   | integration tests (network suite, `--ignored`)         |
| `.goreleaser.yaml`         | Linux release pipeline (binaries, Docker, .deb/.apk)   |
| `.goreleaser.macos.yaml`   | macOS release pipeline (darwin binary, Homebrew cask)   |
| `packaging/`               | systemd unit + deb postinstall used by nFPM            |
| `examples/`                | sample `serve.toml` / `access.toml`                    |

## Pull requests

1. Branch from `main`.
2. Title in [Conventional Commits](https://www.conventionalcommits.org) style —
   `feat(scope): …`, `fix(scope): …`, `refactor(scope): …`, `chore: …` —
   where the scope is usually the module or area (`config`, `service`, `ci`,
   `release`, `role-run`, …).
3. CI (`lint` + `test`) must pass. Heavier artifact builds (GoReleaser
   snapshots, Docker, packages) run on `main` pushes only; you do not need to
   produce release artifacts in a PR.
4. User-visible changes get a line under `[Unreleased]` in
   [CHANGELOG.md](CHANGELOG.md).

## Compatibility rules

- There is deliberately **no protocol negotiation**: the ALPN is
  `iroh-tunnel/{service}` on every version. Any change to connection or
  stream handling must preserve the **serve-first rollout property** — a new
  serve stays compatible with every access version. See the 0.2.0 rollout
  contract in [CHANGELOG.md](CHANGELOG.md) for the compatibility matrix this
  implies.
- Transport settings stay at iroh/noq defaults unless a measured workload
  justifies an override; expose overrides as config, never hard-code them.

## Licensing

By contributing, you agree your contributions are dual-licensed under
[MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at the option of the
user — the same terms as the project. There is no CLA and no DCO sign-off
requirement.
