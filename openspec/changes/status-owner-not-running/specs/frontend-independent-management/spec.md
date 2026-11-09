## MODIFIED Requirements

### Requirement: Service status separates facts without inventing truth
Host service status SHALL report registration/backend facts, kernel-backed role liveness, owner metadata and captured diagnostics as separate fields, with unavailable facts explicit. It MUST NOT derive liveness from manager state or parse localized `systemctl`, `launchctl` or `schtasks` prose. When the role is not running, owner metadata SHALL be reported as known and absent, never as unavailable; unavailable owner metadata MUST mean that it could not be determined.
Source: design.md Decision 4

#### Scenario: Foreground role without a registration
- **WHEN** a Serve role runs in the foreground and no service is registered
- **THEN** status reports the registration as absent and the role as live, without claiming the registration launched it

#### Scenario: Role not running has a known absent owner
- **WHEN** host service status is requested for a role that is not running and the existing required status checks succeed
- **THEN** status reports the role as not running and the owner metadata as known and absent, not as unavailable, and succeeds
