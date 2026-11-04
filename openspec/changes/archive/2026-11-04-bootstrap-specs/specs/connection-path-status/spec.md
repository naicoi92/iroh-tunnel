## ADDED Requirements

### Requirement: Role-scoped status file names and location
The system SHALL write each role's status snapshot to `<state dir>/serve-status.json` for serve and `<state dir>/access-status.json` for access, side by side in one directory, with the access save never creating or touching the serve file.
Evidence: src/status.rs:55-63, src/status.rs:219-238, src/status.rs:611-617, src/status.rs:640-665

#### Scenario: Serve and access use distinct file names
- **WHEN** the serve writer and the access writer each save a snapshot into the same directory
- **THEN** the files are named `serve-status.json` and `access-status.json` respectively

#### Scenario: Access save leaves serve file absent
- **WHEN** only the access writer saves into an empty directory
- **THEN** `access-status.json` exists and `serve-status.json` does not

### Requirement: State directory override via IROH_TUNNEL_STATE_DIR
The system SHALL place the status file directly inside the directory named by a non-empty `IROH_TUNNEL_STATE_DIR` (no `iroh-tunnel` subpath appended, relative values resolved to an absolute path), and SHALL treat an empty value as unset and use the default OS state directory.
Evidence: src/status.rs:65-69, src/status.rs:296-322, src/status.rs:833-888

#### Scenario: Override replaces the directory entirely
- **WHEN** `IROH_TUNNEL_STATE_DIR` is `/tmp/status-override`
- **THEN** the serve path ends with `status-override/serve-status.json` and the access path ends with `status-override/access-status.json`

#### Scenario: Empty override is unset
- **WHEN** `IROH_TUNNEL_STATE_DIR` is empty
- **THEN** the path equals the one resolved with no override, under the default state directory

#### Scenario: Relative override is absolutized
- **WHEN** the override is a relative path such as `rel/state`
- **THEN** the resolved status file path is absolute

### Requirement: Atomic status file writes
The system SHALL write a status file by creating the directory if missing, writing and fsyncing a uniquely named sibling temp file, and renaming it over the destination, so a reader never sees a partial file, a repeated save replaces the previous contents, and no temp file is left behind (including when the rename fails).
Evidence: src/status.rs:361-459, src/status.rs:728-831

#### Scenario: Missing directory is created
- **WHEN** a status file is saved into a nested directory that does not exist
- **THEN** the directory is created and the file exists afterwards

#### Scenario: No temp file lingers
- **WHEN** a serve or access snapshot is saved successfully
- **THEN** no sibling `<file name>.tmp.*` file remains in the directory

#### Scenario: Second save overwrites
- **WHEN** a snapshot is saved twice into the same directory with a changed `node_id`
- **THEN** the file contains the second `node_id`

#### Scenario: Rename failure cleans up
- **WHEN** the destination slot is occupied by a directory so the rename fails
- **THEN** the save returns an error and no temp file is left

### Requirement: Writer and payload role must match
The system SHALL reject a save whose payload schema belongs to a different role than the writer, returning an error in every build profile rather than writing one role's schema under the other role's file name.
Evidence: src/status.rs:200-212, src/status.rs:383-395
Confidence: inferred

#### Scenario: Mismatched payload
- **WHEN** the serve writer is asked to save an access payload (or vice versa)
- **THEN** the save fails with an error stating the writer role does not match the payload role and no file is written

### Requirement: Legacy status.json removal
The system SHALL, after each successful save of either role's status file, remove a stale pre-rename `status.json` from the same directory on a best-effort basis, ignoring removal failures.
Evidence: src/status.rs:57-61, src/status.rs:448-457
Confidence: inferred

#### Scenario: Legacy file present
- **WHEN** a serve or access status file is saved and `status.json` exists in the same directory
- **THEN** `status.json` is removed

### Requirement: Serve status file schema
The system SHALL serialize the serve status file as JSON with `node_id`, `home_relay` (nullable), `pid`, `started_at` (Unix epoch seconds), `services` (each with `name`, `protocol`, `local_addr`, `active_connections` counting active streams, and an optional `udp` block present only for UDP services), and `connections`.
Evidence: src/status.rs:74-106, src/status.rs:619-638

