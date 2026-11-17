## MODIFIED Requirements

### Requirement: Abandoned edits leave no trace
An edit attempt that fails or is dropped without commit MUST remove its scratch copy, MUST NOT overwrite the live file and MUST NOT report a save. A commit that fails after reading the edited text MUST first write it to the `.rejected` sidecar and name it in the error. As exceptions, so the text is not lost, the scratch copy MUST be kept and named when that sidecar cannot be written, and when the commit cannot read the scratch copy.
Source: change keep-failed-edit-text design.md Decisions 1-3 (issue #222)

#### Scenario: Dropped attempt
- **WHEN** a prepared edit attempt is dropped without commit
- **THEN** its scratch copy is removed and the live file is unchanged

#### Scenario: Commit error keeps the text in the sidecar
- **WHEN** commit fails after reading the edited text, for example because the writer lock is not acquired in time
- **THEN** the live file is unchanged, the edited text is in the `.rejected` sidecar, the error names the sidecar and the scratch copy is removed

#### Scenario: Sidecar unavailable keeps the scratch copy
- **WHEN** commit rejects or fails on an edit and the `.rejected` sidecar cannot be written
- **THEN** the scratch copy still holds the edited text and the error or rejection message names it

#### Scenario: Unreadable scratch copy is kept
- **WHEN** commit cannot read an existing scratch copy
- **THEN** the error names the scratch copy and it is not removed
