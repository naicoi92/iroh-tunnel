# Decision review packet — UDP, TCP, ownership, service and update

These are design decisions in progress. Confirmed choices are user decisions; proposed mechanisms and numeric defaults are not approved. The live GitHub issues and their comments carry the discussion and acceptance criteria. No implementation or benchmark is claimed.

## UDP routing — #71

**Confirmed:** UDP payload is opaque; Access associates each client with a flow; Serve has a dedicated upstream socket per active flow. Responses must reach the correct Iroh peer and client flow. UDP remains best-effort.

**Proposed:** namespace a flow by authenticated peer/session, service and flow identity. Return data via its owning session and then the Access client association. Do not trust an identity claimed inside arbitrary application payload.

**Evidence:** [mapping research #70](https://github.com/naicoi92/iroh-tunnel/issues/70) and [transport research #72](https://github.com/naicoi92/iroh-tunnel/issues/72). NodeId identifies an endpoint, not each client. Two clients on one Access share NodeId; two Access endpoints can independently use the same flow number.

**Remaining decisions:** session reconnect and lifetime, source filtering when target replies from another endpoint, late upstream replies after tuple reuse. Transport and fragmentation are separate decisions in #73; benchmark has not run.

**Acceptance tests:** two Access endpoints with equal local flow numbers; two clients under one NodeId; interleaved responses; reconnect/session collision; expired and delayed replies; verify no unapproved absolute guarantee when UDP tuples are reused.

## TCP lifecycle — #76

**Confirmed:** target error affects its own channel; peer loss closes affected channels without replay; new clients retry with backoff under a bounded default deadline; shutdown stops acceptance, drains active channels for a bounded period, then closes remaining channels.

**Proposed:** monotonic total retry deadline includes dial and backoff; cancellation interrupts both; track active channels during drain. Jittered bounded backoff is a candidate. An initial illustrative 30-second retry deadline and 5-second drain deadline are unapproved hypotheses, not performance targets; current five-second delay is not evidence that draining works. Measure realistic dial/recovery and shutdown behavior before selecting defaults.

**Evidence:** current connection loop retries indefinitely; existing shutdown footer delays five seconds after role loop. Neither proves the agreed behavior. See [runtime evidence](runtime-gates-research.md).

**Remaining decisions:** numeric defaults, limits, configurability, repeated-stop semantics and error mapping. No values may be silently promoted to confirmed.

**Acceptance tests:** real clients and target; channel failure isolation, peer loss without replay, new-client deadline and cancellation, shutdown with in-flight streams that complete and that outlive the deadline.

## Local role and identity ownership — #77

**Confirmed:** same-user foreground and user service of the same role share ownership regardless of config/state override. Independent local endpoints must not concurrently use the same public identity, including across roles and scopes. No cross-host intervention is required.

**Proposed:** separate role/scope and public-identity ownership domains, derived without raw secrets. Reject a later owner; fail safely if exclusivity cannot be established. Mechanism is not chosen.

**Evidence:** [Iroh endpoints](https://docs.iroh.computer/concepts/endpoints) describe EndpointId as the public key; [Iroh relay PR 3921](https://github.com/n0-computer/iroh/pull/3921) describes duplicate-identity relay interference. Linux [flock](https://man7.org/linux/man-pages/man2/flock.2.html) is advisory; Windows [object namespaces](https://learn.microsoft.com/en-us/windows/win32/termserv/kernel-object-namespaces) and [security](https://learn.microsoft.com/en-us/windows/win32/sync/synchronization-object-security-and-access-rights) require explicit scope and permissions. None proves a secure machine-wide solution across users.

**Remaining decisions:** local system scope identity, cross-user permission/coordination model, anti-squatting, crash recovery and stale status. A local lock cannot prevent duplicate key use on another host.

**Acceptance tests:** real processes, same/different users, roles and scopes, key/path overrides, racing starts, crash/restart, PID reuse and unauthorized lock manipulation on supported OSes.

## Service trust and Windows user-background — #79

**Confirmed:** a user may edit managed config; an authorized administrator may restart a system service with sudo to apply edits. Pin paths, report invalid configuration, and do not silently change account, ownership or permissions.

**Proposed:** compare system registration with runtime running as the intended unprivileged user against a privileged runtime with strict, limited config semantics and secure path handling. A system registration is not itself proof of root runtime. Account choice is not decided; feasibility differs by OS.

**Evidence:** [systemd execution settings](https://freedesktop.org/software/systemd/man/latest/systemd.exec.html), [Apple launchd guidance](https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPSystemStartup/Chapters/CreatingLaunchdJobs.html) and [Windows service accounts](https://learn.microsoft.com/en-us/windows/win32/services/service-user-accounts) describe platform account mechanisms, not proof that existing adapters implement them securely.

**Remaining decisions:** runtime account, trusted user identification, directory and file permissions, symlink and replacement/TOCTOU handling; Windows logged-in user-background mechanism. Protect against unauthorized writers without rejecting legitimate administrator-approved edits.

**Acceptance tests:** user edit followed by authorized restart, invalid config with clear diagnostics, unauthorized user/process, parent-directory replacement, symlink and race attempts, real OS account privileges and Windows background lifecycle.

## Update recovery — #80

**Confirmed:** verify pinned CI identity/signature and checksum; an explicit unsafe bypass warns. Do not automatically stop a running tunnel; guide the operator. Failed verification without bypass keeps the installed binary.

**Proposed:** download to a staging area and verify before replacement. If OS or running-file constraints prevent safe replacement, stop with guidance and keep the old binary. This is a candidate, not an approved rollback mechanism.

**Evidence:** design decisions in [spec #68](https://github.com/naicoi92/iroh-tunnel/issues/68); actual replacement and recovery behavior across platforms has not been benchmarked or implemented.

**Remaining decisions:** atomic replacement, rollback/partial failure, signing identity rotation, exact checksum bypass and running binary behavior on Windows. No automatic service restart is implied.

**Acceptance tests:** running process is not stopped, verification failure leaves old binary intact, explicit bypass warns, replacement failure/recovery, independent binary selection and real OS-specific file behavior.
