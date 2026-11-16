## 1. Standalone service lifecycle

Blocked by: none

- [ ] 1.1 Add failing tests for isolated standalone role service dispatch, registration argument forms, trusted managed copies and unchanged combined behavior in `tests/role_binaries.rs`, service backend tests and `tests/service_user.rs`.
- [ ] 1.2 Carry invocation identity through `src/app.rs`, `src/cli_service.rs`, `crates/host/src/service/` and backend registration, retaining trust and role policies.
- [ ] 1.3 Cover Windows standalone SCM startup and shutdown using `src/scm_runtime/` and `tests/service_system_windows.rs`; verify Unix systemd, BusyBox and launchd registration paths and platform lifecycle tests.

## 2. Role-specific update guidance

Blocked by: none

- [ ] 2.1 Add failing isolated role update and Homebrew ownership guidance tests in `tests/update.rs`, `tests/update_ownership.rs`, and Windows locked-update tests.
- [ ] 2.2 Update `crates/host/src/update/` guidance for the invoked binary and preserve signature, checksum, ownership and container behavior.

## 3. Independent Homebrew casks

Blocked by: none

- [ ] 3.1 Add renderer and release-pipeline tests for separate archive checksums, aliases, cask coexistence and independent uninstall.
- [ ] 3.2 Update `packaging/homebrew-cask.sh` and `.github/workflows/release.yml` to render and publish all three role-specific casks.

## 4. Documentation and verification

Blocked by: 1, 2, 3

- [ ] 4.1 Update `README.md` and `docs/install.md` and relevant service/update guidance for independent role installations and ownership policies.
- [ ] 4.2 Run focused tests, project validation, and available Linux, macOS and Windows service/release tests; record unverified platform coverage explicitly.
