# Not-Running Owner Fact Implementation Plan

> Executed via `/opsx-apply`, which chooses the executor.
> Spec reference: this change's `specs/` and `design.md` (design defers to `brainstorm.md`).

**Goal:** Report the owner of a role that is not running as `Fact::Known(None)` instead of `Fact::Unavailable("<role> is not running")`.

**Architecture:** `ServiceStatusReport.owner` becomes `Fact<Option<OwnerRecord>>`. The owner mapping in `status_via` moves into a small pure function, `owner_fact`, which turns the liveness probe result into the owner fact. That lets every platform unit-test the mapping without a real lock. No CLI path reads the field, so CLI output does not change.

**Tech Stack:** Rust 2021 (MSRV 1.91), `anyhow`, cargo test.

## Global Constraints

- Rust standards RS-01–RS-16 (`docs/agents/rust-coding-standards.md`); MSRV 1.91.
- Verify with `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features --locked -- -D warnings`, `cargo test --all-features --locked` and `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked` (the public rustdoc changes).
- CLI goldens in `tests/cli_golden.rs` must pass unchanged; do not edit golden files.
- Owner metadata that cannot be determined stays `Fact::Unavailable`; a not-running role is never `Unavailable` for its owner.

## Review Focus

- A running role with a readable owner record must still report `Known(Some(record))`. No current test pins this, so Step 1 adds `owner_fact_is_the_record_when_running`.
- A probe error must report owner `Unavailable` carrying the error text, not `Known(None)`: a failed probe does not prove the role is not running. Step 1 adds `owner_fact_is_unavailable_when_the_probe_fails`.
- A running role with no owner record (the record was deleted after the claim) must stay `Unavailable`. The existing test `status_reports_missing_owner_as_unavailable` covers it end to end, and `owner_fact_is_unavailable_when_running_without_a_record` covers it at unit level.

---

## Contracts

| Name | new / modified | Shape | public |
|---|---|---|---|
| `ServiceStatusReport.owner` (`crates/host/src/service/types.rs`) | modified | `pub owner: Fact<Option<crate::ownership::OwnerRecord>>`. `Known(None)` = role not running; `Known(Some(r))` = running with record `r`; `Unavailable(reason)` = running but record missing or unreadable, or liveness probe failed | yes |
| `owner_fact` (`crates/host/src/service/status.rs`) | new | `fn owner_fact(liveness: &anyhow::Result<crate::ownership::Liveness>) -> Fact<Option<crate::ownership::OwnerRecord>>` | no (private) |

---

## Task 1: Not-running owner is known and absent — implements 1

**Covers:** 1.1, 1.2, 1.3, 1.4, 1.5
**Difficulty:** Easy
**Blocked by:** none

**Files:**
- Modify: `crates/host/src/service/types.rs` (`ServiceStatusReport.owner` field and rustdoc, ~line 106)
- Modify: `crates/host/src/service/status.rs` (`status_via` owner match ~lines 41-52; new `owner_fact`; `mod tests`)

**Interfaces:**
- Consumes: `crate::ownership::{Liveness, OwnerRecord}`, `Fact` (existing).
- Produces: contracts `ServiceStatusReport.owner` and `owner_fact` above.

