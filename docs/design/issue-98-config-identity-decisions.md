# Issue #98 — configuration and identity lifecycle decisions

Status: **decisions D1–D6 recorded from the owner's answers to structured questions in the agent session on 2026-10-31 (not yet echoed as an issue comment) and applied to the spec (CFG-04, CFG-05, AC-CFG-05–12).** This is a decision and evidence gate; the only production change is `config keygen random` (D6). Base: `origin/main` 42ef3a8.

## 1. Inventory of current behavior (criterion 1)

Rows marked *observed* were reproduced with the debug binary against an isolated `HOME` (commands in section 5); rows marked *code-read* come from reading the source only.

| Area | Current behavior | Evidence | Requirement |
|---|---|---|---|
| Config commands | `keygen`, `add`, `remove`, `list`, `show`, `edit`, `path`. Only `keygen` and `add` accept `--config`; the rest use the default path (`ConfigAction` in `src/cli.rs`, `dispatch_config` in `src/app.rs`). | `serve config show --config X` → `unexpected argument` | Story 35 (explicit override) — **gap** |
| Missing config, read | `list` prints "No services configured", exit 0. `show` and `run` fail with `failed to read config … No such file`. Three commands, two behaviors; nothing is created. | `list`/`show`/`run` with empty `HOME` | CFG-05 first-run — **unresolved** |
| Missing config, write | `add`/`keygen` create the file (`load_or_empty`) with mode 0600 and create the parent directory. | `config_cmd::load_or_empty` / `write_doc` | Unspecified |
| Validation on write | `add` validates the full document through the schema before persisting. `keygen`, `remove` and `edit` do not. | `keygen` on a file with an invalid service name wrote `secret_key` into it | CFG-04 — **proposed** |
| Persistence | `config_cmd::write_doc` and `RoleDoc::save` use `fs::write` (truncate then write): not atomic, no fsync, no lock. | `config_cmd::write_doc`, `RoleDoc::save` | CFG-04 atomic contract — **unresolved** |
| File permissions | `config_cmd::write_doc` sets 0600. **`RoleDoc::save`, used when `run` auto-generates a key, does not** — the file ends up 0644. | `run` on a 0644 config with no key left it at 0644 containing `secret_key` | Story 38 intent — **defect** |
| Concurrent edits | None. Last writer wins; read-modify-write window is unprotected. | code read | CFG-05 / AC-CFG-06 — **unresolved** |
| Identity rotation | `keygen` replaces `[node].secret_key` immediately, with no prompt, even when a key exists. It prints the new NodeId and a warning afterwards. | Two consecutive `keygen` runs rotated silently | CFG-05 — **unresolved** |
| Secret display | `config show` prints the whole file including `secret_key` (and `relay_token`). README line 196 says the key is "never printed". | `serve config show \| grep secret_key` | CFG-05 — **unresolved; contradicts README** |
| Legacy migration (macOS) | New config wins; otherwise copy without overwrite, keep source, 0600, failure creates no identity (`paths::migrate_legacy`). | CFG-01–03, stories 36–39 | **Confirmed** — retained |
| `multiplex` | Ignored on load, removed on write. | CFG-03 | **Confirmed** — retained |
| `edit` | Spawns `$EDITOR` on the live file; no validation afterwards. | `config_cmd::edit` | CFG-04 — **unresolved** |

## 2. Options and consequences (criterion 2)

| Topic | Options considered | Platform / security consequence | Chosen |
|---|---|---|---|
| Key rotation | (a) prompt, `--yes` skips; (b) refuse unless `--force`; (c) no confirmation | Rotation changes the NodeId, so every peer config pinning it breaks. (c) allows silent loss of access; (b) cannot prompt a human; (a) serves both. | (a) D1 |
| Secret disclosure | (a) redact by default, `--reveal`; (b) always redact; (c) print as today | `show` output lands in terminals, logs and tickets. (c) contradicts README. (b) forces reading the file for any legitimate need. | (a) D2 |
| Missing config | (a) uniform guidance error, no side effect; (b) `run` bootstraps config and identity; (c) per-command status quo | (b) mints an identity implicitly, against stories 39–40. (c) leaves `list` exit 0 beside failing `show`/`run`. | (a) D3 |
| Validation, atomicity, conflicts | (a) validate all mutations, atomic rename, advisory lock; (b) atomic rename plus optimistic mtime/hash check; (c) atomic only | (c) leaves lost updates. (b) needs no lock but the losing writer must rerun. (a) serializes. `flock` (Unix) and `LockFileEx` (Windows) differ; Windows ACL parity with 0600 is unverified. | (a) D4 |
| Auto-generated key permissions | (a) owner-only through the shared writer; (b) leave | 0644 exposes the key to other local users. | (a) D5 (defect) |
| Ephemeral key | (a) `keygen random`, prints, no write; (b) `--stdout` flag on `keygen`; (c) none | (a) cannot overwrite an existing key by mistake and has no confirmation to bypass. | (a) D6 |

