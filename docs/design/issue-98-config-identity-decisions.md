# Issue #98 — configuration and identity lifecycle decisions

Status: **decisions recorded from owner answers; spec edits (CFG-04, CFG-05, AC-CFG-05–07) pending approval.** This is a decision and evidence gate; no production code changes. Base: `origin/main` 42ef3a8.

## 1. Inventory of current behavior (criterion 1)

Observed with the debug binary against an isolated `HOME`, not only by reading code.

| Area | Current behavior | Evidence | Requirement |
|---|---|---|---|
| Config commands | `keygen`, `add`, `remove`, `list`, `show`, `edit`, `path`. Only `keygen` and `add` accept `--config`; the rest use the default path (`src/cli.rs`, `src/app.rs:231-237`). | `serve config show --config X` → `unexpected argument` | Story 35 (explicit override) — **gap** |
| Missing config, read | `list` prints "No services configured", exit 0. `show` and `run` fail with `failed to read config … No such file`. Three commands, two behaviors; nothing is created. | `list`/`show`/`run` with empty `HOME` | CFG-05 first-run — **unresolved** |
| Missing config, write | `add`/`keygen` create the file (`load_or_empty`) with mode 0600 and create the parent directory. | `src/config_cmd.rs:273-318` | Unspecified |
| Validation on write | `add` validates the full document through the schema before persisting. `keygen`, `remove` and `edit` do not. | `keygen` on a file with an invalid service name wrote `secret_key` into it | CFG-04 — **proposed** |
| Persistence | `config_cmd::write_doc` and `RoleDoc::save` use `fs::write` (truncate then write): not atomic, no fsync, no lock. | `src/config_cmd.rs:300`, `src/config.rs:203` | CFG-04 atomic contract — **unresolved** |
| File permissions | `config_cmd::write_doc` sets 0600. **`RoleDoc::save`, used when `run` auto-generates a key, does not** — the file ends up 0644. | `run` on a 0644 config with no key left it at 0644 containing `secret_key` | Story 38 intent — **defect** |
| Concurrent edits | None. Last writer wins; read-modify-write window is unprotected. | code read | CFG-05 / AC-CFG-06 — **unresolved** |
| Identity rotation | `keygen` replaces `[node].secret_key` immediately, with no prompt, even when a key exists. It prints the new NodeId and a warning afterwards. | Two consecutive `keygen` runs rotated silently | CFG-05 — **unresolved** |
| Secret display | `config show` prints the whole file including `secret_key` (and `relay_token`). README line 196 says the key is "never printed". | `serve config show \| grep secret_key` | CFG-05 — **unresolved; contradicts README** |
| Legacy migration (macOS) | New config wins; otherwise copy without overwrite, keep source, 0600, failure creates no identity (`src/paths.rs:95-135`). | CFG-01–03, stories 36–39 | **Confirmed** — retained |
| `multiplex` | Ignored on load, removed on write. | CFG-03 | **Confirmed** — retained |
| `edit` | Spawns `$EDITOR` on the live file; no validation afterwards. | `src/config_cmd.rs:178` | CFG-04 — **unresolved** |

## 2. Decisions (criteria 2–3)

| ID | Topic | Owner decision | Consequence |
|---|---|---|---|
| D1 | Key rotation | When `[node].secret_key` already exists, `keygen` asks for confirmation. `--yes` skips it. Non-interactive without `--yes` fails with an actionable error and does not rotate. First-time generation needs no prompt. | New flag on `keygen`; new interactive path. |
| D2 | Secret disclosure | `config show` redacts `secret_key` and `relay_token` by default; `--reveal` prints them. `config edit` and reading the file directly are unchanged. | Behavior change to `show`; README line 196 becomes true. |
| D3 | Missing config | Every read command (`run`, `list`, `show`) fails with the same guidance error naming the path and suggesting `config add` / `config keygen`. Nothing is created and no identity is generated. Write commands (`add`, `keygen`) may still create the file. | `list` changes from exit 0 to an error. Consistent with stories 39–40. |
| D4 | Write safety | Every mutation validates the resulting document, writes via temp file in the same directory → fsync → rename, and holds an advisory lock on a sibling lock file so writers serialize. `edit` validates after the editor exits and rejects an invalid result, leaving the prior file. All config commands accept `--config`. | Replaces both `fs::write` sites with one shared writer. |
| D5 | Auto-generated key permissions | Defect, not a policy choice: the `run` auto-generation save path must create the file owner-only, same as `config_cmd`. Folded into D4's shared writer. | Fixes the 0644 finding. |

