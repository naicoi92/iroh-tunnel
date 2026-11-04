## ADDED Requirements

### Requirement: Config file target resolution
The system SHALL operate every `config` subcommand on the file given by `--config`, and SHALL fall back to the role's platform default config path only when `--config` is absent.
Evidence: src/config_cmd.rs:360-372, src/config_cmd.rs:326-331, tests/config_lifecycle.rs:577-636, tests/config_lifecycle_windows.rs:247

#### Scenario: Explicit path wins
- **WHEN** keygen, add, list, show, path, remove and edit are each run with `--config custom.toml`
- **THEN** all succeed against `custom.toml` and `config path` prints exactly that path
- **AND** the default config file and default directory are not created or touched

#### Scenario: Access role uses the same flag
- **WHEN** `access config add --config access.toml ... --node-id <id>` is run
- **THEN** the entry is written to `access.toml`

### Requirement: Symlinked config files are written through
The system SHALL replace the target of a symlinked config file rather than the symlink itself when mutating it.
Evidence: src/config_store.rs:46-53, src/config_cmd.rs:70, src/config_cmd.rs:316
Confidence: inferred

#### Scenario: Config path is a symlink
- **WHEN** a mutating `config` command is run on a path that is a symlink to a file
- **THEN** the linked file receives the new content and the symlink remains in place

### Requirement: Add a service entry
The system SHALL append a `[[services]]` entry with name, protocol, host and port on `config add`, SHALL create the config file (and its parent directory) if missing, and SHALL print `Added service '<name>' to <path>.`.
Evidence: src/config_cmd.rs:121-155, tests/config_lifecycle.rs:238-278, tests/config_lifecycle.rs:577-636

#### Scenario: Add to an existing config
- **WHEN** `serve config add --config c.toml --name svc --protocol tcp --port 3000` is run on a valid config
- **THEN** the file gains a `[[services]]` entry named `svc`
- **AND** existing entries and comments are preserved

#### Scenario: Add creates a missing config
- **WHEN** `config add` is run against a path that does not exist
- **THEN** the file is created containing the new service entry

### Requirement: Access services require a node ID
The system SHALL fail `access config add` with a config error (exit code 2) when `--node-id` is not supplied, and SHALL record `node_id` on the entry when it is.
Evidence: src/config_cmd.rs:139-146, tests/config_lifecycle.rs:577-636

#### Scenario: Missing node-id for access
- **WHEN** `access config add` is run without `--node-id`
- **THEN** it fails with a message saying `--node-id` is required and the file is unchanged

### Requirement: Duplicate service names are rejected
The system SHALL reject `config add` with a config error (exit code 2) naming the service and file when a service with the same name already exists, leaving the file unchanged.
Evidence: src/config_cmd.rs:124-132, src/config_cmd.rs:441-446
Confidence: inferred

#### Scenario: Add an existing name
- **WHEN** `config add --name web` is run and `web` is already configured
- **THEN** the command fails with exit code 2 and `a service named 'web' already exists`

### Requirement: Remove a service entry
The system SHALL delete the `[[services]]` entry whose name matches on `config remove`, SHALL fail with a config error (exit code 2) when no such entry (or no services table) exists, and SHALL print `Removed service '<name>' from <path>.` on success.
Evidence: src/config_cmd.rs:158-183, tests/config_lifecycle.rs:577-636

#### Scenario: Remove existing
- **WHEN** `config remove --name web` is run and `web` exists
- **THEN** the entry is deleted from the file

#### Scenario: Remove unknown name
- **WHEN** `config remove --name nope` is run
- **THEN** it fails with exit code 2 and `no service named 'nope'`

### Requirement: Remove on a missing config creates nothing
The system SHALL fail `config remove` with exit code 2 when the config file does not exist, and SHALL NOT create the file or its parent directory.
Evidence: src/config_cmd.rs:160-163, tests/config_lifecycle.rs:557-574

#### Scenario: Missing file with missing parent
- **WHEN** `serve config remove --config sub/serve.toml --name x` is run and `sub/` does not exist
- **THEN** the exit code is 2 and `sub/` still does not exist

### Requirement: List services as a table
The system SHALL print a header plus one row per service on `config list`, using columns NAME, PROTO, HOST, PORT for the serve role and NAME, NODE_ID, PROTO, PORT for the access role, and SHALL print `No services configured (<path>).` when there is no services table.
Evidence: src/config_cmd.rs:186-224, tests/config_lifecycle.rs:577-636

#### Scenario: Serve list
- **WHEN** `serve config list` runs on a config with services
- **THEN** a NAME/PROTO/HOST/PORT table is printed

#### Scenario: Access list
- **WHEN** `access config list` runs on a config with services
- **THEN** a NAME/NODE_ID/PROTO/PORT table is printed

#### Scenario: No services
- **WHEN** the config has no `[[services]]`
- **THEN** `No services configured (<path>).` is printed and the exit code is 0

