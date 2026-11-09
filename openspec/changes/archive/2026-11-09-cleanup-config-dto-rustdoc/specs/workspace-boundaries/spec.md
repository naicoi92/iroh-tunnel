## ADDED Requirements

### Requirement: Configuration documents carry no filesystem behaviour
`ServeConfig`, `AccessConfig` and the crate-private `RoleDoc` SHALL hold only schema, validation and node-section access. Reading, writing and key persistence MUST be crate-private host `config` store operations, with unchanged diagnostics, locking and key adoption.
Source: refactor design Decision 3; this change's design.md D1–D3

#### Scenario: Public DTO API has no file methods
- **WHEN** the public API of `iroh-tunnel-host` is built
- **THEN** neither `ServeConfig` nor `AccessConfig` exposes a `load`, `save`, `resolve_and_save_key` or other method that takes a filesystem path

#### Scenario: No file methods remain on the documents internally
- **WHEN** the non-test sources of `iroh-tunnel-host` are inspected
- **THEN** neither the DTOs nor `RoleDoc` define a method that reads or writes a file, the replacement load, parse and key-persistence functions are `pub(crate)` store functions, and no whole-document save function exists

#### Scenario: Run still persists a generated key
- **WHEN** `run` starts a role whose config has an empty `secret_key`, and the file still has no key when the writer lock is taken
- **THEN** a key is generated and written into `[node].secret_key` under the writer lock, legacy `multiplex` fields are removed, and the started NodeId matches the persisted key, as before

#### Scenario: Run adopts a concurrently persisted key
- **WHEN** `run` loaded a config with an empty `secret_key`, but another process persisted a valid key before the writer lock was taken
- **THEN** the role adopts that key without rewriting the file, and the started NodeId matches the adopted key, as before

#### Scenario: Load diagnostics are unchanged
- **WHEN** a role config is missing or contains a TOML syntax error on a line that holds a secret
- **THEN** the error reads `failed to read config: <path>` or `failed to parse config: <path> (line N): <message>` as before, and it does not contain the secret

#### Scenario: Legacy fields still ignored and dropped on serialize
- **WHEN** a config carrying any legacy `multiplex` value is parsed and serialized again
- **THEN** the parsed document equals the one without the field, and the serialized text has no `multiplex` key

### Requirement: Public rustdoc links resolve
`cargo doc --workspace --no-deps --locked` on Rust 1.91 SHALL complete with no warnings. `iroh-tunnel-host` and `iroh-tunnel-engine` MUST deny `rustdoc::broken_intra_doc_links` and `rustdoc::private_intra_doc_links`, so public documentation never links to an unresolved or crate-private item.
Source: issue #167; this change's design.md D5

#### Scenario: Clean doc build
- **WHEN** `RUSTDOCFLAGS="-D warnings" cargo +1.91 doc --workspace --no-deps --locked` runs on Linux
- **THEN** it succeeds with no warnings

#### Scenario: Broken link is rejected
- **WHEN** a doc comment on an item compiled for the current target in `iroh-tunnel-host` or `iroh-tunnel-engine` links to an item that does not resolve or is crate-private while the documented item is public
- **THEN** `cargo doc` for that crate fails

### Requirement: Drop counter docs expose the unusable-datagram deviation
The rustdoc of the `limit_shrink` drop counter, in both the engine observation and the host status schema, SHALL state that until #152 is resolved a connection whose datagrams are never usable also counts every datagram as `limit_shrink`, consistent with `docs/udp-integration-contract.md`.
Source: issue #167, #152

#### Scenario: Deviation visible in API docs
- **WHEN** the rustdoc of `limit_shrink` in `iroh-tunnel-engine` and `iroh-tunnel-host` is read
- **THEN** both mention the never-usable-datagrams case and reference #152

## MODIFIED Requirements

### Requirement: Validation gates cover every member
The existing gates SHALL run workspace-wide on Rust 1.91: `fmt --all`, `clippy --workspace --all-targets --all-features --locked -D warnings`, `test --workspace --all-features --locked` and `doc --workspace --no-deps --locked` with rustdoc warnings denied; the doc gate covers items compiled for the target it runs on. Default-feature and release builds without test features MUST also succeed.
Source: design.md Decision 9; this change's design.md D5

#### Scenario: Gates fail on a member-only defect
- **WHEN** a clippy warning or failing test exists only in `iroh-tunnel-engine`
- **THEN** the workspace-wide gate commands fail

#### Scenario: Doc gate fails on a member-only broken link
- **WHEN** a broken intra-doc link exists only in `iroh-tunnel-host`, on an item compiled for the target the gate runs on
- **THEN** the workspace-wide `doc` gate fails
