## ADDED Requirements

### Requirement: Node relay settings are replaced, kept or cleared by flag
The system SHALL, on `<role> config node`, apply exactly one of these mutually exclusive cases:
- `--clear-relays`: remove both `relay_urls` and `relay_token`;
- `-r/--relay-url` (repeatable): set `relay_urls` to exactly the given URLs in order, and set `relay_token` from a token flag if one is given, otherwise delete it;
- a token flag only: set `relay_token` and keep `relay_urls`.
Source: brainstorm.md Design Summary, Key Decisions; design.md D1, D6

#### Scenario: Replace relay list
- **WHEN** `[node]` has `relay_urls = ["https://old.example"]` and `serve config node -c c.toml -r https://a.example -r https://b.example` runs
- **THEN** `relay_urls = ["https://a.example", "https://b.example"]` and the exit code is 0

#### Scenario: Relay change without token deletes the token
- **WHEN** `[node]` has `relay_urls` and `relay_token = "t1"` and `access config node -r https://c.example` runs
- **THEN** `relay_urls = ["https://c.example"]` and `[node]` has no `relay_token`

#### Scenario: Relay and token replaced together
- **WHEN** `serve config node -r https://a.example --relay-token t2` runs on a node with `relay_token = "t1"`
- **THEN** `relay_urls = ["https://a.example"]` and `relay_token = "t2"`

#### Scenario: Token only keeps relays
- **WHEN** `[node]` has `relay_urls = ["https://a.example"]` and `serve config node --relay-token t3` runs
- **THEN** `relay_urls` is unchanged and `relay_token = "t3"`

#### Scenario: Clear relays
- **WHEN** `[node]` has `relay_urls`, `relay_token` and `secret_key`, and `serve config node --clear-relays` runs
- **THEN** `[node]` has neither `relay_urls` nor `relay_token`, and `secret_key` is unchanged

### Requirement: Node command touches only the role's relay keys
The system SHALL make `config node` write only that role's config file and SHALL leave other `[node]` keys, `[[services]]` entries and comments unchanged, except the retired-field cleanup every write performs. A missing config file SHALL be created holding only `[node]` when the edit stores a value; an edit that stores nothing (such as `--clear-relays`) SHALL report unchanged and create no file.
Source: brainstorm.md Design Summary; design.md D6, D8

#### Scenario: Role isolation
- **WHEN** `serve config node -r https://a.example` runs
- **THEN** only the serve config file is written and the access config file is untouched

#### Scenario: Services and comments preserved
- **WHEN** `config node -r https://a.example` runs on a file with services and a `# operator comment`
- **THEN** the services and the comment are unchanged

#### Scenario: Missing file
- **WHEN** `serve config node -c new.toml -r https://a.example` runs and `new.toml` does not exist
- **THEN** `new.toml` is created holding only `[node]` with `relay_urls = ["https://a.example"]`

#### Scenario: Clear on a missing file
- **WHEN** `serve config node -c new.toml --clear-relays` runs and `new.toml` does not exist
- **THEN** stdout is `Node settings unchanged (new.toml).`, the exit code is 0 and `new.toml` is not created

#### Scenario: Retired fields dropped
- **WHEN** an access config has a service with a legacy `multiplex` key and `access config node -r https://a.example` runs
- **THEN** `relay_urls` is set, the `multiplex` key is removed, and every other service key is unchanged

### Requirement: Relay token input sources
The system SHALL read `--relay-token-file PATH` from the file, or from stdin when `PATH` is `-`, trimming one trailing line ending only. An unreadable or empty source SHALL fail with exit code 2, no help, naming the flag and path but never the contents, and SHALL leave the config unchanged. The `--relay-token` help SHALL warn that the value is visible in shell history and process lists.
Source: brainstorm.md Key Decisions; design.md D3

#### Scenario: Token from file
- **WHEN** `tok.txt` holds `secret-token\n` and `serve config node -r https://a.example --relay-token-file tok.txt` runs
- **THEN** `relay_token = "secret-token"`

#### Scenario: Token from stdin
- **WHEN** `printf 'secret-token' | serve config node --relay-token-file - -r https://a.example` runs
- **THEN** `relay_token = "secret-token"`

