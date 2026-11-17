## MODIFIED Requirements

### Requirement: CLI start and restart re-check the registered binary
The system SHALL, for `service start` and `service restart` of a root-run registration on Linux or macOS, read the binary the service manager will actually run from its effective (loaded) definition with a read-only query, and apply the root-only binary check before any start, restart or stop command is issued. Start and restart SHALL NOT copy, refresh or replace the registered binary. Without a registration the verb SHALL behave as before. For BusyBox, the `DAEMON` value SHALL be read both from scripts that store it as a plain literal (unquoted safe characters, `'…'` segments and `\'`) and from scripts written by earlier versions that store it in double quotes; a value the shell would expand or reinterpret (`$`, backtick, `\`, an inner `"`, whitespace outside quotes, an unterminated quote) SHALL count as unreadable.

#### Scenario: Restart after an upgrade of a managed install
- **WHEN** the role runs from its managed copy, the user's Homebrew binary was upgraded, and `service restart --system` runs
- **THEN** the managed copy is unchanged and the service is restarted from it

#### Scenario: Effective definition per backend
- **WHEN** the binary is read for systemd, launchd or BusyBox
- **THEN** it is the loaded `ExecStart` program including drop-ins for systemd, the program of the loaded job for launchd, and `DAEMON` of the init script for BusyBox

#### Scenario: Loaded definition differs from the file
- **WHEN** the loaded launchd job or systemd unit runs a user-writable binary while the file on disk names a trusted one, and `service start --system` runs
- **THEN** the check is applied to the loaded binary and the command exits with code 3

#### Scenario: Trusted registered binary
- **WHEN** the registered binary passes the binary check and `service start --system` runs
- **THEN** the service manager is asked to start the service as before

#### Scenario: BusyBox script generations
- **WHEN** the init script stores `DAEMON` as a single-quoted literal containing a space and a `'`, or as a legacy double-quoted path
- **THEN** the effective binary is that exact path

#### Scenario: BusyBox DAEMON with shell expansion
- **WHEN** the `DAEMON` value contains an unquoted `$` or backtick, or is a legacy double-quoted value containing `$`, a backtick, `\` or `"`
- **THEN** the effective binary is unknown and CLI start and restart are refused with exit code 3
