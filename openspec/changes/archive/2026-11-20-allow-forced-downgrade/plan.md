# Allow Forced Downgrade Implementation Plan

> Executed via `/opsx-apply`, which chooses the executor.
> Spec reference: this change's `specs/` (`self-update`, `cli-entrypoints`) and `design.md`.

**Goal:** `update --to VERSION` installs any published release (including release candidates) and refuses a downgrade unless `--force` is given, ordering versions by semver precedence.

**Architecture:** The host use case `update(req, ctx)` gains a `DowngradePolicy` in `UpdateRequest` and a semver-based guard that runs before any network I/O. The CLI adds `--force` (requires `--to`), maps it to the policy, and keeps `check_owner` ahead of `update()` so ownership/container outcomes win. Rendering only changes the "newer than latest" hint.

**Tech Stack:** Rust 2021 (MSRV 1.91), clap, tokio, anyhow, `semver` 1.x (already in `Cargo.lock` at 1.0.28).

## Global Constraints

- MSRV 1.91; `semver = "1"` is the only new dependency (user-approved).
- Refusal: exit code 1 (plain `anyhow` error, not `CliError::Config`), before `http_client()`; executable unchanged.
- `--force` without `--to`: clap usage error, exit 2.
- `--force` never bypasses `--insecure`/verification, `--yes`/ownership or container protection; does not reinstall the same version.
- Refusal message (exact): `refusing to downgrade {exe} from {current} to {requested}; pass --force to install it anyway; the installed executable was not changed`.
- Hint (exact): `{exe} {current} is newer than the latest release {latest}; nothing to do (use --to {latest} --force to install it anyway)`.
- Follow `docs/agents/rust-coding-standards.md` (RS-01–RS-16); `cargo clippy --all-targets -- -D warnings` stays clean.

## Review Focus

- `--to v0.8.0-rc.1` with a leading `v` and pre-release: normalized to `0.8.0-rc.1` and used verbatim in URL/archive name.
- Version with build metadata (`--to 0.8.0+build.5`): accepted by parsing; ordering ignores metadata (precedence-equal to `0.8.0`) — not refused against running `0.8.0`, and not "already at" (string differs); it proceeds to download. Pin with a unit test of `is_older`.
- Leading-zero or empty identifiers (`--to 01.2.3`, `--to 1.2.3-`): rejected as "invalid version" before any request.
- Running `0.7.0-dev` (every test binary): `--to 0.7.0` is an upgrade, `--to 0.6.9` is a downgrade.
- Refusal never contacts the server even with `--insecure`.
- Cosmetic, accepted: with `-i` the `--insecure` warning is printed before the refusal error (warning is emitted in `run_update_cli` before `update()`).

---

## Contracts

| Name | new / modified | Shape | public |
|---|---|---|---|
| `iroh_tunnel_host::update::DowngradePolicy` | new | `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum DowngradePolicy { Refuse, Allow }` — rustdoc on the enum and each variant (RS-02): `Refuse`: an explicit `--to` older than the running version fails; `Allow`: installs it | public (crate API) |
| `iroh_tunnel_host::update::UpdateRequest` | modified | adds field `pub downgrade: DowngradePolicy` (doc: "Whether an explicit `to` older than the running version may be installed.") | public (crate API) |
| `update` CLI option `--force` | new | `UpdateArgs { …, #[arg(long, requires = "to")] pub force: bool }`; help: "Allow --to to install a release older than the running one." | public (CLI) |
| `NewerThanLatest` stdout text | modified | hint text per Global Constraints | public (CLI output) |
| `parse_version(raw: &str) -> anyhow::Result<String>` | modified (private) | same signature; validation via `semver::Version::parse` after stripping one leading `v`; error `invalid version {raw:?}; expected e.g. 0.4.2` | no |
| `is_older(version: &str, current: &str) -> bool` | modified (private) | true iff both parse as semver and `version.cmp_precedence(&current) == Less` | no |

---

