## REMOVED Requirements

### Requirement: Drop counter docs expose the unusable-datagram deviation

**Reason**: The deviation it exposed (issue #152) is resolved: a connection whose datagrams are never usable now fails at setup and no longer counts as `limit_shrink`. Replaced by "Drop counter docs describe mid-session limit loss".

**Migration**: None for callers; only rustdoc text changes. The `limit_shrink` field name, type and serialization are unchanged.

## ADDED Requirements

### Requirement: Drop counter docs describe mid-session limit loss
The rustdoc of the `limit_shrink` drop counter, in both the engine observation and the host status schema, SHALL state that it counts datagrams dropped because the datagram limit became unusable during a session incarnation, and MUST NOT describe the resolved #152 deviation.
Source: issue #152, issue #167

#### Scenario: Mid-session meaning visible in API docs
- **WHEN** the rustdoc of `limit_shrink` in `iroh-tunnel-engine` and `iroh-tunnel-host` is read
- **THEN** both describe drops caused by the limit becoming unusable mid-session, and neither describes the never-usable deviation or references #152

### Requirement: Datagram-disable test seam stays in the explicit test feature graph
The engine SHALL let a role process started with `IROH_TUNNEL_TEST_DISABLE_DATAGRAMS=1` advertise no datagram support only when built with the `test-utils` feature. Builds without `test-utils` MUST NOT read that variable, so it cannot change their datagram behavior.
Source: issue #152, design.md D7

#### Scenario: Seam disables datagrams in one role
- **WHEN** a `test-utils` build starts one role with `IROH_TUNNEL_TEST_DISABLE_DATAGRAMS=1` and its peer without it
- **THEN** only the role with the variable advertises no datagram support, and the peer detects the setup failure

#### Scenario: Production build ignores the variable
- **WHEN** the engine is built without `test-utils`
- **THEN** no code reads `IROH_TUNNEL_TEST_DISABLE_DATAGRAMS`, and endpoints keep datagram support