#### Scenario: Unreadable token file
- **WHEN** `--relay-token-file missing.txt` is given and the file does not exist
- **THEN** the exit code is 2, stderr names `--relay-token-file` and `missing.txt` without `Usage:`, and the config file is unchanged

#### Scenario: Token file with CRLF
- **WHEN** `tok.txt` holds `secret-token\r\n` and `--relay-token-file tok.txt` is given with `-r https://a.example`
- **THEN** `relay_token = "secret-token"`

#### Scenario: Token file with extra line
- **WHEN** `tok.txt` holds `secret-token\n\n` and `--relay-token-file tok.txt` is given with `-r https://a.example`
- **THEN** the exit code is 2 with the schema's `invalid relay_token` error, stdout and stderr do not contain `secret-token`, and the config file is unchanged

#### Scenario: Empty token source
- **WHEN** `--relay-token-file` points to an empty file
- **THEN** the exit code is 2, the error says the source is empty, and the config file is unchanged

#### Scenario: Argv token warning
- **WHEN** `serve config node --help` is run
- **THEN** the `--relay-token` help mentions shell history and recommends `--relay-token-file`

#### Scenario: Token with whitespace
- **WHEN** `--relay-token "has space"` is given with `-r https://a.example`
- **THEN** the exit code is 2 with the schema's `invalid relay_token` error and the config file is unchanged

### Requirement: Relay values are validated before writing
The system SHALL reject, with exit code 2, no help, and the config file unchanged:
- a relay URL that does not start with `https://`, naming the URL;
- a URL repeated within one `-r` list, naming the duplicate;
- a `config node` token flag whose resulting `relay_urls` would be empty.

These rules SHALL apply to `config node` and to access `config set`.
Source: brainstorm.md Key Decisions; design.md D4, D5

#### Scenario: Insecure relay URL
- **WHEN** `serve config node -r http://relay.example` runs
- **THEN** the exit code is 2, stderr contains `invalid relay_url 'http://relay.example'`, and the file is unchanged

#### Scenario: Duplicate relay URL
- **WHEN** `serve config node -r https://a.example -r https://a.example` runs
- **THEN** the exit code is 2, stderr names `https://a.example` as a duplicate, and the file is unchanged

#### Scenario: Duplicate override URL
- **WHEN** `access config set -n web -r https://a.example -r https://a.example` runs
- **THEN** the exit code is 2, stderr names `https://a.example` as a duplicate, and the file is unchanged

#### Scenario: Token without relays
- **WHEN** `[node]` has no `relay_urls` and `serve config node --relay-token t` runs
- **THEN** the exit code is 2, the error says a relay token needs relay URLs set with `-r`, and the file is unchanged

### Requirement: Node command reports what changed without revealing the token
The system SHALL, after `config node` changes a value, print `Updated node settings in <path>:` and one `  <field>: <old> -> <new>` line per changed field. Lists read `[url, url]` or `(default)`; a token reads `<redacted>` or `(none)`. With no difference it SHALL print `Node settings unchanged (<path>).` without rewriting. The token value SHALL never appear in output.
Source: brainstorm.md Key Decisions; design.md D7

#### Scenario: Change report
- **WHEN** `[node]` has `relay_urls = ["https://old.example"]`, `relay_token = "t1"`, and `serve config node -r https://a.example` runs
- **THEN** stdout lists `relay_urls: [https://old.example] -> [https://a.example]` and `relay_token: <redacted> -> (none)`
- **AND** neither stdout nor stderr contains `t1`

#### Scenario: Token replaced
- **WHEN** `relay_token = "t1"` and `serve config node --relay-token t2` runs with relays present
- **THEN** stdout lists `relay_token: <redacted> -> <redacted> (changed)` and does not contain `t1` or `t2`

#### Scenario: Nothing changed
- **WHEN** `[node]` already has `relay_urls = ["https://a.example"]` and no token, and `serve config node -r https://a.example` runs
- **THEN** stdout is `Node settings unchanged (<path>).`, the exit code is 0, and the file's bytes and modification time are unchanged

