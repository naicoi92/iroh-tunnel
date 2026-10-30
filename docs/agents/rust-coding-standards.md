# Rust coding standards

These rules apply to new and changed Rust code and its reviews; they do not require unrelated cleanup. Standards reviews should cite the relevant rule IDs in findings, including the concrete behavior or risk. Explain justified exceptions in the review and, where the constraint is not evident, in code. Existing compatibility contracts and security requirements take precedence over generic advice.

## Design and APIs

- **RS-01 — Modules and names:** Keep related behavior behind cohesive modules; expose only the visibility callers need. Use descriptive Rust names, small focused functions and early returns where they clarify flow. Prefer a little duplication to an abstraction that couples unrelated behavior; do not add speculative generality.
- **RS-02 — Public contracts:** Document public APIs with rustdoc: purpose, invariants, relevant errors, panics and safety requirements. Use executable examples when useful. Comments explain non-obvious reasons or constraints, not syntax; update stale comments with the code.
- **RS-03 — Types and dispatch:** Represent meaningful alternatives and invariants with enums or types rather than loosely coupled flags. Use generics or trait objects when their trade-offs serve actual callers; use typestate only when its enforcement outweighs complexity. Avoid introducing abstractions solely for imagined future uses.

## Ownership and errors

- **RS-04 — Borrow deliberately:** Borrow when ownership is unnecessary (for example `&str`, `&[T]`, `&Path`); move values when the callee needs ownership. Clone only when an independent owned value or task lifetime requires it. Use `mut` only for mutation and choose `Arc`/locks only for genuine shared ownership or synchronization. Do not impose a fixed size threshold for `Copy`.
- **RS-05 — Propagate actionable errors:** Return `Result` for expected failures, propagate with `?`, and add context identifying the failed operation without exposing secrets. Do not silently discard errors. Avoid production `unwrap`/`expect` and panics for recoverable input or I/O; if an invariant makes a panic necessary, explain it. Tests may use them for setup and assertions.
- **RS-06 — Preserve exit classification:** Preserve `CliError` through context and conversion so `src/main.rs` can downcast it: config 2, permission 3, Iroh 4, service 5, start refused because the role or identity already runs 6; unclassified errors 1 and success 0. Test classification at the CLI boundary when changing it. `anyhow` already serves both library and binary code here; use typed errors where callers need to distinguish cases, not a blanket ban on existing `anyhow`.

## Async and resource lifecycle

- **RS-07 — Do not block Tokio workers:** Use async I/O for potentially slow operations; move blocking filesystem, process or CPU-heavy work to an appropriate blocking thread when it can stall the runtime. Do not nest `block_on` inside async execution. Avoid holding a blocking lock across `.await`.
- **RS-08 — Cancellation and tasks:** Define what happens if a future is dropped or a `select!` branch loses: preserve invariants, account for partial I/O, and clean up owned resources. Give spawned tasks an owner; observe their result or explicitly justify detachment, and arrange cancellation and joining during shutdown. Do not assume dropping a `JoinHandle` stops its task or that started blocking work can be aborted.
- **RS-09 — Bound work and shutdown:** Bound queues, concurrent streams/tasks and buffers according to workload; apply backpressure rather than unlimited spawning. Choose timeouts, retry/backoff and cancellation appropriate to external operations and long-lived tunnels, without arbitrarily timing out healthy streams. Shutdown should stop accepting work and close or drain active resources according to the approved lifecycle. Unresolved limits, defaults and behavior remain subject to the relevant decision gate.

## Security, filesystem and platforms

- **RS-10 — Secret and log hygiene:** Never log secret keys, relay tokens, credentials or full sensitive configuration, including via `Debug` or error context. Log useful non-secret operation context; redact sensitive values in diagnostics and tests. Treat user-provided paths and identifiers as potentially sensitive.
- **RS-11 — Paths and filesystem:** Use `Path`/`PathBuf`, not string concatenation or assumed UTF-8, for filesystem paths. Validate untrusted path components against the intended destination; account for absolute paths, traversal, symlinks and races rather than relying on lexical checks alone. Use appropriate permissions for secrets and prevent partial writes from corrupting state using the approved persistence contract; handle platform-specific behavior and errors explicitly. Do not choose unresolved migration synchronization or updater replacement policy without its decision gate.
- **RS-12 — Unsafe:** Prefer safe APIs. Introduce `unsafe` only for a justified need, keep its scope minimal, and put a `// SAFETY:` explanation at each block stating the invariants and why they hold. Document caller obligations for unsafe APIs and test the safe boundary where feasible.
- **RS-13 — Platform and TLS boundaries:** Respect `cfg`-gated platform support and report unsupported operations rather than assuming Unix behavior on Windows. CI covers Linux and Windows, not all platform-specific runtime behavior; test affected platforms when available. The `test-utils` feature disables relay certificate verification for self-signed test relays; it must not be enabled in production builds. `--all-features` is for validation, not a production build recipe.

## Testing and validation

- **RS-14 — Test from specification:** Derive cases from requirements and observable contracts, including success, invalid input, boundaries, failure and cancellation as applicable. Use descriptive behavior names and focused tests; multiple assertions are appropriate when they establish one behavior. Unit tests cover local logic, integration tests cover public boundaries, and rustdoc examples can test public usage. Keep tests deterministic and isolate temporary files and resources.
- **RS-15 — Process and network coverage:** For CLI changes affecting arguments, exit codes, output, signals or lifecycle, exercise the actual binary in process-level end-to-end tests as appropriate, not only in-process functions. Cover cleanup and failure paths. Prefer isolated local-relay fixtures for real-process E2E. The ignored tests in `tests/serve_access_tunnel.rs` use public relays; run them only with certificate verification actually enabled. The current test-feature binary disables relay TLS verification, so do not run that binary against public relays; report coverage as blocked until a verified-TLS harness is available.
- **RS-16 — Toolchain and gates:** Preserve MSRV Rust 1.91, including dependencies and APIs; do not use newer language or standard-library features without an approved MSRV change. Run the existing CI gates shown below. For reproducible dependency resolution, also use `--locked` for Clippy and tests; this is local validation, not a claim that CI currently passes `--locked`. Explain any lint suppression. No new lint gate is implied.

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-features --locked
```

CI currently runs the same fmt, Clippy and test checks without `--locked`, on Rust 1.91; see [CI workflow](../../.github/workflows/ci.yml) and [contributing guide](../../CONTRIBUTING.md). Run on 1.91 when verifying MSRV; a newer compiler alone cannot establish it.

## Sources and project adaptations

These rules select and adapt the stack-neutral coding-standards skill and Apollo GraphQL's MIT-licensed [rust-best-practices skill v1.1.2, pinned source](https://github.com/apollographql/skills/tree/ff4534868e089580c2e09563c20f86a410179628/skills/rust-best-practices), including chapters 1–9. Project contracts come from [Cargo.toml](../../Cargo.toml), [CI](../../.github/workflows/ci.yml), [error types](../../src/error.rs), [CLI entry point](../../src/main.rs), and [endpoint construction](../../src/endpoint.rs). Project-specific async, security, filesystem and process-test rules supplement those sources. In particular, this guide does not adopt Apollo's fixed Copy-size suggestion, one-assertion preference, library-wide `anyhow` ban, newer-than-MSRV APIs, or proposed additional lint gates.
