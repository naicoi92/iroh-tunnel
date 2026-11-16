## Why

Standalone Serve and Access executables currently refuse service management and the Homebrew cask only installs the combined executable. Operators cannot rely on a single role-specific installation for its complete lifecycle or install it independently through Homebrew.

## What Changes

- Replace standalone service refusal with role-specific service lifecycle support independent of the combined executable, preserving existing security and platform policies.
- Publish co-installable Homebrew casks for the combined, Serve and Access executables with their respective aliases.
- Verify independent role operation and invoked-binary update, retaining existing signature, ownership and container policies.
- Preserve existing combined CLI behavior and role isolation; no new protocol features or cross-role commands are in scope.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `cli-entrypoints`: standalone role commands support service management instead of refusing it.
- `system-service-management`: standalone executables can register and manage their own role services without the combined executable.
- `install-release-packaging`: publish independent, co-installable Homebrew casks for all three executables.
- `self-update`: make Windows locked-service guidance usable without the combined executable and direct Homebrew users to the matching cask.

## Impact

Affects CLI dispatch, service registration and platform execution paths, release automation, Homebrew cask rendering, documentation and tests. Existing service configurations and combined CLI behavior remain compatible. Trust, privileges, update verification and managed-install safeguards must not be weakened; standalone service refusal changes to supported behavior.