### Requirement: Access set replaces a service's relay override
The system SHALL accept, on `access config set -n NAME`, `-r/--relay-url URL` (repeatable) to replace that service's `relay_urls` override with exactly the given list, and `--clear-relays` (conflicting with `-r`) to remove the override so the service uses `[node]` relays. Without either flag the override SHALL be kept. The flags SHALL also apply when the entry is created. `[node]` SHALL never be changed by `config set`.
Source: brainstorm.md Design Summary; design.md D2, D6

#### Scenario: Set override on existing service
- **WHEN** access service `web` has no `relay_urls` and `access config set -n web -r https://a.example -r https://b.example` runs
- **THEN** `web` holds `relay_urls = ["https://a.example", "https://b.example"]` and `[node]` is unchanged

#### Scenario: Replace override
- **WHEN** `web` has `relay_urls = ["https://old.example"]` and `access config set -n web -r https://a.example` runs
- **THEN** `web` holds `relay_urls = ["https://a.example"]`

#### Scenario: Clear override
- **WHEN** `web` has `relay_urls` and `access config set -n web --clear-relays` runs
- **THEN** `web` has no `relay_urls` key

#### Scenario: Override kept without relay flags
- **WHEN** `web` has `relay_urls = ["https://a.example"]` and `access config set -n web -l 9090` runs
- **THEN** `web` still holds `relay_urls = ["https://a.example"]`

#### Scenario: Create with override
- **WHEN** `access config set -n web -l 8080 -N <id> -r https://a.example` runs and `web` does not exist
- **THEN** the new entry holds `relay_urls = ["https://a.example"]`

#### Scenario: Insecure override
- **WHEN** `access config set -n web -r http://a.example` runs
- **THEN** the exit code is 2 with `invalid relay_url` naming the URL and `'web'`, and the file is unchanged

## MODIFIED Requirements

### Requirement: Mutations are validated and all-or-nothing
The system SHALL validate the resulting document against the role's schema before writing any mutation (`set`, `node`, `rm`, `keygen`, `edit`), and on any failure SHALL exit with code 2 and leave the config file byte-for-byte unchanged.
Evidence: src/config_cmd.rs:385-432, tests/config_lifecycle.rs:114-161, tests/config_lifecycle_windows.rs:54

#### Scenario: Invalid existing config
- **WHEN** set, node, rm, keygen `--yes`, or edit is run against an invalid config file
- **THEN** each exits with code 2 and the file bytes are identical to before

#### Scenario: Invalid new value
- **WHEN** `config set -n BAD -t 3000` is run
- **THEN** it exits with code 2 naming the name rule, without help, and the file is unchanged

#### Scenario: Node edit on a file with an invalid service
- **WHEN** `serve config node -r https://a.example` runs on a file whose service `BAD` breaks the name rule
- **THEN** it exits with code 2 without help and the file bytes are identical to before

### Requirement: Set reports what changed
The system SHALL, after updating an existing entry, print `Updated service '<name>' in <path>:` followed by one line per changed field in the form `  <field>: <old> -> <new>`, using the field labels `target` (serve), `listen` (access), `protocol`, `node_id` and `relay_urls` (access). A `relay_urls` value SHALL be shown as `[url, url]`, and an absent override as `(node)`. When no value differs it SHALL print `Service '<name>' unchanged (<path>).`, SHALL NOT rewrite the file, and SHALL exit 0. The report SHALL never include `[node]` values.
Source: brainstorm.md Key Decisions; design.md D3, D7

#### Scenario: Change report
- **WHEN** `serve config set -n web -t 3001 -p udp` changes `web` from `tcp` on `127.0.0.1:3000`
- **THEN** stdout lists `target: 127.0.0.1:3000 -> 127.0.0.1:3001` and `protocol: tcp -> udp`

#### Scenario: Relay override report
- **WHEN** access `web` has no override and `access config set -n web -r https://a.example` runs
- **THEN** stdout lists `relay_urls: (node) -> [https://a.example]`

#### Scenario: Nothing changed
- **WHEN** `serve config set -n web -t 3000` runs and `web` already targets `127.0.0.1:3000`
- **THEN** stdout is `Service 'web' unchanged (<path>).`, the exit code is 0 and the file's bytes and modification time are unchanged

#### Scenario: Name only on existing entry
- **WHEN** `serve config set -n web` runs and `web` exists
- **THEN** it reports `unchanged` and exits 0