#### Scenario: Saved serve file content
- **WHEN** a serve snapshot is saved
- **THEN** the file is valid JSON whose `node_id`, `pid` and `services[0].name` match the snapshot

#### Scenario: TCP service omits udp
- **WHEN** a service has no UDP diagnostics
- **THEN** its row has no `udp` key

### Requirement: UDP diagnostics block
The system SHALL include, for UDP services only, a `udp` object with `active_flows`, `quarantined_flows`, `datagram_limit` (null until a connection exists) and a `drops` object with counters `oversize`, `malformed`, `unknown_flow`, `upstream_error`, `cap`, `queue_full`, `limit_shrink`, `quarantine_reply` and `foreign_source`.
Evidence: src/status.rs:105-146, src/status.rs:100-103
Confidence: inferred

#### Scenario: UDP service row
- **WHEN** a UDP service's status is rendered into the status file
- **THEN** the row carries the `udp` object with those field names

### Requirement: Serve status connections schema
The system SHALL list each currently connected peer in the serve file's `connections` array as an object with `peer` (full node id), `services` (service names the peer is connected to), `transports` (each `kind`, `addr`, `active`) and `local_bound_addrs`, with the path fields flattened into the peer row.
Evidence: src/status.rs:148-160, src/status.rs:667-699, tests/serve_status_conn_path.rs:176-205

#### Scenario: Connected access peer reported
- **WHEN** an access node has completed a tunnel round-trip through serve and the 5 s flush has run
- **THEN** `serve-status.json` has exactly one `connections` entry whose `peer` is the access node id, whose `services` is `["echo"]`, whose `transports` is non-empty with `kind` of `relay` or `direct`, a non-empty `addr` and boolean `active`, and whose `local_bound_addrs` is non-empty

#### Scenario: Peer disappears after disconnect
- **WHEN** the access role shuts down and the next flush runs
- **THEN** the peer's entry is removed from `connections`

### Requirement: Access status file schema
The system SHALL serialize the access status file with `node_id`, `pid`, `started_at` and one `services` row per configured service in config order, each carrying `name`, `listen_addr`, `peer` (configured serve node id), `transports`, `local_bound_addrs` and an optional `udp` block, with no `home_relay` field, and the file SHALL deserialize back into the same structure.
Evidence: src/status.rs:162-198, src/status.rs:701-726, tests/access_status_file.rs:196-211

#### Scenario: Schema round-trip
- **WHEN** an access snapshot is serialized to JSON and parsed back
- **THEN** the parsed value equals the original and the field names are `listen_addr`, `peer`, `transports`, `local_bound_addrs`

### Requirement: Access status before and after first connection
The system SHALL write the access status file at startup showing the configured peer with an empty `transports` array (access dials only on the first local client), and SHALL later report the live transports and a non-empty `local_bound_addrs` for the service once a connection exists.
Evidence: src/status.rs:177-197, tests/access_status_file.rs:157-228

#### Scenario: Pre-connection file
- **WHEN** access has started and no local client has connected
- **THEN** `access-status.json` contains the node id, the service `name`, its `listen_addr`, `peer` equal to the serve node id, and an empty `transports` array

#### Scenario: Post-connection file
- **WHEN** a local client has completed a round-trip through the tunnel and the flush has run
- **THEN** the service's `transports` is non-empty, each with `kind` `relay` or `direct`, non-empty `addr` and boolean `active`, and `local_bound_addrs` is non-empty

### Requirement: Status flush cadence
The system SHALL refresh each role's status file about every 5 seconds and SHALL rewrite it only when the rendered snapshot changes.
Evidence: src/status.rs:66-72, src/conn_path.rs:96-108, tests/access_status_file.rs:49, tests/serve_status_conn_path.rs:40-41

#### Scenario: Change appears within the poll window
- **WHEN** a connection is established or closed
- **THEN** the status file reflects the change within one flush interval plus margin (tests allow 25 s)

