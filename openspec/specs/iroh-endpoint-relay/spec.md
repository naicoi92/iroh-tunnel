# iroh-endpoint-relay Specification

## Purpose

Define how Serve and Access create their Iroh endpoints (NodeId identity, ALPNs, keep-alive and stream budget) and use relays: default and custom relay sets, relay token, per-service override and dial addresses. It also covers the self-hosted relay installer.

## Requirements

### Requirement: Serve endpoint identity and ALPNs
The system SHALL build the serve-role endpoint with the secret key resolved from the node configuration (generating a fresh key when none is configured) and SHALL register every supplied service ALPN on the endpoint so it accepts incoming streams on all of them.
Evidence: src/endpoint.rs:72-77, src/endpoint.rs:131-135, src/endpoint.rs:364-377 (serve_endpoint_with_same_key_has_same_node_id)

#### Scenario: Pinned key gives stable node id
- **WHEN** a serve endpoint is created with a configured secret key
- **THEN** its node id equals the public key of that secret key

#### Scenario: ALPNs registered
- **WHEN** a serve endpoint is created with a non-empty ALPN list
- **THEN** the endpoint is bound accepting those ALPNs

### Requirement: Access endpoint identity
The system SHALL build the access-role endpoint with the secret key resolved from the node configuration, so a pinned key yields a stable node id, and SHALL register no ALPNs because access only dials out.
Evidence: src/endpoint.rs:79-104, src/endpoint.rs:352-393 (access_endpoint_binds_and_has_node_id, access_endpoint_with_same_key_has_same_node_id)

#### Scenario: Pinned access key
- **WHEN** an access endpoint is created with a configured secret key
- **THEN** its node id equals that key's public key

#### Scenario: Unpinned access key
- **WHEN** an access endpoint is created with an empty secret key
- **THEN** it binds with a generated key whose node id is a 64-character hex string

### Requirement: Default to n0 public relays when none configured
The system SHALL use iroh's default (n0 public) relay mode for the endpoint when the configured relay URL list is empty.
Evidence: src/endpoint.rs:170-188, src/endpoint.rs:272-276 (empty_relay_urls_yields_default_mode), docs/self-hosted-relay.md:6-9

#### Scenario: Empty relay list
- **WHEN** no relay_urls are configured
- **THEN** the endpoint uses the default n0 relay map

### Requirement: Custom relay set with home and failover
The system SHALL, when relay URLs are configured, use a custom relay map containing every configured URL, treating the first as the home relay and the rest as failover candidates.
Evidence: src/endpoint.rs:8-10, src/endpoint.rs:170-204, src/endpoint.rs:294-318 (custom_relay_urls_parse_into_custom_mode)

#### Scenario: Two relays configured
- **WHEN** two relay URLs are configured
- **THEN** the endpoint's relay map contains both URLs

### Requirement: Invalid relay URL rejected
The system SHALL fail endpoint construction, and relay-URL resolution for dialing, with an error naming the offending entry ("invalid relay_url: <value>") when a configured relay URL cannot be parsed.
Evidence: src/endpoint.rs:189-195, src/endpoint.rs:236-247, src/endpoint.rs:344-349 (invalid_relay_url_errors), src/endpoint.rs:420-425 (resolve_relay_urls_errors_on_invalid_url)

#### Scenario: Unparseable URL
- **WHEN** a relay URL such as "not a url" is configured
- **THEN** an error containing "invalid relay_url" is returned

### Requirement: Relay token applied to all relays
The system SHALL apply the configured relay_token to every relay in the custom relay map (home and failover), sent as an `Authorization: Bearer` header on the relay WebSocket upgrade; relays without access control ignore it.
Evidence: src/endpoint.rs:177-203, src/endpoint.rs:320-342 (relay_token_is_applied_to_all_map_entries), docs/self-hosted-relay.md:297-306, tests/relay_auth.rs:67-87

#### Scenario: Token on multiple relays
- **WHEN** two relays and a relay_token are configured
- **THEN** each relay entry in the map carries that token

