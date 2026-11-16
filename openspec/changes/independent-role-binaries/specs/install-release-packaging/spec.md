## MODIFIED Requirements

### Requirement: Homebrew cask rendering
The system SHALL render and publish separate Homebrew casks for `iroh-tunnel`, `iroh-tunnel-serve` and `iroh-tunnel-access`. Each SHALL download its own `darwin-arm64` archive with its own checksum, require arm64, link only its executable and matching alias (`it`, `its` or `ita`), clear quarantine on its executable and remove its own alias on uninstall without affecting the other casks.

#### Scenario: Render
- **WHEN** the cask renderer is given a supported executable, version and checksum
- **THEN** it prints the matching cask with that version, checksum and release URL and rejects missing required arguments

#### Scenario: Independent installation and removal
- **WHEN** any cask is installed alone or all three are installed together and one is uninstalled
- **THEN** its executable and alias work while installed and removal leaves the other installed executables and aliases intact

#### Scenario: Release publication
- **WHEN** a stable release is published
- **THEN** the tap receives all three casks using their corresponding release archive checksums