| D6 | Ephemeral keygen | `config keygen random` prints a fresh random secret key and its node ID to stdout (`Secret key:` / `Node ID:`) and writes nothing: it neither reads nor creates a config file and ignores any existing key. Needs no confirmation (D1 does not apply) and, because the output is the secret, is the one command meant to disclose it (D2 does not apply). **Implemented in this PR** (tests: `keygen_random_*` in `tests/config_paths.rs`). | Role-agnostic; `--config` is accepted but unused. |

### Still open (do not block D1–D5)

- Windows file-permission/ACL equivalent of 0600 for the shared writer: needs Windows evidence; ownership belongs with #79 for service contexts.
- Exact lock mechanism per platform (`flock` vs `LockFileEx`) and behavior when the lock is held (wait with timeout vs fail fast). Default proposal: bounded wait, then a clear error.
- Whether `edit` restores the previous content or keeps the edited text in a sidecar file when validation fails.
- Display of `node_id` for access services is public and stays unredacted.

## 3. Testable scenarios

Each is exercised through the real binary against an isolated `HOME`/`--config`.

| ID | Scenario | Expected |
|---|---|---|
| AC-CFG-05 | `add`, `remove`, `keygen`, or `edit` would leave the document invalid | Command fails; file bytes unchanged; existing identity unchanged. |
| AC-CFG-06 | Two `add` processes race on one file | Both succeed with both services present, or the loser reports a conflict; never a corrupted file or silent lost update. |
| AC-CFG-07a | `keygen` with an existing key, no TTY, no `--yes` | Non-zero exit, key unchanged. With `--yes`: rotates and prints new NodeId. Without an existing key: generates without prompting. |
| AC-CFG-07b | `show` with a configured key and relay token | Output contains neither secret; `show --reveal` contains both. |
| AC-CFG-08 | `run`, `list`, `show` with no config | Same guidance error naming the path; no file or directory created; no identity generated. |
| AC-CFG-09 | `run` with a config lacking a key | After first run the file is mode 0600 and contains the persisted key; restart reuses the same NodeId. |
| AC-CFG-10 | Kill the process during a save | The file is either the old or the new complete document, never truncated. |
| AC-CFG-12 | `keygen random` with an existing config | Prints key and matching node ID; the config file is byte-identical; no config is created when none exists; two runs differ. |
| AC-CFG-11 | Every config subcommand with `--config <path>` | Operates on that path and not on the default. |

Confirmed behaviors CFG-01–03 and AC-CFG-01–04 are retained unchanged.

## 4. Follow-up ownership

- #99 implements D1–D5 and AC-CFG-05–11. It is already blocked by #98 and #83; no new native blocker needed.
- Windows ACL parity and lock mechanism: decide inside #99 with Windows evidence; if unavailable, report UNVERIFIED rather than PASS (#97 rule).
- Spec edits to apply on approval: CFG-04 Proposed→Confirmed with the atomic contract above; CFG-05 Deferred→Confirmed for rotation, disclosure and first-run; add AC-CFG-05–11 to `redesign-acceptance.md` and `redesign-traceability.md`.

## 5. Limits of this evidence

Observations come from a macOS debug build on one machine. Linux and Windows behavior was not run; the permission and lock findings for those platforms are from code reading only.