## Task 1: Host semver ordering and downgrade guard — implements 1

**Covers:** 1.1, 1.2, 1.3, 1.4, 1.5
**Difficulty:** Normal
**Blocked by:** none

**Files:**
- Modify: `crates/host/Cargo.toml`, `Cargo.lock`
- Modify: `crates/host/src/update/types.rs` (add `DowngradePolicy`, field on `UpdateRequest`)
- Modify: `crates/host/src/update/mod.rs` (re-export `DowngradePolicy`)
- Modify: `crates/host/src/update/update.rs` (`parse_version`, `is_older`, guard, unit tests)
- Modify: `src/app.rs` (`run_update_cli` request literal: `downgrade: update::DowngradePolicy::Refuse` — compile only, Task 2 maps the flag)
- Modify: `tests/update.rs` (request literal + new tests), `tests/update_ownership.rs`, `tests/update_zip_windows.rs` (request literals only)

**Interfaces:**
- Produces: `DowngradePolicy`, `UpdateRequest::downgrade` (see Contracts).

- [ ] **Step 1:** Add `semver = "1"` under `[dependencies]` in `crates/host/Cargo.toml`; `cargo check -p iroh-tunnel-host`; confirm `Cargo.lock` still resolves `semver 1.0.28` (no new crate).
- [ ] **Step 2:** Add `DowngradePolicy` and `UpdateRequest::downgrade` per Contracts; re-export in `mod.rs`; add `downgrade: DowngradePolicy::Refuse` to every existing literal (`src/app.rs:304`, `crates/host/src/update/update.rs` test, `tests/update.rs` `args()`, `tests/update_ownership.rs` `run()`, `tests/update_zip_windows.rs`). Run `cargo test -p iroh-tunnel-host update` and `cargo test --test update --test update_ownership`: PASS (no behaviour change yet, all existing callers are not downgrading: in-process `current_version` is `0.0.1`).
- [ ] **Step 3: Failing unit tests** in `update.rs` `mod tests`:
  ```rust
  #[test]
  fn parse_version_accepts_semver_with_optional_v() {
      assert_eq!(parse_version("v0.8.0-rc.1").unwrap(), "0.8.0-rc.1");
      assert_eq!(parse_version("0.8.0+build.5").unwrap(), "0.8.0+build.5");
      for bad in ["../x", "01.2.3", "1.2.3-", "1.2.3-..", "1.2.3-rc..1", "1.2", "vv1.2.3", ""] {
          assert!(parse_version(bad).unwrap_err().to_string().contains("invalid version"), "{bad}");
      }
  }

  #[test]
  fn older_uses_semver_precedence() {
      assert!(is_older("0.8.0-rc.1", "0.8.0"));
      assert!(is_older("0.8.0-rc.1", "0.8.0-rc.2"));
      assert!(!is_older("0.8.0-rc.2", "0.8.0-rc.1"));
      assert!(!is_older("0.7.0", "0.7.0-dev"));
      assert!(is_older("0.8.0-beta.1", "0.8.0-dev"));
      assert!(!is_older("0.8.0-rc.1", "0.8.0-dev"));
      assert!(!is_older("0.8.0+build.5", "0.8.0"));
      assert!(is_older("0.4.1", "0.4.2"));
      assert!(!is_older("0.0.1", "garbage"));
      assert!(!is_older("x", "0.4.2"));
  }
  ```
  Replace the old `older_detection_ignores_dev_suffix_and_handles_garbage` (its cases are subsumed). Run `cargo test -p iroh-tunnel-host update::update`: FAIL (the regex accepts `01.2.3`, `1.2.3-..`, `1.2.3-rc..1`; rc/beta ordering is wrong).
