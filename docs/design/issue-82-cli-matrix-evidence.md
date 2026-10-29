# Issue #82 bounded CLI matrix

Base `12bed8dc4f2c37e438acdd03a51fd972b927e6e7`. Live #82 body and comments
refreshed; native blocker #81 is closed. This is not issue or release acceptance.
User authorized actual-process seam and limited scope to help, existing options,
TCP default, and invalid/conflicting options. Keys, aliases and lifecycle remain
out of scope. Matrix recorded before tests in `/tmp/issue-82-matrix.md`.

| Source criterion | Process test | Expected values and observed result |
|---|---|---|
| #82 AC2 / CL2, CLI-HELP-01 | `role_help_documents_existing_options_and_tcp_default` | All four forms expose existing role-specific and common long/short pairs, relay, help and `[default: tcp]`; option literals follow #81 map. Passed. |
| #82 CL3, CLI-HELP-01 | `role_specific_options_are_rejected_with_guidance` | Each form rejects other role's long and short options, names rejected flag and suggests help. Passed. |
| #81 CL1 / #82 CL3 | `quick_options_conflict_with_configured_commands` | Each form rejects all existing long/short quick flags with `config path`, including explicitly supplied default TCP, and gives conflict/help guidance. Passed. |
| #82 CL2, TCP default | Existing `quick_run` real TCP tests | All-in-one and standalone real-process forwarding tests omit protocol and exchange fixed request/response bytes via isolated local relay; separately rerun, 16 tests passed. Does not prove all long/short standalone combinations. |

Initial test run had two passing tests and one failure: clap reported canonical
`--node-id` for `-N`, while test incorrectly demanded original spelling. That was
a test expectation defect, not an implementation red; assertion changed to check
conflict, subcommand and help. Subsequent run passed all three. No product behavior
failure was observed and no CLI fix was needed; no fabricated red/green claim.

Validation on macOS arm64, Rust 1.99.1: `cargo fmt --all -- --check`,
`cargo test --locked --test cli_role_matrix`, `cargo test --locked --test
quick_run -- --nocapture` and `cargo test --all-features --locked` passed. Full
suite: 147 passed, five ignored (four public-relay tests and one interoperability
case). No ignored public-relay tests run; test-utils disables relay verification.
`cargo +1.91.1 test --locked --test cli_role_matrix` passed. `cargo clippy
--all-targets --all-features --locked -- -D warnings` failed on preexisting
`tests/quick_run.rs` `access_ready_with_collision_probe` (eight arguments,
`clippy::too_many_arguments`); confirmed present in base. Left unchanged to avoid
unrelated lifecycle/helper edits. Applicable Rust standards: RS-01, RS-05,
RS-13–16. Independent Standards and Spec reviews still required.

Not established: persistent-key contract, installed aliases, all role/flag
forwarding combinations, verified-TLS production artifact, Linux/Windows runtime,
full MSRV CI gates, or full #82/#97 acceptance. Tests exercise four binary role
forms, not release installation. No public relay used.