### Requirement: Status file removed on graceful shutdown if owned
The system SHALL remove a role's status file on graceful shutdown only when the `pid` in the file equals the current process id, and SHALL skip removal when the file is missing, unreadable, unparsable, or owned by another pid.
Evidence: src/status.rs:473-552, src/status.rs:890-935, tests/access_status_file.rs:253-266

#### Scenario: Own file removed
- **WHEN** the file's pid is the current process and cleanup runs
- **THEN** the file is deleted

#### Scenario: Foreign pid preserved
- **WHEN** the file's pid belongs to another process
- **THEN** the file remains

#### Scenario: Missing file
- **WHEN** no status file exists at cleanup
- **THEN** cleanup is a no-op without error

#### Scenario: End-to-end shutdown
- **WHEN** access and serve roles shut down gracefully
- **THEN** `access-status.json` and `serve-status.json` no longer exist

### Requirement: Local address formatting in status files
The system SHALL format `host:port` values in status files with IPv6 literals bracketed (`[::1]:8080`) and IPv4 addresses and hostnames left as `host:port`.
Evidence: src/status.rs:461-472, src/status.rs:786-803

#### Scenario: IPv6 host
- **WHEN** the host is `::1` and port `8080`
- **THEN** the result is `[::1]:8080`

#### Scenario: IPv4 or hostname
- **WHEN** the host is `127.0.0.1` or `localhost`
- **THEN** the result is `127.0.0.1:8080` or `localhost:8080`

### Requirement: Peer path snapshot contents and ordering
The system SHALL report a peer's path as its full node id, the relay and direct transports iroh knows for it, and the sorted local UDP socket candidates of the endpoint, with transports ordered active first, then relay before direct, then by address, duplicates collapsed, and non-relay/non-direct (custom) transports dropped, returning no report when iroh has no entry for the peer.
Evidence: src/conn_path.rs:76-160, src/conn_path.rs:349-420, src/conn_path.rs:453-466
Confidence: inferred

#### Scenario: Active transports sort first
- **WHEN** transports contain inactive relay, active direct and inactive direct
- **THEN** the order is active direct, inactive relay, inactive direct

#### Scenario: Address tie-break
- **WHEN** several transports share active state and kind
- **THEN** they are ordered by `addr`

#### Scenario: Custom transport dropped
- **WHEN** iroh reports a custom transport address
- **THEN** it does not appear in the report

#### Scenario: Never-connected peer
- **WHEN** the endpoint has no remote entry for the peer
- **THEN** no report is produced (not an empty one)

### Requirement: Transport kind serialization
The system SHALL serialize transport kinds as lowercase `relay` and `direct`, and render them identically in logs and tables.
Evidence: src/conn_path.rs:44-67, src/conn_path.rs:422-465

#### Scenario: JSON form
- **WHEN** a relay or direct transport is serialized
- **THEN** `kind` is `"relay"` or `"direct"` and equals its display text

### Requirement: Status command output
The system SHALL, for `iroh-tunnel <role> status`, print the role's status file as human-readable tables by default, or print the file contents byte-for-byte unchanged with `--json`, after validating it parses as the role's schema.
Evidence: src/status_cmd.rs:27-95,414-484 (unit tests), src/cli.rs:202-206
Confidence: inferred

#### Scenario: JSON mode is verbatim
- **WHEN** `status --json` is run with a valid status file
- **THEN** stdout is the file body unchanged

#### Scenario: Invalid file
- **WHEN** the status file exists but does not parse
- **THEN** the command fails with an error naming the file path

#### Scenario: Unreadable file
- **WHEN** reading the status file fails for a reason other than not-found (for example a directory at the path)
- **THEN** the command fails with "failed to read status file" and the path, rather than reporting "not running"

### Requirement: Status command when role is not running
The system SHALL, when the status file is missing and no live owner is detected, fail with the general error (exit 1) and a message naming the role, the file name and the exact path searched.
Evidence: src/status_cmd.rs:1-16, src/status_cmd.rs:39-58
Confidence: inferred

#### Scenario: No file, no owner
- **WHEN** `serve status` runs and the file is missing and the ownership probe reports not running
- **THEN** the error reads `serve is not running (no serve-status.json found at <path>)`