#### Scenario: Matching token end to end
- **WHEN** serve and access both use a token-enforcing relay and the correct token
- **THEN** a payload written to the access local port is echoed back unchanged through the tunnel

### Requirement: Unused relay token warning
The system SHALL log a warning that the relay_token is unused, and fall back to the n0 default relays, when relay_token is set but relay_urls is empty.
Evidence: src/endpoint.rs:180-188
Confidence: inferred

#### Scenario: Token without relays
- **WHEN** relay_token is set and relay_urls is empty
- **THEN** a warning is emitted and the default relay mode is used

### Requirement: Wrong relay token denies relay traffic
The system SHALL NOT complete a tunnel round trip through a token-enforcing relay when the access role presents a wrong token; the local TCP connection stays open and no echo arrives while the dial retry loop backs off silently.
Evidence: tests/relay_auth.rs:93-122, tests/relay_auth.rs:13-16, docs/self-hosted-relay.md:341

#### Scenario: Access with wrong token
- **WHEN** serve uses the correct token and access uses a wrong token
- **THEN** no echo is received within 5 seconds and the client connection remains open

### Requirement: Per-service relay override
The system SHALL, for the access role, connect the endpoint to the union (without duplicates) of node relay URLs and per-service relay override URLs, while per-service dials attach only that service's own URLs.
Evidence: src/endpoint.rs:87-104, tests/relay_auth.rs:128-151, tests/relay_auth.rs:17-22, docs/self-hosted-relay.md:308-310

#### Scenario: Node has no relays, service overrides
- **WHEN** the access node has no relay_urls and one service overrides relay_urls to the relay where the serve peer is registered
- **THEN** the round trip through that relay succeeds

### Requirement: Dial relay URL resolution
The system SHALL resolve the relay URLs used to dial a peer by parsing the configured URLs, and SHALL fall back to the n0 default relay URLs when the configured list is empty.
Evidence: src/endpoint.rs:221-247, src/endpoint.rs:397-418 (resolve_relay_urls_falls_back_to_n0_defaults_when_empty, resolve_relay_urls_parses_configured_urls)

#### Scenario: Empty config
- **WHEN** no relay URLs are configured
- **THEN** the n0 default relay URLs (including relay.n0.iroh.link hostnames) are returned

#### Scenario: Configured URLs
- **WHEN** two valid URLs are configured
- **THEN** both are returned in configured order

### Requirement: Dial address carries every relay URL
The system SHALL build the dial address for a peer by attaching every supplied relay URL to the peer's node id, and SHALL produce an address with only the node id (no panic) when the list is empty.
Evidence: src/endpoint.rs:249-266, src/endpoint.rs:427-462 (build_dial_addr_attaches_every_relay_url, build_dial_addr_with_empty_urls_is_just_the_node_id)

#### Scenario: Three relays
- **WHEN** a dial address is built with three relay URLs
- **THEN** the address lists exactly those three relay URLs

#### Scenario: No relays
- **WHEN** a dial address is built with an empty URL list
- **THEN** the address has zero relay URLs

### Requirement: Transport keep-alive and stream budget
The system SHALL configure endpoint transport with a 5-second QUIC keep-alive interval, and SHALL apply the configured max concurrent bidirectional stream limit when present, otherwise keeping the transport default.
Evidence: src/endpoint.rs:140-168
Confidence: inferred

#### Scenario: Stream limit configured
- **WHEN** max_concurrent_streams is set in the node configuration
- **THEN** the endpoint's concurrent bidirectional stream limit is that value

#### Scenario: Stream limit absent
- **WHEN** max_concurrent_streams is not set
- **THEN** the default stream limit is kept

### Requirement: Home relay display
The system SHALL report an endpoint's home relay URL when assigned, and SHALL report none right after bind if the relay has not yet been assigned.
Evidence: src/endpoint.rs:211-219
Confidence: inferred

#### Scenario: Immediately after bind
- **WHEN** the home relay is queried before the relay connection is established
- **THEN** no URL may be returned

