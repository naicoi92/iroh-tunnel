## MODIFIED Requirements

### Requirement: Consistent short options across roles
The system SHALL use the same short letter for the same option in every role form and every command, with no duplicate short option within one command:
- `-c` config
- `-n` name
- `-t` target
- `-l` listen
- `-N` node-id
- `-p` protocol
- `-r` relay-url
- `-k` key-file
- `-y` yes

`--reveal`, `--print`, `--system`, `--user`, `--relay-token`, `--relay-token-file` and `--clear-relays` SHALL have no short form.
Evidence: src/cli.rs:386-417 (unit test), tests/role_binaries.rs:463-554

#### Scenario: Short letters do not collide
- **WHEN** the command definitions of `it`, `its`, `ita`, `serve`, `access` and all their subcommands are built
- **THEN** no short option maps to two long options, and each letter above maps to its long option wherever it appears

#### Scenario: Long-only options
- **WHEN** the same definitions are inspected
- **THEN** `reveal`, `print`, `system`, `user`, `relay-token`, `relay-token-file` and `clear-relays` have no short letter

### Requirement: Config actions dispatch
The system SHALL expose these `config` subcommands, each acting on the role's config file or the one given by `-c/--config`:
- `config keygen`, with `-c` and `-y`; `keygen --print` prints a key without saving and conflicts with both;
- `set -n NAME`;
- `node`, editing the role's `[node]` relay settings (see `config-management-commands`);
- `rm -n NAME`;
- `list`;
- `show [--reveal]`;
- `edit`;
- `path`;
- `check`.
Evidence: src/cli.rs:197-278, src/app.rs:253-271, tests/quick_run.rs:786-808

#### Scenario: Config path
- **WHEN** `config path` is run
- **THEN** the role's config path is printed

#### Scenario: Keygen random conflicts
- **WHEN** `config keygen --print` is combined with `--config` or `--yes`
- **THEN** parsing rejects the combination as a usage error with the `config keygen` help

#### Scenario: Removed syntax
- **WHEN** `config add`, `config remove` or `config keygen random` is run
- **THEN** it is a usage error (exit code 2) and stderr contains the `config` (or `config keygen`) help listing the current commands

#### Scenario: Node command on every executable
- **WHEN** `it serve config node --help`, `it access config node --help`, `its config node --help` or `ita config node --help` is run
- **THEN** it exits successfully and lists `-r/--relay-url`, `--relay-token`, `--relay-token-file` and `--clear-relays`

### Requirement: Usage errors print the failing command's help
The system SHALL, on a usage error (any argument-parsing failure, or a missing creation-time attribute of `config set`), print to stderr `error: <description>`, a blank line and the full help of the deepest matched subcommand, then exit 2, on all three executables. Every other failure (schema rules on values, `--key-file`, `--relay-token-file`, config-file, I/O, endpoint, service, conflict) SHALL print only `error: <message>`, without help. The scenario "Relay flag not accepted by config set" now covers serve only, because access `config set` accepts `-r`.
Source: brainstorm.md Errors and help; design.md D1

#### Scenario: Unknown option on a leaf command
- **WHEN** `it serve config set -n web --bogus` is run
- **THEN** the exit code is 2, stderr starts with `error:` naming `--bogus`, and stderr contains the `config set` usage and its options list

#### Scenario: Unknown subcommand
- **WHEN** `it serve config bogus` is run
- **THEN** the exit code is 2 and stderr contains the error followed by the `config` help listing its subcommands

#### Scenario: Missing quick-mode flag
- **WHEN** `its -n web` is run without `-t`
- **THEN** the exit code is 2, the error mentions `--target`, and stderr contains the `iroh-tunnel-serve` help

#### Scenario: Invalid value
- **WHEN** `it serve config set -n web -t 3000 -p sctp` is run
- **THEN** the exit code is 2, stderr names the possible values `tcp` and `udp`, and contains the `config set` help

#### Scenario: Schema rule failure has no help
- **WHEN** `it serve config set -n BAD -t 3000` or `it serve -n BAD -t 3000` is run
- **THEN** the exit code is 2, stderr names the name rule, and stderr does not contain `Usage:`

#### Scenario: Wrong-role flag on config set
- **WHEN** `it serve config set -n web -l 8080` or `it access config set -n web -t 3000 -N <id>` is run
- **THEN** the exit code is 2 and stderr contains the error followed by that role's `config set` help

#### Scenario: Relay flag not accepted by config set
- **WHEN** `it serve config set -n web -t 3000 -r https://relay.example` or `its config set -n web -t 3000 -r https://relay.example` is run
- **THEN** the exit code is 2 and stderr contains the error naming `-r` followed by the serve `config set` help

#### Scenario: Conflicting relay flags
- **WHEN** `it serve config node` is run with any of `-r https://a.example --clear-relays`, `--relay-token t --clear-relays`, `--relay-token-file f --clear-relays` or `--relay-token t --relay-token-file f`
- **THEN** the exit code is 2, stderr contains the conflict error followed by the `config node` help, and the config file is unchanged

#### Scenario: Conflicting override flags on access config set
- **WHEN** `it access config set -n web -r https://a.example --clear-relays` is run
- **THEN** the exit code is 2, stderr contains the conflict error followed by the access `config set` help, and the config file is unchanged

#### Scenario: Node command without an edit flag
- **WHEN** `it serve config node` or `it serve config node -c c.toml` is run
- **THEN** the exit code is 2, stderr contains the `config node` help, and no file is created or changed

#### Scenario: Runtime error has no help
- **WHEN** `it serve config list` is run against a config file with a TOML syntax error
- **THEN** the exit code is 2, stderr contains `error:` and the line number, and stderr does not contain `Usage:`

#### Scenario: Help goes to stderr
- **WHEN** any usage error occurs
- **THEN** stdout is empty
