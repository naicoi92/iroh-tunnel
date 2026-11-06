## MODIFIED Requirements

### Requirement: Host owns persistence and platform policy
`iroh-tunnel-host` SHALL be the single owner of configuration storage, identity persistence, path resolution, trust and ownership checks, foreground orchestration, status-file adapters, OS service backends and the updater. The root package SHALL NOT duplicate that policy.
Source: design.md Decisions 1, 3, 4

#### Scenario: CLI mutations go through host operations
- **WHEN** `iroh-tunnel config set`, `config rm`, `keygen` or `serve service install` mutates a file or registration
- **THEN** a host operation applies the lock, validation, privilege and trust policy, and the CLI only translates arguments, collects decisions and renders the outcome

#### Scenario: Creation requirements decided by host
- **WHEN** `config set` needs to know whether an entry exists to decide which flags are required
- **THEN** the host decides under the writer lock and returns a domain error, and the CLI alone turns it into a usage error with help