### Requirement: Test-only relay trust and relay-only mode
The system SHALL, only when built with the `test-utils` feature, skip TLS verification of relay certificates and optionally clear IP transports so all paths use relays; production builds SHALL keep the platform verifier.
Evidence: src/endpoint.rs:22-40, src/endpoint.rs:121-129, tests/relay_auth.rs:24-26

#### Scenario: Self-signed in-process relay
- **WHEN** built with test-utils and connecting to a relay with a self-signed certificate
- **THEN** the endpoint connects without certificate errors

### Requirement: Self-hosted relay installer preflight
The system SHALL, through the self-hosted relay installer, run only as root on Debian (warning on non-12 versions), on x86_64 or aarch64, and SHALL require --domain when no TTY is available to prompt.
Evidence: docs/install-relay-debian.sh:98-126, docs/self-hosted-relay.md:64-81

#### Scenario: Non-Debian host
- **WHEN** the script runs on a non-Debian OS
- **THEN** it exits with an error

#### Scenario: Piped without domain
- **WHEN** the script runs with no --domain and no TTY
- **THEN** it exits with "missing --domain"

#### Scenario: Unsupported architecture
- **WHEN** the machine architecture is not x86_64 or aarch64
- **THEN** it exits with "unsupported arch"

### Requirement: Installer verifies and installs the relay binary
The system SHALL, through the relay installer, resolve the iroh-relay release asset (default pin v1.1.0, or newest with --latest) through the GitHub Releases API, verify the asset's sha256 digest when provided (aborting on mismatch, warning if absent), and install the binary to /usr/local/bin/iroh-relay.
Evidence: docs/install-relay-debian.sh:182-213, docs/self-hosted-relay.md:64-70, docs/self-hosted-relay.md:343-347

#### Scenario: Digest mismatch
- **WHEN** the downloaded asset's sha256 does not match the API digest
- **THEN** the install aborts

#### Scenario: Asset missing
- **WHEN** the asset for the target is not in the release
- **THEN** the install exits with "asset ... not found"

### Requirement: Relay bound to loopback with metrics
The system SHALL, through the relay installer, write /etc/iroh-relay/config.toml (mode 0600) binding the relay HTTP server to 127.0.0.1:3340 and metrics to 127.0.0.1:9090, with metrics enabled, because the relay's own defaults bind all interfaces.
Evidence: docs/install-relay-debian.sh:236-265, docs/self-hosted-relay.md:29-35, docs/self-hosted-relay.md:186-201

#### Scenario: Default install
- **WHEN** the installer completes
- **THEN** the config sets http_bind_addr to 127.0.0.1:3340 and metrics_bind_addr to 127.0.0.1:9090

### Requirement: Open relay by default, token with --enable-token
The system SHALL, through the relay installer, leave the relay open (access = everyone) with a warning by default, and SHALL write `access.shared_token` into the config only when --enable-token is given, using the --token value, else the persisted token, else a generated one.
Evidence: docs/install-relay-debian.sh:128-169, docs/install-relay-debian.sh:247-251, docs/self-hosted-relay.md:54-62, docs/self-hosted-relay.md:194-196

#### Scenario: Default run
- **WHEN** --enable-token is not passed
- **THEN** no shared_token is configured and an OPEN-relay warning is printed (a supplied --token is ignored with a warning)

#### Scenario: Token enabled without value
- **WHEN** --enable-token is passed with no --token and no persisted token
- **THEN** a token is generated, stored in /etc/iroh-relay/token (0600), and printed in the summary

#### Scenario: Token with a double quote
- **WHEN** the token contains a double quote
- **THEN** the installer exits with an error

### Requirement: Token persists across toggling and re-runs
The system SHALL, through the relay installer, persist the token in /etc/iroh-relay/token, migrate a token embedded in an older config.toml into it, and be idempotent on re-run.
Evidence: docs/install-relay-debian.sh:128-147, docs/self-hosted-relay.md:88-89, docs/self-hosted-relay.md:194-196