- [ ] **Step 4:** Implement `parse_version` with `semver::Version::parse` (strip one leading `v`, return the input remainder unchanged so URLs keep the published spelling) and `is_older` with `cmp_precedence`; drop the `regex` static (keep the `regex` dependency if used elsewhere in the crate — check with `rg 'regex::' crates/host/src`). Run: PASS.
- [ ] **Step 5: Failing guard tests** in `update.rs` `mod tests` using `ctx_with(Owner::Standalone, …, &AcceptAll)` (base URL `http://127.0.0.1:9`, so any network attempt fails with a connect error, not the guard message):
  - `explicit_downgrade_is_refused_before_any_request`: `ctx.current_version = "0.8.0"`, `to = Some("0.8.0-rc.1")`, `Refuse`, `Verification::Insecure` → error string contains `refusing to downgrade`, `0.8.0`, `0.8.0-rc.1`, `--force`; install file still `b"OLD"`.
  - `older_release_candidate_is_refused`: current `0.8.0-rc.2`, `to 0.8.0-rc.1` → same assertions.
  - `same_version_with_force_is_already_current` (characterization, already passes): current `0.0.1`, `to 0.0.1`, `Allow`, `Insecure` → `Ok(AlreadyCurrent { version: "0.0.1" })`, no network (127.0.0.1:9).
  Run: the two refusal tests FAIL.
- [ ] **Step 6:** Implement the guard in `update()` right after `requested` is parsed and before `http_client()`: if `req.downgrade == DowngradePolicy::Refuse` and `requested` is `Some(v)` with `is_older(v, ctx.current_version)` → `bail!` the exact message from Global Constraints. Run: PASS.
- [ ] **Step 7: Integration tests** in `tests/update.rs` (in-process, `ReleaseServer`, `in_process` helper extended to take `current_version: &str`):
  - `forced_downgrade_installs_older_release`: publish `0.6.0`, current `0.7.0`, `to 0.6.0`, `Allow`, insecure → installed `NEW-0.6.0`.
  - `newer_release_candidate_installs_without_force`: publish `0.8.0-rc.2`, current `0.8.0-rc.1`, `Refuse`, insecure → `NEW-0.8.0-rc.2`.
  - `release_candidate_can_be_pinned`: publish `9.9.9-rc.1` (latest `9.9.9`), current `0.0.1`, `to 9.9.9-rc.1`, insecure → `NEW-9.9.9-rc.1`.
  - `forced_downgrade_keeps_verification`: publish `0.6.0`, current `0.7.0`, `Allow`, verified with the `Reject` verifier → error chain contains `signature verification failed`, file `OLD_BYTES`.
  - `latest_upgrades_dev_build_to_its_release` (regression): own `ReleaseServer` with latest `0.7.0`, fresh `old_install()`, current `0.7.0-dev` → `NEW-0.7.0`.
  - `latest_upgrades_release_candidate_to_its_release` (regression): own `ReleaseServer` with latest `0.8.0`, fresh `old_install()`, current `0.8.0-rc.2` → `NEW-0.8.0`.
  - `unordered_running_version_installs_explicit_target`: publish `9.9.8`, current `garbage`, `Refuse`, insecure, `to 9.9.8` → installed `NEW-9.9.8`.
  Run `cargo test --test update`: PASS (Steps 4/6 implemented the behaviour; if any fails, fix before continuing).
- [ ] **Step 8:** `cargo clippy --all-targets -- -D warnings` and `cargo test -p iroh-tunnel-host`: clean.
- [ ] **Step 9: Commit** `feat(update): order versions by semver and refuse explicit downgrades without force`.

---

## Task 2: CLI `--force`, mapping and hint — implements 2

**Covers:** 2.1, 2.2, 2.3, 2.4, 2.5
**Difficulty:** Easy
**Blocked by:** Task 1

**Files:**
- Modify: `src/cli.rs` (`UpdateArgs`)
- Modify: `src/app.rs` (`run_update_cli`)
- Modify: `src/render.rs` (`render_update` + unit test)
- Modify: `tests/update.rs`, `tests/update_ownership.rs`