### Requirement: Status command when running but snapshot absent
The system SHALL, when the file is missing but the role is live, report success: with `--json` a JSON object with `role`, `running: true`, `pid`, `node_id` and `snapshot: null`, otherwise a line naming the live owner, its tier and the path (noting another state directory may be in use).
Evidence: src/status_cmd.rs:40-56, src/status_cmd.rs:139-148
Confidence: inferred

#### Scenario: JSON for live owner without file
- **WHEN** `status --json` runs, the file is absent and the role is running
- **THEN** the output is one JSON object with `running` true and `snapshot` null

#### Scenario: Text for live owner without file
- **WHEN** `status` runs, the file is absent and the role is running
- **THEN** the output line says `<role> is running for …; no status snapshot at <path> (it may use another state directory)`

### Requirement: Stale snapshot warnings
The system SHALL still print the snapshot on stdout while writing a warning to stderr when the role is not running, when the running owner's pid differs from the snapshot's pid, or when the running role's owner record is missing, each warning stating the snapshot is stale or unverifiable and showing its `started_at`.
Evidence: src/status_cmd.rs:104-137
Confidence: inferred

#### Scenario: Not running, file left behind
- **WHEN** the snapshot exists but the ownership probe reports not running
- **THEN** stderr warns `<role> is not running; this snapshot (pid N) is stale` and stdout still shows the snapshot

#### Scenario: Different live pid
- **WHEN** the role runs as another pid than the snapshot's
- **THEN** stderr warns the snapshot was written by the old pid and is stale

#### Scenario: Matching live owner
- **WHEN** the running owner pid equals the snapshot pid
- **THEN** no warning is printed

### Requirement: Broken pipe tolerance
The system SHALL treat a closed stdout pipe (for example `status | head`) as a normal exit while propagating other write failures.
Evidence: src/status_cmd.rs:92-107
Confidence: inferred

#### Scenario: Consumer closes early
- **WHEN** the status command writes to a closed pipe
- **THEN** it exits successfully without panicking

### Requirement: Serve status table rendering
The system SHALL render the serve status as a header of `node_id`, `home_relay` (`none` when absent) and `pid`/`started_at`, followed by a SERVICE/PROTOCOL/LOCAL ADDR/STREAMS table (or `services: none`) and a PEER/SERVICES/TRANSPORTS table (or `connections: none`), with peer ids shortened, several services joined by commas, and each transport on its own aligned line.
Evidence: src/status_cmd.rs:150-205, src/status_cmd.rs:414-450, tests/access_status_file.rs:236-251

#### Scenario: Full table
- **WHEN** a serve file with one service and one peer with relay and direct transports is rendered
- **THEN** the output matches the golden table with `1aa27080…` as the peer and `relay https://relay.example/ [active]` above `direct 192.168.1.10:52618`

#### Scenario: No relay or connections
- **WHEN** `home_relay` is null and there are no connections
- **THEN** the output shows `home_relay: none` and `connections: none`

### Requirement: Access status table rendering
The system SHALL render the access status as `node_id` and `pid`/`started_at` lines followed by a SERVICE/LISTEN/PEER/TRANSPORTS table, showing `(no connection)` for services without transports and `services: none` when there are no services, with every configured service getting a row.
Evidence: src/status_cmd.rs:207-245, src/status_cmd.rs:452-477, tests/access_status_file.rs:236-246

#### Scenario: Mixed connected and unconnected services
- **WHEN** one service has transports and another has none
- **THEN** the first lists its transports and the second shows its configured short peer id with `(no connection)`

#### Scenario: Empty service list
- **WHEN** the access file has no services
- **THEN** the output ends with `services: none`

### Requirement: Transport line rendering in tables
The system SHALL render each transport as `<kind> <addr>` and append ` [active]` only to transports iroh is actively sending on.
Evidence: src/status_cmd.rs:254-275, src/status_cmd.rs:414-431

#### Scenario: Idle candidate
- **WHEN** a direct transport has `active` false
- **THEN** its line is `direct <addr>` without the marker

