## Context

CLI dispatch currently rejects standalone service actions. Service installation resolves `iroh-tunnel` from PATH, backends generate combined-binary argument forms, and Windows SCM parses the combined CLI. Homebrew renders and publishes only the combined cask; Homebrew update guidance currently names it for every executable.

## Goals / Non-Goals

**Goals:** Preserve the existing CLI and host management boundary while carrying executable invocation identity through registration, runtime and update guidance. Keep each role independently deployable and service-capable.

**Non-Goals:** Rewrite service backends, alter transport behavior, weaken trust checks, or add the other role to standalone CLIs.

## Decisions

- Carry an explicit combined-versus-role-specific invocation form with installation inputs and registration generation. Resolve the running standalone executable directly, not a combined binary from PATH. Generate the matching argument form for systemd, BusyBox, launchd, Windows Task Scheduler and SCM, including managed copies; preserve existing combined registrations. Explicit identity avoids inferring behavior from a renamed or copied path. Adapt public host request contracts only as needed and retain explicit executable inputs.
- Route Windows SCM startup through the matching role parser and dispatcher so the registered standalone binary can enter service context. Preserve existing readiness, stop, identity and trust policy.
- Render and publish one cask per existing Darwin role archive and checksum, each owning only its own binary and alias. Ensure independent uninstall and co-installation; maintain the existing combined cask name. Generate role-specific Homebrew upgrade guidance using invoked executable identity, preserving ownership and confirmation rules.
- Exercise role services, standalone update assets and cask rendering through isolated tests, with real platform lifecycle tests where available; do not assume Linux validates launchd or Windows SCM.

## Risks / Trade-offs

- [Argument form or copied executable identity is lost] → Carry it explicitly and test generated registrations and execution for each backend and managed copy.
- [Windows SCM accepts only the combined parser] → Test standalone service startup and shutdown on Windows in addition to registration generation.
- [One cask removes another cask's alias] → Give each cask distinct, owned paths and test all install/uninstall orderings.
- [Homebrew guidance names the wrong cask] → Test the output for all executable identities.
- [Platform tests unavailable locally] → Record missing coverage and require corresponding CI/platform verification.

## Migration Plan

Existing combined registrations and cask remain valid. Publish role casks alongside the existing cask; users opt in to standalone installations or reinstall their role service. Roll back by reverting the release and using existing combined installations; do not silently rewrite existing registrations.

## Open Questions

None at product scope. Resolve public request shape and platform-specific mechanics during implementation without changing these invariants.