**Interfaces:**
- Consumes: `DowngradePolicy`, `UpdateRequest::downgrade` from Task 1.
- Produces: CLI option `--force`, hint text (Contracts).

- [ ] **Step 1: Failing real-binary tests** in `tests/update.rs` (binary runs as `<cargo version>-dev`, newer than `0.0.1`):
  - `force_without_to_is_a_usage_error`: `run_update(&exe, "http://127.0.0.1:9", &["--force"])` → `status.code() == Some(2)`; the first stderr paragraph (`stderr.split("\n\n").next()`, as in `tests/cli_usage.rs:23`) contains `--to` (the full stderr always includes help listing `--to`); file unchanged.
  - `explicit_downgrade_exits_1_and_names_force` (already passes after Task 1; pins the CLI contract): `run_update(&exe, "http://127.0.0.1:9", &["--to", "0.0.1"])` → code `Some(1)`, stderr contains `0.0.1`, `--force`, `refusing to downgrade`; file unchanged. (127.0.0.1:9 proves no request: a connect attempt would produce a different error.)
  - `forced_explicit_downgrade_installs_the_older_build`: skip when `host_target()` is `None`; `publish(releases, "0.0.1", target, false)`, `ReleaseServer::start(.., "9.9.9")`, `run_update(&exe, &server.url, &["-i", "--to", "0.0.1", "--force"])` → success and `run_installed(&exe) == "NEW-0.0.1"`.
  Run: `force_without_to_is_a_usage_error` and `forced_explicit_downgrade_installs_the_older_build` FAIL (unknown argument `--force`).
- [ ] **Step 2:** Add `UpdateArgs::force` per Contracts; in `run_update_cli` set `downgrade: if args.force { Allow } else { Refuse }`. Run: PASS.
- [ ] **Step 3:** In `tests/update_ownership.rs` add real-binary tests (ownership runs before `update()`):
  - `real_binary_in_a_container_gives_redeploy_guidance_before_the_downgrade_guard`: copy of `real_binary_in_a_container_prints_redeploy_guidance_and_changes_nothing` with args `["update", "--to", "0.0.1"]` → success, stdout contains `redeploy`, file unchanged.
  - `force_does_not_override_homebrew_ownership`: `HomebrewInstall::new(SERVE, "iroh-tunnel-serve")`, `.command("http://127.0.0.1:9").args(["--to", "0.0.1", "--force"])` (follow the existing `real_homebrew_binaries_delegate_to_their_own_casks` call shape) → exit 0, stdout contains `brew upgrade` and `--yes`, file unchanged.
  Run `cargo test --test update_ownership`: PASS.
- [ ] **Step 4: Failing render test** in `src/render.rs` `mod tests`: `render_update(&UpdateOutcome::NewerThanLatest { current: "0.1.0".into(), latest: "0.0.5".into() }, "iroh-tunnel")` equals `"iroh-tunnel 0.1.0 is newer than the latest release 0.0.5; nothing to do (use --to 0.0.5 --force to install it anyway)\n"`. Run: FAIL. Update the format string. Run: PASS. Then add real-binary `newer_than_latest_hint_suggests_force` in `tests/update.rs`: skip when `host_target()` is `None`; `publish(.., "0.0.5", target, false)`, `ReleaseServer::start(.., "0.0.5")`, `run_update(&exe, &server.url, &["-i"])` → success, stdout contains `--to 0.0.5 --force`, file unchanged.
- [ ] **Step 5:** Run `cargo test --test update --test update_ownership --test cli_golden` and confirm `short_flag_and_explicit_version_select_that_release` and `unknown_version_fails_and_keeps_installed_binary` pass unchanged (9.9.8 / unpublished versions are newer than the running `-dev` build).
- [ ] **Step 6:** `cargo clippy --all-targets -- -D warnings`; full `cargo test`: clean.
- [ ] **Step 7: Commit** `feat(cli): add update --force for explicit downgrades`.

---

## Task 3: Docs and changelog — implements 3