### Requirement: Short peer id form
The system SHALL abbreviate node ids in tables and log lines to their first 8 characters followed by `…`, keeping shorter ids whole with the ellipsis, while machine-readable fields carry the full id.
Evidence: src/conn_path.rs:223-239, src/status_cmd.rs:478-484

#### Scenario: Truncation
- **WHEN** a 64-hex-character id `1aa27080cc69…` is abbreviated
- **THEN** the result is `1aa27080…`

#### Scenario: Short id
- **WHEN** the id is `peer456`
- **THEN** the result is `peer456…`

### Requirement: Active-transport log rendering
The system SHALL render active transports in log lines as `relay=<url>` or `direct=<addr>` joined by `, `, skipping inactive candidates, and SHALL render an established connection's paths as `paths pending` when no active path snapshot exists.
Evidence: src/conn_path.rs:241-266, src/conn_path.rs:562-578, tests/access_path_logging.rs:230-247

#### Scenario: Mixed active and inactive
- **WHEN** two active transports and one inactive transport are rendered
- **THEN** the output is `relay=https://use1-1.relay.iroh.network/, direct=192.168.1.10:52618`

#### Scenario: No snapshot yet
- **WHEN** the report is absent or has no active transports
- **THEN** the established rendering is `paths pending`

### Requirement: Connection established log lines
The system SHALL log connection establishment with the service, full peer node id and active relay/direct transports for both roles, regardless of a legacy multiplex setting.
Evidence: tests/access_path_logging.rs:226-262, src/conn_path.rs:253-266

#### Scenario: Role-specific connection format
- **WHEN** a service connection is established
- **THEN** Access logs `connected to serve peer (multiplexed`, `peer=<id>` and trailing `svc_name=<service>`; Serve logs `peer connected via` with `peer=<id>` and `service=<service>`

#### Scenario: Access established lines
- **WHEN** clients round-trip through a multiplexed service `echo` and a `multiplex = false` service `echo2`
- **THEN** each service has its own `connected to serve peer (multiplexed` line with the serve peer's full id and a rendered active transport

#### Scenario: Serve accept line
- **WHEN** the access peer connects to serve
- **THEN** serve logs `peer connected via …` with `peer=<access node id>` and `service=echo`

### Requirement: Path change detection
The system SHALL consider a path changed only when the set of active `(kind, addr)` pairs differs between two snapshots, ignoring ordering and inactive candidates; an address swap within one kind SHALL count as that kind being both added and removed, and an active-to-inactive flip SHALL count as removal.
Evidence: src/conn_path.rs:166-221, src/conn_path.rs:480-560

#### Scenario: No active change
- **WHEN** snapshots differ only in order or inactive candidates
- **THEN** no change is reported

#### Scenario: Direct path gained
- **WHEN** active transports go from relay to relay plus direct
- **THEN** `direct` is reported as added and nothing as removed

#### Scenario: Relay failover
- **WHEN** the active relay URL changes to another relay URL
- **THEN** `relay` is both added and removed

### Requirement: Path change log line
The system SHALL log each active-path change once as `path changed <from>→<to> (now active: <transports>)`, using relay/direct kinds; initial, unchanged and inactive snapshots SHALL stay silent without losing the last active baseline.
Evidence: src/conn_path.rs:268-339, src/conn_path.rs:580-662, tests/access_path_logging.rs:264-285

#### Scenario: Kind rendering
- **WHEN** a path-change line is rendered
- **THEN** kinds use `relay`, `direct`, `relay+direct` or `none` as appropriate

#### Scenario: Relay to direct
- **WHEN** the baseline is active relay, a transient all-inactive snapshot arrives, then an active direct snapshot arrives
- **THEN** one line `path changed relay→direct (now active: direct=203.0.113.7:41641)` is logged

#### Scenario: First active snapshot
- **WHEN** the previous baseline had no active transport and an active snapshot arrives
- **THEN** no line is logged and the baseline advances

#### Scenario: Steady state
- **WHEN** nothing changes between polls (poll interval 5 s)
- **THEN** no `path changed` lines are emitted after initial migrations settle
