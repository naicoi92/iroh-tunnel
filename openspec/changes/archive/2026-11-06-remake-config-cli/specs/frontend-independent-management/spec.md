## MODIFIED Requirements

### Requirement: Configuration operations return outcomes
The host SHALL provide configuration operations for path resolution, set (create or update a service entry), remove, list, show, check, keygen, random key generation and edit. Each operation MUST return the affected service, path or data instead of a printed message. Set MUST return one of the following:
- `Created` with the service name and path;
- `Updated` with the path and the list of changed fields, each with its old and new value;
- `Unchanged` with the path.
Source: design.md D3, D7; openspec/specs/config-management-commands

#### Scenario: Add returns the affected entry
- **WHEN** a caller sets a valid Serve service entry whose name is not yet in an existing config
- **THEN** the operation returns `Created` with the service name and config path

#### Scenario: Random key touches no file
- **WHEN** a caller requests a random key
- **THEN** the secret key is returned explicitly and no config file is created or modified

#### Scenario: Update returns the changes
- **WHEN** a caller sets only a new port on an existing entry
- **THEN** the operation returns `Updated` with exactly one change, for the address, holding the old and new values

#### Scenario: Unchanged writes nothing
- **WHEN** a caller sets values equal to the existing entry's values
- **THEN** the operation returns `Unchanged` and the config file is not rewritten

#### Scenario: Check returns the validated path
- **WHEN** a caller checks a valid existing config
- **THEN** the operation returns the resolved path without taking the writer lock or writing

### Requirement: Configuration mutations keep store guarantees
Host configuration mutations MUST apply the existing writer lock, validation, all-or-nothing semantics, owner-only permissions, symlink write-through, atomic rename/fsync ordering and formatting preservation. When set would create an entry without the attributes required for creation, the host MUST return a typed missing-for-create error naming the missing attributes, decided under the writer lock, and MUST NOT write.
Source: design.md D2, D3; openspec/specs/config-management-commands

#### Scenario: Invalid mutation changes nothing
- **WHEN** a caller sets a new Access service without a node ID
- **THEN** the operation returns a missing-for-create error naming the node ID, and the config file bytes are unchanged

#### Scenario: Concurrent mutations serialize
- **WHEN** two callers set different new service entries on the same config concurrently
- **THEN** both entries are present afterwards and neither write is lost
