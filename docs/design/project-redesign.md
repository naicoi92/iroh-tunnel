# Project redesign: interview decisions

This is a working record of the confirmed design interview, not an implementation specification. Detailed design remains open where noted.

## Confirmed directions

- Use a DDD-oriented modular monolith with domain, application, and infrastructure boundaries. Do not introduce network services merely to split the executables.
- Ship three standalone executables: `iroh-tunnel` contains both roles; `iroh-tunnel-serve` and `iroh-tunnel-access` contain their respective roles. Provide aliases `it`, `its`, and `ita`.
- Permit breaking changes to CLI, configuration, and wire protocol. Independently versioned peers interoperate when their protocol versions are compatible; reject incompatible peers clearly without silent downgrade.
- Upgrade Iroh to the newest stable version, currently 1.4.1 as verified on crates.io. Assess API and dependency changes during implementation.
- Forward TCP and UDP to configured fixed services. TCP is the default CLI protocol. UDP is best-effort, preserves datagram boundaries, and automatically routes responses to the correct concurrent client.
- UDP mapping uses an opaque payload, an Access client-to-flow association and one Serve-side UDP socket per active flow. Socket lifetime bounds endpoint stability; expiry, reconnect, late-packet handling and resource policies remain open. This does not promise original client IP preservation or reliable delivery.
- Multiplexing is always enabled. Ignore the legacy `multiplex` configuration field silently and remove it on subsequent configuration writes. Legacy peers may no longer work.
- Provide quick execution directly via role flags and separate `run` commands for configuration files. Quick execution defaults to an ephemeral identity, with an option for a persistent key.
- Add short options for convenient CLI usage, with consistent meanings and complete help.
- Each executable updates independently. Publish separate packages so package managers can update one executable without upgrading the others; guide users through migration from any bundled legacy package.
- Default to the latest stable release, with an option to select a version. Verify release signatures and checksums; an explicit unsafe bypass is allowed. On verification failure without bypass, retain the existing binary. Do not silently interrupt running tunnels; offer stop/restart guidance or confirmation.
- Use keyless signing with a pinned CI issuer and identity. Publish three images: all-in-one, serve, and access. Update containers by replacing the image, not by self-updating within it.
- Provide shell and PowerShell installers. Install all three by default, system-wide by default; confirm before changing PATH or overwriting existing files.
- Preserve currently released platforms and add Windows x64 and ARM64. When installing a service, a non-elevated user is prompted to choose user or system scope; if system scope is selected interactively or with `--system` without sufficient privileges, stop and instruct the user to rerun with `sudo` on Unix or in an elevated Administrator terminal on Windows. Do not elevate automatically or fall back to user scope. Preserve the intended arguments and resolved configuration/state paths in the guidance. Effective root or an elevated Windows Administrator defaults to system scope without a scope prompt. A user-scope target must be identified unambiguously. Skipping the scope prompt does not skip safety confirmations, and does not change the pinned absolute configuration and state paths. This service-scope policy does not change the binary installer's default. On Windows, regular-user background execution must work while logged in; its mechanism remains to be selected.
- A peer knowing the Serve NodeId and service name can access the service. NodeId is a public identity, not an authorization secret; the service must provide its own access protection.

## Decisions from issue review