### Requirement: Show redacts secrets by default
The system SHALL print the config file with `[node].secret_key` and `[node].relay_token` values replaced by `<redacted>` on `config show`, preserving comments and other formatting, and SHALL print the file verbatim only when `--reveal` is given. It SHALL never modify the file.
Evidence: src/config_cmd.rs:227-259, tests/config_lifecycle.rs:436-456, tests/config_lifecycle_windows.rs:189

#### Scenario: Default show
- **WHEN** `config show` runs on a config with a secret key and relay token
- **THEN** neither secret appears in stdout, while other values (such as relay URLs), service entries and comments do

#### Scenario: Reveal
- **WHEN** `config show --reveal` runs
- **THEN** the secret key and relay token appear in stdout

### Requirement: Print the resolved config path
The system SHALL print the resolved config file path on `config path`.
Evidence: src/config_cmd.rs:326-331, tests/config_lifecycle.rs:610-613

#### Scenario: Explicit path
- **WHEN** `config path --config /x/custom.toml` runs
- **THEN** stdout is `/x/custom.toml`

### Requirement: Read commands on a missing config share one guidance error
The system SHALL fail `config list`, `config show` and `run` with exit code 2 and one identical message when the config file is missing; the message SHALL name the path and mention `config add` and `config keygen`; no file or directory SHALL be created.
Evidence: src/config_cmd.rs:339-349, tests/config_lifecycle.rs:458-484, tests/config_lifecycle_windows.rs:221

#### Scenario: No config present
- **WHEN** `serve run`, `serve config list` and `serve config show` are run with no config at the default path
- **THEN** each exits with code 2 and prints the same stderr naming the default path and suggesting `config add` / `config keygen`
- **AND** neither the file nor its parent directory exists afterwards

### Requirement: Keygen writes a new node identity
The system SHALL generate a new secret key and store it in `[node].secret_key` on `config keygen` for either role, creating the config file if needed, and SHALL print `Generated new secret_key.`, `New NodeId: <id>` and a warning that configs referencing the node ID must be updated.
Evidence: src/config_cmd.rs:41-66, tests/config_lifecycle.rs:379-433, tests/config_lifecycle.rs:530-538

#### Scenario: First key
- **WHEN** `config keygen` is run on a config without a secret key (or a missing file)
- **THEN** it succeeds without prompting, the file holds a non-empty `secret_key` and has mode 0600 on Unix

#### Scenario: Inline node table
- **WHEN** keygen is run on a config containing `node = { relay_urls = [] }`
- **THEN** a `secret_key` is stored successfully

#### Scenario: Non-table node
- **WHEN** `node` exists but is not a table
- **THEN** keygen fails with a config error (exit 2) saying `[node] must be a table`

### Requirement: Key rotation needs confirmation
The system SHALL refuse to replace an existing non-empty `secret_key` unless `--yes` is given or an interactive terminal answers `y`/`yes`; on refusal the key SHALL remain unchanged and the error SHALL mention `--yes`.
Evidence: src/config_cmd.rs:41-102, tests/config_lifecycle.rs:379-421, tests/config_lifecycle_windows.rs:170

#### Scenario: Non-interactive without --yes
- **WHEN** `config keygen` is run on a config with a key, without a terminal and without `--yes`
- **THEN** it fails mentioning `--yes` and the stored key is unchanged

#### Scenario: Interactive answers
- **WHEN** a terminal user answers `n`
- **THEN** it fails and the key is unchanged
- **WHEN** the user answers `y`
- **THEN** the key is replaced

#### Scenario: Forced rotation
- **WHEN** `config keygen --yes` is run on a config with a key
- **THEN** the key is replaced and stdout contains `New NodeId`

### Requirement: Random keygen touches no file
The system SHALL print `Secret key: <key>` and `Node ID: <id>` to stdout for `config keygen random` without reading or writing any config file or the default path.
Evidence: src/config_cmd.rs:104-113, src/app.rs:256-258, tests/config_paths.rs:402-471

#### Scenario: Random key
- **WHEN** `config keygen random` is run
- **THEN** a fresh secret key and node ID are printed and no config file is created

### Requirement: Mutations are validated and all-or-nothing
The system SHALL validate the resulting document against the role's schema before writing any mutation (`add`, `remove`, `keygen`, `edit`), and on any failure SHALL exit with code 2 and leave the config file byte-for-byte unchanged.
Evidence: src/config_cmd.rs:385-432, tests/config_lifecycle.rs:114-161, tests/config_lifecycle_windows.rs:54

#### Scenario: Invalid existing config
- **WHEN** add, remove, keygen `--yes`, or edit is run against an invalid config file
- **THEN** each exits with code 2 and the file bytes are identical to before

### Requirement: Config edits preserve formatting and drop retired fields
The system SHALL edit config files in place so existing comments and formatting survive, and SHALL remove legacy `multiplex` keys from `[[services]]` entries on every write.
Evidence: src/config_store.rs:216-232, src/config_cmd.rs:402-405, tests/config_lifecycle.rs:238-278, tests/config_multiplex_compat.rs:221-250

