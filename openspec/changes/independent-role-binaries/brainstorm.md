## Design Summary

The Serve-only `its` (`iroh-tunnel-serve`) and Access-only `ita` (`iroh-tunnel-access`) executables must each work independently, without the combined `iroh-tunnel` binary. Each exposes only its own role's operations: `its` corresponds to `it serve` and `ita` to `it access`. Each also provides applicable auxiliary commands such as its own update, help and version. Neither exposes the other role's operations. The combined binary retains both roles.

Standalone role binaries must support their respective service-management operations on the same platforms and scopes already supported for that role, while preserving existing privilege, configuration, ownership and executable-trust protections. Each binary's update acts on that binary and retains existing verification and installation-owner policies. Homebrew must allow each binary to be installed independently, with its matching alias, and allow the three installations to coexist.

This changes the existing requirement to refuse standalone service management in `openspec/specs/cli-entrypoints/spec.md` (Standalone binaries refuse service management) and `openspec/specs/system-service-management/spec.md` (Service management refused for standalone role binaries). It extends the single-cask behavior in `openspec/specs/install-release-packaging/spec.md` (Homebrew cask rendering). Existing role isolation and invoked-executable update requirements in `openspec/specs/cli-entrypoints/spec.md`, and verification and ownership rules in `openspec/specs/self-update/spec.md`, remain in force.

## Alternatives Considered

### Alternative A: Complete the existing role executables
- **Approach**: Preserve the combined and role-specific executables and their shared functionality, and complete the missing role-specific service and distribution behavior.
- **Pros**: Preserves the familiar role-specific CLI; avoids duplicating functionality; supports independent deployment.
- **Cons**: Requires careful handling of role-specific service invocation, update and packaging behavior.
- **Why chosen**: It achieves the requested independence while retaining the existing product structure.

### Alternative B: Separate products with independent implementations
- **Approach**: Build and release each role as a separately implemented product.
- **Pros**: Strong source-level separation between roles.
- **Cons**: Duplicates functionality and maintenance without being necessary for independent operation.
- **Why not chosen**: Source-level separation is not the user's goal.

## Agreed Approach

Use Alternative A. Maintain a single role vocabulary and make the existing standalone executables independently usable for their own role, including service management, updates, and independently installable Homebrew distribution. Do not add cross-role commands to either standalone executable.

## Key Decisions

- `ita` maps to Access operations and `its` maps to Serve operations; neither is a replacement for the combined binary's other role.
- Standalone service management must not require the combined executable, and must preserve the existing safety guarantees and platform limitations.
- Update affects only the invoked executable. For Homebrew-owned installations the default remains guidance to use Homebrew; explicit `--yes` retains the existing warned override. Container update remains image redeployment rather than in-place replacement.
- Homebrew installations for the three executables must be independently installable and co-installable, including their respective aliases.
- Verify independent operation with other project binaries absent, including role-specific service and update behavior; cover release and Homebrew installation, coexistence and removal.
- This change does not extend a role's protocol features or add the opposite role to a standalone executable.

## Open Questions

No product-scope questions remain. Platform-specific implementation and test mechanics are to be resolved in the subsequent artifacts without weakening existing guarantees.