- [ ] **Step 1: Write the failing tests** in `mod tests` of `crates/host/src/service/status.rs`.

  Unit tests (all platforms; `Liveness`, `OwnerRecord` and `Tier` are already imported):
  ```rust
  fn record() -> OwnerRecord {
      OwnerRecord { role: "serve".to_owned(), pid: 42, node_id: None, started_at: 0 }
  }

  #[test]
  fn owner_fact_is_known_absent_when_not_running() {
      let fact = owner_fact(&Ok(Liveness::NotRunning));
      assert!(matches!(&fact, Fact::Known(None)), "{fact:?}");
  }

  #[test]
  fn owner_fact_is_the_record_when_running() {
      let fact = owner_fact(&Ok(Liveness::Running { tier: Tier::User, owner: Some(record()) }));
      assert!(matches!(&fact, Fact::Known(Some(r)) if r.pid == 42), "{fact:?}");
  }

  #[test]
  fn owner_fact_is_unavailable_when_running_without_a_record() {
      let fact = owner_fact(&Ok(Liveness::Running { tier: Tier::User, owner: None }));
      assert!(matches!(&fact, Fact::Unavailable(_)), "{fact:?}");
  }

  #[test]
  fn owner_fact_is_unavailable_when_the_probe_fails() {
      let fact = owner_fact(&Err(anyhow::anyhow!("lock probe boom")));
      assert!(
          matches!(&fact, Fact::Unavailable(m) if m == "lock probe boom"),
          "{fact:?}"
      );
  }
  ```
  `OwnerRecord` does not derive `PartialEq`, so `Fact<Option<OwnerRecord>>` has no `PartialEq`. Every assertion matches on `&fact` with `matches!`. Do not add a derive to `OwnerRecord` for this.

  End-to-end test through `status_via` (Unix, next to `status_reports_missing_owner_as_unavailable`):
  ```rust
  #[cfg(unix)]
  #[test]
  fn status_reports_not_running_owner_as_known_absent() {
      const NAME: &str = "service::status::tests::status_reports_not_running_owner_as_known_absent";
      in_isolated_home(NAME, |_| {
          let report = run_status(vec![]).expect("a stopped role is not a failure");
          assert!(matches!(report.liveness, Fact::Known(Liveness::NotRunning)), "{report:?}");
          assert!(matches!(report.owner, Fact::Known(None)), "{report:?}");
      });
  }
  ```

- [ ] **Step 2: Run the tests to confirm they fail**
  Run: `cargo test -p iroh-tunnel-host --all-features --locked service::status`
  Expected: compile failure because `owner_fact` is not defined. That counts as the red state.

- [ ] **Step 3: Change the contract** in `crates/host/src/service/types.rs`. Replace the field with `pub owner: Fact<Option<crate::ownership::OwnerRecord>>` and give it this rustdoc: "The role's owner, from the owner record: `Known(None)` when the role is not running; `Unavailable` when it runs but the record is missing or unreadable, or liveness could not be probed."

- [ ] **Step 4: Implement `owner_fact`** in `crates/host/src/service/status.rs` (private, below `status_via`), and use it in `status_via`: `let owner = owner_fact(&liveness);`.
  Mapping:
  - `Ok(Running { owner: Some(r), .. })` → `Known(Some(r.clone()))`
  - `Ok(Running { owner: None, .. })` → `Unavailable("the owner record is missing or unreadable")` (the existing text)
  - `Ok(NotRunning)` → `Known(None)`
  - `Err(e)` → `Unavailable(e.to_string())`

  Also check the `status` rustdoc ("A fact that cannot be read is `Fact::Unavailable`") and keep it accurate.

- [ ] **Step 5: Run the targeted tests to confirm they pass**
  Run: `cargo test -p iroh-tunnel-host --all-features --locked service::status`
  Expected: PASS, including the existing `status_reports_missing_owner_as_unavailable`, `status_reports_foreground_role_without_registration` and `status_does_not_parse_native_prose`.

- [ ] **Step 6: Full verification**
  Run: `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features --locked -- -D warnings`, `cargo test --all-features --locked`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked`.
  Expected: all green, and `tests/cli_golden.rs` passes with no golden file changes (`git status` shows no change under `tests/`).

- [ ] **Step 7: Commit**
  ```bash
  git add crates/host/src/service/types.rs crates/host/src/service/status.rs
  git commit -m "fix(host): report a not-running role's owner as known and absent (#165)"
  ```

---

## Scenario → test

- Role not running has a known absent owner → `status_reports_not_running_owner_as_known_absent` (E2E through `status_via`, Unix) and `owner_fact_is_known_absent_when_not_running` (unit, all platforms)
- Foreground role without a registration (unchanged, carried in the MODIFIED block) → existing `status_reports_foreground_role_without_registration`
- Owner metadata unavailable (requirement "Optional status facts do not cause failure", unchanged) → existing `status_reports_missing_owner_as_unavailable` and `owner_fact_is_unavailable_when_running_without_a_record`
