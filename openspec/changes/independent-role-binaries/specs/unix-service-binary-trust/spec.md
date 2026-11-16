## MODIFIED Requirements

### Requirement: Unsafe registered binary refuses CLI start and restart
The system SHALL refuse a CLI start or restart whose registered binary fails the check, or cannot be read from the registration, with a permission error (exit code 3) naming the binary path, and its owner and mode where known, and telling the operator how to re-run service installation using the invoked combined or standalone executable. Combined invocation SHALL retain the `sudo iroh-tunnel <role> service install --system` guidance; standalone guidance SHALL instruct an elevated `service install --system` using the invoked standalone executable, with appropriate path quoting, and SHALL NOT require the combined binary. No start, restart or stop command SHALL be issued, so a running service keeps running.

#### Scenario: Legacy registration with a user-writable binary
- **WHEN** a system registration written before this change runs `/home/alice/.cargo/bin/iroh-tunnel` and `sudo iroh-tunnel serve service start --system` runs
- **THEN** the command exits with code 3, stderr names that path and `service install --system`, and only the read-only definition query was run

#### Scenario: Directory loosened after install
- **WHEN** a directory above the registered binary becomes group-writable after install and `service restart --system` runs
- **THEN** the command exits with code 3 naming that directory, and the running service is neither stopped nor restarted

#### Scenario: Unreadable binary in the registration
- **WHEN** the registration is loaded but its effective definition names no single binary the system can extract
- **THEN** the command exits with code 3 telling the operator to re-run `service install --system`

#### Scenario: Standalone invocation guidance
- **WHEN** a standalone role binary refuses a start or restart due to an unsafe registered binary
- **THEN** the diagnostic identifies the invoked standalone binary and instructs the operator to reinstall the role service with elevation and `--system` without requiring the combined executable