#### Scenario: Comment preserved
- **WHEN** `config add` runs on a file containing `# operator comment`
- **THEN** the comment is still present afterwards

#### Scenario: Legacy multiplex
- **WHEN** a write is made to a config where a service has `multiplex = ...`
- **THEN** the written file no longer contains that key

### Requirement: Concurrent writers are serialized
The system SHALL serialize writers with a sibling `<file>.lock`, preserving concurrent updates; lock timeout (10 seconds by default) SHALL fail with `another process is writing` and leave the file unchanged.
Evidence: src/config_store.rs:30-39, src/config_store.rs:84-126, tests/config_lifecycle.rs:237-278, tests/config_lifecycle.rs:340-377, tests/config_lifecycle_windows.rs:96-169

#### Scenario: Eight parallel adds
- **WHEN** eight `config add` processes run concurrently on the same file
- **THEN** all succeed, all eight services plus the original are present, comments survive and the file parses

#### Scenario: Lock held by another process
- **WHEN** another process holds the lock and the timeout elapses
- **THEN** `config add` fails with `another process` in stderr and the file is unchanged

### Requirement: Saves are atomic and owner-only
The system SHALL write config files via a sibling temp file renamed into place so that readers or a killed writer never observe a truncated or partial document, SHALL create config and lock files with mode 0600 on Unix, and SHALL refuse to overwrite a read-only config file.
Evidence: src/config_store.rs:128-175, src/config_store.rs:67-76, tests/config_lifecycle.rs:280-338, tests/config_lifecycle.rs:191-210, tests/config_lifecycle.rs:424-433

#### Scenario: Killed writer
- **WHEN** `config add` processes are killed at random points while a reader polls the file
- **THEN** the reader always sees a complete valid TOML document

#### Scenario: Permissions tightened
- **WHEN** a valid edit or key write is saved to a file that was mode 0644
- **THEN** the file is mode 0600 afterwards (Unix)

#### Scenario: Read-only target
- **WHEN** a mutation targets a read-only config file
- **THEN** it fails with a message that the file is read-only and the file is unchanged

### Requirement: Edit via the user's editor
The system SHALL open a private copy of the config in `$EDITOR` on `config edit`, and on exit SHALL validate the result and replace the live file, printing `Saved <path>.`, or print `No changes.` when the text is unchanged.
Evidence: src/config_cmd.rs:262-312, tests/config_lifecycle.rs:191-210, tests/config_lifecycle.rs:577-636

#### Scenario: Valid edit
- **WHEN** the editor changes `8080` to `9090` and the result is valid
- **THEN** the file contains `9090`, is mode 0600 and no `.rejected` file exists

#### Scenario: No change
- **WHEN** the editor leaves the text unchanged
- **THEN** `No changes.` is printed and the file is not rewritten

### Requirement: Rejected edits are kept in a sidecar
The system SHALL, when an edited document is invalid, exit with code 2, leave the live file unchanged, save the edited text to `<config>.rejected` (owner-only) and mention that path in the error.
Evidence: src/config_cmd.rs:296-318, tests/config_lifecycle.rs:163-189

#### Scenario: Invalid service name
- **WHEN** the editor appends a service named `BAD`
- **THEN** exit code is 2, the original file and its secret key are intact, stderr mentions `serve.toml.rejected`, the sidecar contains `BAD` and has mode 0600

### Requirement: Edits conflicting with concurrent changes are rejected
The system SHALL refuse to save an edit when the live file changed while the editor was open, keeping the concurrent change, writing the edit to the `.rejected` sidecar, and failing with a message containing `changed`.
Evidence: src/config_cmd.rs:279-292, tests/config_lifecycle.rs:212-235

#### Scenario: Concurrent add during edit
- **WHEN** another process adds a service while the editor is open and the editor then saves a different change
- **THEN** the command fails, the concurrently added service remains and the stale edit is not applied

### Requirement: Diagnostics never echo secret lines
The system SHALL report config parse errors with the message and line number only (e.g. `line 2`), without quoting the offending source line, so `secret_key` and `relay_token` values do not appear in output.
Evidence: src/config_store.rs:189-201, src/config_cmd.rs:420-428, tests/config_lifecycle.rs:540-555, tests/config_lifecycle_windows.rs:211

#### Scenario: Malformed token line
- **WHEN** `config show` or `config keygen --yes` is run on a file with `relay_token = "leak-me-token" oops` on line 2
- **THEN** exit code is 2, stderr contains `line 2` and does not contain `leak-me-token`

### Requirement: Run persists a generated key
The system SHALL, when `run` has no configured secret key, write a generated key into `[node].secret_key` in place (owner-only, comments preserved), reuse an already stored key, and use the same key and NodeId on later starts; when two starts race, the first stored key wins.
Evidence: src/config_store.rs:246-275, tests/config_lifecycle.rs:486-528

#### Scenario: Restart keeps identity
- **WHEN** `serve run --config c.toml` is started, killed, and started again on a config without a key
- **THEN** the file gains a key (mode 0600, `# keep me` comment intact) and both runs print the same NodeId
