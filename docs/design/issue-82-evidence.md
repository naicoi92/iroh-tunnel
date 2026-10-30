# Issue #82 evidence — independently packaged role binaries

Scope: three executables (`iroh-tunnel`/`it`, `iroh-tunnel-serve`/`its`,
`iroh-tunnel-access`/`ita`), consistent long/short options, `-k/--key-file`
persistent identity for quick runs, help with aliases and defaults. Individual
slice evidence; not final-artifact acceptance (#97).

## Implemented option letters (recorded, not new policy)

`-n/--name`, `-t/--target` (Serve), `-l/--listen` and `-N/--node-id` (Access),
`-p/--protocol` (default `tcp`), `-k/--key-file`, `--relay-url`; global `-v`,
`-q`, `-V`, `-h`. `-k` reads a base64 key as written by `config keygen`;
diagnostics name the path, never the contents, and nothing is written.

## Acceptance → test (`tests/role_binaries.rs`, process level, local relay)

| Case | Test |
|---|---|
| CLI-INDEPENDENT-01 | each executable is copied alone into its own `bin` dir with `PATH` limited to it; `all_in_one_alias_runs_both_roles_alone`, `standalone_full_names_forward_with_long_options` forward real bytes |
| CLI-ALIAS-01 | aliases are symlinks (copies on Windows) to the installed binary; `its`/`ita`/`it serve`/`it access` forward bytes in the alias tests |
| CLI-SHORT-01 | `standalone_aliases_forward_with_short_options`, `all_in_one_and_standalone_roles_interoperate_with_short_options`; unit test `short_options_are_consistent_and_conflict_free` |
| IDENTITY-KEY-01 | `supplied_key_keeps_public_identity_across_restarts_for_every_role_form` (short and long forms, four role forms, key file unchanged, no config written) |
| CLI-HELP-01 | `help_lists_role_options_aliases_and_defaults`, `role_specific_options_are_rejected_by_the_other_role`, `unusable_key_files_fail_without_revealing_contents_or_writing_config` |

Red: before the change `cargo test --test role_binaries` failed to compile
(`CARGO_BIN_EXE_iroh-tunnel-serve` undefined). Green: all 8 pass;
`cargo fmt --check`, `cargo clippy --all-targets --all-features --locked -D warnings`
and `cargo test --all-features --locked` pass on macOS (Darwin 24.6).

## Limits

macOS only; Linux/Windows runtime, packaged artifacts, installers and alias
creation by installers (#93–#96) are not covered. The `test-utils` binaries
disable relay TLS verification: local fixtures only. Exact diagnostics and exit
codes beyond clap's usage error (2) and `CliError::Config` (2) are not asserted.

## Known gap (code review)

`its service install` / `ita service install` reuse `service::resolve_binary`
and the `<role> run --config` argv, which only the all-in-one binary accepts.
Standalone service units are therefore not yet correct; this slice's tests
cover foreground forwarding only. Also open: key-file permission warning,
README updates and release packaging (#94).