## 3. Decisions

| ID | Topic | Owner decision | Consequence |
|---|---|---|---|
| D1 | Key rotation | When `[node].secret_key` already exists, `keygen` asks for confirmation. `--yes` skips it. Non-interactive without `--yes` fails with an actionable error and does not rotate. First-time generation needs no prompt. | New flag on `keygen`; new interactive path. |
| D2 | Secret disclosure | `config show` redacts `secret_key` and `relay_token` by default; `--reveal` prints them. `config edit` and reading the file directly are unchanged. | Behavior change to `show`; README line 196 becomes true. |
| D3 | Missing config | Every read command (`run`, `list`, `show`) fails with the same guidance error naming the path and suggesting `config add` / `config keygen`. Nothing is created and no identity is generated. Write commands (`add`, `keygen`) may still create the file. | `list` changes from exit 0 to an error. Consistent with stories 39–40. |
| D4 | Write safety | Every mutation validates the resulting document, writes via temp file in the same directory → fsync → rename, and holds an advisory lock on a sibling lock file so writers serialize. `edit` validates after the editor exits and rejects an invalid result, leaving the prior file. All config commands accept `--config`. | Replaces both `fs::write` sites with one shared writer. |
| D5 | Auto-generated key permissions | Defect, not a policy choice: the `run` auto-generation save path must create the file owner-only, same as `config_cmd`. Folded into D4's shared writer. | Fixes the 0644 finding. |

| D6 | Ephemeral keygen | `config keygen random` prints a fresh random secret key and its node ID to stdout (`Secret key:` / `Node ID:`) and writes nothing: it neither reads nor creates a config file and ignores any existing key. Needs no confirmation (D1 does not apply) and, because the output is the secret, is the one command meant to disclose it (D2 does not apply). **Implemented in this PR** (tests: `keygen_random_*` in `tests/config_paths.rs`). | Role-agnostic; `--config` together with `random` is rejected at parse time rather than silently ignored. |

### Still open (do not block D1–D5)

- Windows file-permission/ACL equivalent of 0600 for the shared writer: needs Windows evidence; ownership belongs with #79 for service contexts.
- Exact lock mechanism per platform (`flock` vs `LockFileEx`) and behavior when the lock is held (wait with timeout vs fail fast). Default proposal: bounded wait, then a clear error.
- Whether the rejected edited text is kept in a sidecar file for the user to fix. The main config file is always left unchanged (AC-CFG-05); only the sidecar is open.
- Display of `node_id` for access services is public and stays unredacted.

## 4. Testable scenarios

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
| AC-CFG-11 | Every config subcommand with `--config <path>` | Operates on that path and not on the default. |
| AC-CFG-12 | `keygen random` with an existing config | Prints key and matching node ID; the config file is byte-identical; no config is created when none exists; two runs differ. |

Confirmed behaviors CFG-01–03 and AC-CFG-01–04 are retained unchanged.

## 5. Follow-up ownership

- #99 implements D1–D5 and AC-CFG-05–11; D6 and AC-CFG-12 are implemented in the #98 PR. It is already blocked by #98 and #83; no new native blocker needed.
- Windows ACL parity and lock mechanism: decide inside #99 with Windows evidence; if unavailable, report UNVERIFIED rather than PASS (#97 rule).
- Spec edits applied: CFG-04 and CFG-05 confirmed in `redesign-spec.md`; case `CFG-LIFECYCLE-01` in `redesign-acceptance.md` and rows in `redesign-traceability.md` cover AC-CFG-05–12.

## 6. Limits of this evidence

Reproduction (isolated `HOME`, debug build): `serve config keygen` twice and compare `secret_key`; `serve config show | grep secret_key`; `serve config list|show|run` with an empty `HOME`; `serve config add --name a --port 1` then a duplicate; `keygen` on a file whose service name is `X`; `serve run` on a 0644 config without a key, then `stat`. Rotation without prompt, secret display, the three missing-config behaviors, validation-less `keygen` and the 0644 result were *observed*; non-atomic `fs::write`, absence of locking and `edit` without validation are *code-read*.

Observations come from a macOS debug build on one machine. Linux and Windows behavior was not run; the permission and lock findings for those platforms are from code reading only.