- Superseding the earlier proposal for a separate wire-version framework: the user does not want expanded version negotiation in this redesign. Preserve TCP compatibility with 0.4 where feasible and test both old/new peer directions. If either fails, present the trade-off for user decision rather than silently approving a breaking release. Existing code documents an unchanged service ALPN and no version negotiation; it does not prove redesigned interoperability (#75).
- A TCP target failure closes only the affected channel. Peer loss closes the affected channels; bytes are not replayed automatically. New clients may establish a new connection with retries and backoff bounded by a default deadline; on expiry report failure. On shutdown, stop accepting new clients, allow existing channels to drain for a bounded period, then close remaining channels. Exact retry and drain deadlines, backoff and configurability remain open (#76).
- Foreground and user service instances of the same role and user share ownership scope. Changing config or state paths must not bypass exclusivity. Same Iroh secret implies the same public EndpointId; concurrent use can interfere through relay routing. Independent local endpoints using the same identity must not run concurrently, including across roles and user/system scopes; reject the later process and guide the operator to another key. Do not expand this redesign to intervene across hosts; a local lock cannot guarantee detection on another host. System scope, cross-user lock permissions and stale status remain open (#77).

- The user may edit managed configuration and an authorized administrator may restart the system service using sudo to apply changes. Treat those administrative actions as trusted; do not require immutable or root-only config merely to prevent legitimate edits. Validate configuration and report errors without silently changing permissions or runtime account. Privileged-service trust design must address unauthorized writers, parent directories, symlinks, file replacement and TOCTOU (#79).
- The updater must not automatically stop a running tunnel; provide operator guidance. Replacement and rollback details remain open (#80).
- UDP replies must return to the correct Iroh peer and client flow. NodeId identifies an endpoint, not each client behind Access; target response-source filtering remains open (#71).

## Configuration and status locations

- On macOS and Linux, user defaults are `~/.config/iroh-tunnel` for configuration and `~/.local/state/iroh-tunnel` for status; ignore XDG directory variables on both platforms. Explicit application overrides remain available, including `-c` and `IROH_TUNNEL_STATE_DIR`.
- On Windows, user configuration and status default to `LocalAppData/iroh-tunnel/config` and `LocalAppData/iroh-tunnel/state`; resolve the Windows known folder rather than hardcoding a profile path.
- On all platforms, including Windows system services, service installation binds absolute configuration and status paths to the installing user's managed configuration and state directories, subject to service-account permissions. This supersedes the earlier proposed `ProgramData` system-service default. Installing a system service over an existing service targeting another user requires confirmation and changes the service target without modifying either user's configuration.
- If the original user cannot be reliably identified when installing via `sudo`, offer an interactive selection of configuration. Without an interactive terminal, stop with a clear error and instructions to specify paths. Trusted identity resolution remains a detailed design concern.
- Reject concurrent instances of the same role in the same scope rather than allowing status files to overwrite each other. Different roles remain independent; scope identity and locking require detailed design.
- If the new macOS configuration exists, use it without checking the legacy location. Otherwise, copy an existing legacy configuration into the new location. Never overwrite the destination, including under concurrent migration; preserve the original and protect secret-key permissions. Report copy failures clearly and do not generate a new identity as a substitute for the old configuration. Exact legacy locations require verification.
- If an account's home or state directory is unusable, or the service lacks required access, stop with a clear error and guidance instead of silently using another account's home or automatically broadening permissions.

## Confirmed CLI direction

```text
it serve [quick-run flags]       its [quick-run flags]
it access [quick-run flags]      ita [quick-run flags]
it serve run --config …          its run --config …
it access run --config …         ita run --config …
<role> config … | service … | status
<binary> update …
```

Suggested short options, subject to consistency and conflict checks: `-n` name, `-t` target, `-l` listen, `-N` node ID, `-p` protocol, `-c` config, `-k` key. Reserve `-h` for help, `-V` for version, and `-v` for verbosity. Exact names and conflicts require review; the unsafe verification bypass remains explicit and long-form.

## Deferred decisions and verification gates

- Benchmark UDP transport and payload limits using Iroh 1.4.1 before choosing QUIC DATAGRAM framing or fragmentation. Preserve the agreed best-effort contract; see [UDP transport research](../research/udp-transport.md).
- Define exact platform matrix from release configuration and verify Windows execution and service lifecycle on suitable CI runners, including ARM64 limitations.
- Specify update rollback, verification trust rotation, package-manager behavior, platform privilege handling, and precise CLI arguments before implementation.
- The [target architecture dossier](target-diagrams.html) maps redesign requirements to system context, component boundaries, runtime, protocol, storage, service, operating-system, distribution, and security views. The [capability and logic dossier](capability-logic.html) records diagram selection, inputs, branches, outcomes and deferred decisions for each capability. Exact ports, crate boundaries and the deferred topics above remain open. The separate [interactive overview](system-overview.html) is a simplified sketch, not the full architecture.

## Detailed design review draft

The [redesign plan](redesign-plan.md) summarizes implementation phases and decision gates. The [approved capability map](capability-map.md) is the scope boundary. The [capability specifications](redesign-spec.md) and [detailed design entrypoint](detailed-design.html) are review drafts; proposed rules and data cardinalities are not approved implementation contracts. The dossier links executable delta, scenario sequences, operational decisions, data models, planned work census, and [dependency, UML and story views](structural-views.html). UML interfaces and story release cuts are proposed. UDP transport, trust boundaries, ownership mechanics, Windows user-background and release recovery remain open.

## Diagram validation

- The architecture dossier contains 15 figures with navigation. The capability dossier adds a selection matrix and four figures. Diagram Design accessibility self-check and geometry checker passed for both dossiers.
- Browser checks confirmed the capability dossier opens at desktop and mobile sizes, its four figures scroll locally on mobile and its local anchors resolve. The architecture dossier's browser check predates its latest revision. Neither complete dossier has received a full perceptual visual review.
- Structural views passed accessibility self-check and geometry checker. Before the final routing revision, desktop/mobile browser DOM checks found no page-wide overflow and all local anchors resolved; perceptual visual review was not performed.
- The detailed design, executable delta, scenario sequence and operational decision pages passed accessibility self-check and geometry checks; the delta-specific structural checker passed. Browser DOM checks at 390 and 1440 pixels showed no page-wide horizontal overflow and no missing local anchors on those four pages. Perceptual visual review was not performed.
- Browser reported one console error in an earlier overview check; its cause was not investigated, so external font availability is not guaranteed and system font fallbacks apply.