**Covers:** 3.1, 3.2
**Difficulty:** Easy
**Blocked by:** none

**Files:**
- Modify: `docs/cli.md` (`## update` synopsis `iroh-tunnel update [--to VERSION [--force]] [-i] [-y]`, the `--to VERSION` row that says "allows downgrades" (~line 123), and a new `--force` row)
- Modify: `docs/install.md` (the `update` examples and the "does not downgrade unless you pass `--to`" bullet)
- Modify: `CHANGELOG.md` (`## [Unreleased]`)

- [ ] **Step 1:** `docs/cli.md`: document `--force` ("allow `--to` to install a release older than the running one; requires `--to`") and that `--to` accepts pre-releases such as `0.8.0-rc.1`.
- [ ] **Step 2:** `docs/install.md`: replace the bullet with "does not downgrade unless you pass `--to VERSION --force`"; add example `iroh-tunnel update --to 0.8.0-rc.1  # a release candidate`.
- [ ] **Step 3:** `CHANGELOG.md` under `[Unreleased]`: `### Breaking` — "`update --to VERSION` refuses a version older than the running one; add `--force` to downgrade." `### Added` — "`update --force`; `update --to` accepts release candidates and orders versions by semver precedence."
- [ ] **Step 4: Commit** `docs: document update --force and release candidates`.

---

## Scenario → test

**self-update**
- Explicit version → `tests/update.rs::short_flag_and_explicit_version_select_that_release` (existing, Task 2 Step 5)
- Release candidate → `tests/update.rs::release_candidate_can_be_pinned`
- Unknown version → `tests/update.rs::unknown_version_fails_and_keeps_installed_binary` (existing)
- Path traversal value → `tests/update.rs::invalid_version_is_rejected_before_any_request` (existing) + `update.rs::parse_version_accepts_semver_with_optional_v`
- Pre-release value accepted → `update.rs::parse_version_accepts_semver_with_optional_v`
- Latest older than running → `tests/update.rs::latest_never_downgrades_a_newer_installed_version` (existing) + `tests/update.rs::newer_than_latest_hint_suggests_force` + `render.rs` hint test
- Dev build updates to its release → `tests/update.rs::latest_upgrades_dev_build_to_its_release` + `update.rs::older_uses_semver_precedence`
- Release candidate updates to its release → `tests/update.rs::latest_upgrades_release_candidate_to_its_release`
- Same version with force → `update.rs::same_version_with_force_is_already_current` + `tests/update.rs::same_version_with_force_reports_already_current`
- Unordered running version → `tests/update.rs::unordered_running_version_installs_explicit_target` (in-process; defensive, `build.rs` always emits semver)
- Downgrade refused → `tests/update.rs::explicit_downgrade_exits_1_and_names_force` + `update.rs::explicit_downgrade_is_refused_before_any_request`
- Release candidate below running release refused → `update.rs::explicit_downgrade_is_refused_before_any_request`
- Older release candidate refused → `update.rs::older_release_candidate_is_refused`
- Newer release candidate installed → `tests/update.rs::newer_release_candidate_installs_without_force`
- Forced downgrade → `tests/update.rs::forced_downgrade_installs_older_release` + `tests/update.rs::forced_explicit_downgrade_installs_the_older_build`
- Force keeps verification → `tests/update.rs::forced_downgrade_keeps_verification`
- Force does not override ownership → `tests/update_ownership.rs::force_does_not_override_homebrew_ownership` (Homebrew stands in for package ownership: a real binary cannot fake a deb/apk database, and `check_owner` does not read `DowngradePolicy`)
- Container decision precedes the guard → `tests/update_ownership.rs::real_binary_in_a_container_gives_redeploy_guidance_before_the_downgrade_guard`

**cli-entrypoints**
- All-in-one role update → `tests/cli_usage.rs::all_in_one_role_update_is_refused`
- Standalone update → existing coverage (unchanged)
- Force without a version → `tests/update.rs::force_without_to_is_a_usage_error`
