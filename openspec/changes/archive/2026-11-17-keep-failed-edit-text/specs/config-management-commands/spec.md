## ADDED Requirements

### Requirement: Failed edit commits keep the edited text
The system SHALL, when `config edit` cannot save an edit because of a lock timeout or an I/O error, leave the live file unchanged, save the edited text to `<config>.rejected` (owner-only) and name it in the error with a reminder to delete it once recovered. If the sidecar cannot be written (also for rejections), the message MUST name the scratch copy holding the text. Saved or unchanged edits MUST NOT leave a scratch copy. Exit codes MUST stay as without this recovery.

#### Scenario: Lock held during commit
- **WHEN** another writer holds the config lock past the timeout while `config edit` commits an edit changing `8080` to `9090`
- **THEN** exit code is 2, the live file is unchanged, stderr mentions `serve.toml.rejected` and tells the user to delete it once recovered, the sidecar contains `9090` and has mode 0600, and no scratch copy remains

#### Scenario: Config directory not writable
- **WHEN** the config directory becomes read-only while the editor is open (Unix, not root)
- **THEN** the command fails, the live file is unchanged, and stderr names a file that exists and contains the edited text

#### Scenario: Saved or unchanged edit leaves no scratch copy
- **WHEN** the editor leaves the text unchanged, or saves a valid change
- **THEN** no `serve.toml.edit-*` file remains next to the config
