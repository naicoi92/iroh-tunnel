# TCP lifecycle and local identity ownership — evidence and candidates

Status: research and proposed implementation candidates for [TCP lifecycle #76](https://github.com/naicoi92/iroh-tunnel/issues/76) and [ownership #77](https://github.com/naicoi92/iroh-tunnel/issues/77). Numeric settings and implementation mechanisms below are not approved. This note does not establish cross-user or cross-platform lock safety.

## Confirmed operator contract

A target failure affects its own TCP channel; peer loss ends affected channels without replaying old bytes. New clients may retry with backoff until a default bounded deadline. Shutdown stops acceptance, drains active channels for a bounded period, then closes remaining channels. Same-user foreground and user service of the same role share ownership regardless of path overrides. Independent local endpoints with the same public identity must not run concurrently across roles/scopes. No cross-host intervention is required.

## Existing behavior and evidence

The current connection retry loop has no deadline and sleeps between retries; redesign must not assume it already satisfies the confirmed bounded behavior. The existing role skeleton invokes a five-second shutdown helper, but a sleep alone is not evidence that active streams are tracked and drained. Inspect and verify the actual execution path before treating any shutdown behavior as implemented.

[Iroh endpoint documentation](https://docs.iroh.computer/concepts/endpoints) states the endpoint identifier is the public half of its keypair. [Iroh relay PR 3921](https://github.com/n0-computer/iroh/pull/3921) documents relay routing effects when two endpoints with the same ID connect. Its description does not prove identical behavior for every release or path; verify target versions. Never persist or log raw secret material in lock/status artifacts.

On Linux, [`flock`](https://man7.org/linux/man-pages/man2/flock.2.html) is advisory and tied to open file descriptions; a safe design also needs secure shared namespace, file ownership and symlink handling. Windows [kernel object namespaces](https://learn.microsoft.com/en-us/windows/win32/termserv/kernel-object-namespaces) and [synchronization security](https://learn.microsoft.com/en-us/windows/win32/sync/synchronization-object-security-and-access-rights) show that cross-session scope and access control must be designed explicitly. Do not infer a portable machine-wide lock simply from an API name. macOS and Windows behavior must be validated independently.

## Candidate packet — not approved

- A monotonic total retry deadline bounds connection establishment for a new client, including in-flight dial and backoff; cancellation and role shutdown interrupt both. Report deadline exhaustion without replay. A bounded exponential backoff with jitter is a candidate; numeric defaults require measurement and review.
- Shutdown closes listeners first; it tracks in-flight channels, permits completion until the drain deadline, cancels the remainder and closes the endpoint. A repeated stop policy is unresolved. Existing five-second delay is not an approved timeout.
- Two separate ownership domains are needed: role/user-scope exclusivity and local public EndpointId exclusivity across roles/scopes. Do not derive either from config/state override paths. A lock acquisition failure or unsafe namespace should fail closed with actionable diagnostics rather than claim exclusivity.
- Secure host-wide coordination across users, elevation and services is not yet proven. Threat-model permissions, object squatting, symlinks, PID reuse, crashes and trust of the service identity. An unprivileged process must not falsely claim another owner or bypass the rule. Do not expose secrets.

## Verification gates

Use real processes with same/different roles and identities, foreground/user/system scope, path overrides, concurrent start, crash and restart; test cross-user/elevated permissions on actual Linux, macOS and Windows. Verify stale status and safe cleanup. TCP E2E needs real target failure, peer loss, bounded dial, cancellation, active transfer at shutdown, drain expiry and no replay. Defaults and viable coordination mechanisms require measured evidence and explicit review before closing #76 or #77.