#### Scenario: Toggle token off then on
- **WHEN** the installer is re-run without then with --enable-token
- **THEN** the same persisted token is reused

### Requirement: Optional QUIC address discovery
The system SHALL, through the relay installer,, only with --enable-quic, resolve a PEM cert and key (from --quic-cert/--quic-key or Caddy's ACME cert for the domain), fail early if unreadable, and append a `[tls]` Reloading config with `quic_bind_addr = "0.0.0.0:<port>"` (default 7824/udp); re-running without the flag disables it.
Evidence: docs/install-relay-debian.sh:216-233, docs/install-relay-debian.sh:252-264, docs/self-hosted-relay.md:128-184

#### Scenario: Cert not found
- **WHEN** --enable-quic is used and no cert can be found or read
- **THEN** the installer exits before changing the service, with guidance

#### Scenario: QUIC enabled
- **WHEN** a usable cert and key are found
- **THEN** the config sets cert_mode "Reloading" and an explicit QUIC bind address

### Requirement: systemd service and hang-guard
The system SHALL, through the relay installer, install a systemd service running `iroh-relay --config-path=/etc/iroh-relay/config.toml --dev` with Restart=always, CPUQuota=200% and MemoryMax=1G, plus a 30-second timer-driven health unit that restarts the relay when `http://127.0.0.1:3340/healthz` fails, and SHALL fail with journal logs if /healthz does not return 200 within 15 seconds.
Evidence: docs/install-relay-debian.sh:267-328, docs/self-hosted-relay.md:203-249

#### Scenario: Relay hangs
- **WHEN** the health check curl fails
- **THEN** the hang-guard restarts iroh-relay

#### Scenario: Relay fails to start
- **WHEN** /healthz does not respond within 15 seconds after start
- **THEN** the installer prints recent logs and exits with an error

### Requirement: Caddy TLS terminator
The system SHALL, through the relay installer, install Caddy from the official apt repository and write a Caddyfile that serves the domain with automatic ACME TLS and reverse-proxies to 127.0.0.1:3340, with an optional ACME email.
Evidence: docs/install-relay-debian.sh:330-352, docs/self-hosted-relay.md:50-52, docs/self-hosted-relay.md:251-271

#### Scenario: Install with domain
- **WHEN** the installer completes with --domain relay.example.com
- **THEN** the Caddyfile proxies relay.example.com to 127.0.0.1:3340

### Requirement: Firewall guidance or application
The system SHALL, through the relay installer, print firewall instructions by default and SHALL apply ufw rules (SSH, 80/tcp, 443/tcp, and QUIC udp when enabled, default deny incoming) only with --apply-firewall; ports 3340 and 9090 need no rules.
Evidence: docs/install-relay-debian.sh:354-377, docs/self-hosted-relay.md:273-283

#### Scenario: Without --apply-firewall
- **WHEN** the installer completes
- **THEN** the ufw commands are printed but not executed

#### Scenario: With --apply-firewall and --enable-quic
- **WHEN** both flags are passed
- **THEN** ufw allows the SSH port, 80/tcp, 443/tcp, and the QUIC udp port, then is enabled

### Requirement: Installer summary
The system SHALL, through the relay installer, print a summary with the relay URL, local and public health commands, metrics command, QAD state, access mode (and token when enabled), and client wiring hints.
Evidence: docs/install-relay-debian.sh:379-401, docs/self-hosted-relay.md:331-341

#### Scenario: Token mode
- **WHEN** the installer completes with --enable-token
- **THEN** the summary shows "token required" and the token value

### Requirement: Client configuration for self-hosted relay
The system SHALL document that serve and access `[node]` configs point at the relay with `relay_urls` and, for token-enforcing relays, a matching `relay_token`, and that other Rust clients use `RelayMap::from_iter(..).with_auth_token(..)` with `RelayMode::Custom`.
Evidence: docs/self-hosted-relay.md:293-329, src/endpoint.rs:177-203

#### Scenario: Open relay
- **WHEN** the relay is open
- **THEN** relay_token may be omitted from the node configuration
