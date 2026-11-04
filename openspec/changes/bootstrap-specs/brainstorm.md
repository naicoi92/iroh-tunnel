## Design Summary

Bootstrap an evidence-linked specification baseline for the existing iroh-tunnel codebase at commit `a45312e`. The audience is maintainers reviewing what the product already does; this change introduces no new runtime behavior. Drafts remain under this change until reviewed and completed through `/opsx-ff bootstrap-specs`.

No product code changes; tasks verify each spec against the code and its evidence.

The initial read-only map covered the Cargo manifest, README, entry points, `src/`, `src/service/`, `tests/`, relevant `docs/`, `examples/`, `packaging/`, `.github/workflows/`, installers, Docker files and `build.rs`. It returned 12 candidates, with no additional mapped area omitted. `openspec/specs/` was empty, so no candidate was removed for matching an existing spec.

## Alternatives Considered

### Alternative A: Keep all 12 mapped capabilities

- **Approach**: Give platform-specific config and state paths their own `platform-paths` spec alongside `tunnel-configuration`.
- **Pros**: Makes path selection independently discoverable.
- **Cons**: Splits closely related configuration behavior and overlaps the same tests.
- **Why not chosen**: The user chose to merge this area into configuration.

### Alternative B: Merge platform paths into tunnel configuration

- **Approach**: Keep 11 capability specs, including platform-specific paths in `tunnel-configuration`.
- **Pros**: Keeps configuration loading, defaults and path resolution together while retaining separate operational capabilities.
- **Cons**: The configuration spec covers more than TOML field validation.
- **Why chosen**: The user confirmed this boundary in the Plannotator Inbox.

## Agreed Approach

Use Alternative B. Draft the following 11 capabilities from existing source, tests and documentation. Each requirement states observable behavior, includes evidence, and has at least one WHEN/THEN scenario. Requirements backed only by source are marked `Confidence: inferred`.

| Capability | Purpose | Primary coverage |
|---|---|---|
| `cli-entrypoints` | Executable surfaces, quick mode and command dispatch | `src/cli.rs`, `src/app.rs`, entry points; role-binary and quick-run tests |
| `tcp-tunnel-forwarding` | TCP forwarding, multiplexing, retries and channel isolation | `src/serve.rs`, `src/access.rs`, `src/pipe.rs`, `src/proto.rs`, `src/role_run.rs`; tunnel and multiplex tests |
| `udp-datagram-forwarding` | Experimental UDP forwarding and flow limits | `src/udp.rs`, `docs/udp-integration-contract.md`, `tests/udp_tunnel.rs` |
| `iroh-endpoint-relay` | Identity, endpoint transport and custom relay operation | `src/endpoint.rs`, relay documentation and installer, `tests/relay_auth.rs` |
| `tunnel-configuration` | TOML loading, validation and platform-specific paths | `src/config.rs`, `src/paths.rs`, `examples/`, configuration path tests |
| `config-management-commands` | Configuration commands and safe persistence | `src/config_cmd.rs`, `src/config_store.rs`, configuration lifecycle tests |
| `connection-path-status` | Status snapshots, status commands and path diagnostics | `src/status.rs`, `src/status_cmd.rs`, `src/conn_path.rs`, status and logging tests |
| `role-ownership-shutdown` | Role ownership, conflict prevention and shutdown | `src/ownership.rs`, `src/shutdown.rs`, `tests/role_ownership.rs` |
| `system-service-management` | User/system service installation and lifecycle across platforms | `src/service/`, service tests |
| `self-update` | Release selection, verification and executable ownership | `src/update.rs`, `src/update_owner.rs`, update tests |
| `install-release-packaging` | Installers, distribution packages, containers and release checks | Root installers and Docker files, `packaging/`, `build.rs`, `.github/workflows/`, installer/release tests |

## Key Decisions

- The user confirmed 11 capabilities by merging `platform-paths` into `tunnel-configuration`; no other candidate was dropped or renamed.
- Generated prose is English, as configured by `language: en` in `openspec/config.yaml`.
- Ignore `openspec/` when mapping product behavior, `.git/`, `node_modules/`, `dist/`, `build/`, `vendor/`, `target/` and generated files.
- Write delta specs only under `openspec/changes/bootstrap-specs/specs/`; do not modify `openspec/specs/`.
- These are reviewable drafts, not proof of runtime correctness or approval of every implementation detail as a permanent contract.

## Open Questions

The capability boundary is confirmed. Review must resolve these observed disagreements before accepting an enduring baseline:

- `cli-entrypoints`: `src/cli.rs:362-384` makes one `-v` informational and two debug, whereas `README.md:701,720-721` describes `-v` as debug. The draft records source behavior and marks it inferred rather than treating the README as supporting evidence.
- `udp-datagram-forwarding`: `docs/udp-integration-contract.md:25-26` explicitly says setup failure for unsupported datagrams is not implemented as specified. The draft records the documented current drop-and-count behavior, not a claim that the intended setup check exists.

Review must also decide which inferred or implementation-specific behavior belongs in the enduring specification. No product implementation, new tests, tasks or plan are produced by this drafting step.
