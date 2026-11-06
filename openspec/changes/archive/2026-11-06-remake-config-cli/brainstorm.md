## Design Summary

**Intent.** The operator finds the current `config` commands awkward. Every value has to be passed as a long flag (`config add --name web --protocol tcp --port 3000`). Requirements and defaults are inconsistent: `--protocol` is mandatory in `config add` but defaults to `tcp` in quick mode, and access `--node-id` is optional at parse time but required later. Help text has blank descriptions and no examples. A wrong command prints either a one-line usage or a bare `config error:` with no help at all. The goal is a config CLI you can use without memorising flags, which tells you exactly what was wrong and shows the relevant help when you get it wrong.

**Prior art reviewed.** tailscale serve, ngrok, cloudflared, kubectl config, gh config, git config, dumbpipe, rathole/frp and clig.dev. Patterns adopted:
- named flags with visible defaults for every attribute, the primary object included (clig.dev "prefer flags to args"), and one flag spelling shared by every command that takes the same value;
- set means upsert (kubectl `set-context`);
- addresses accept `PORT` or `HOST:PORT` (tailscale, ngrok);
- a validate-only command (`ngrok config check`);
- an error followed by the full help of the failing command (tailscale, git);
- help with examples, possible values and defaults (all of them).

**Command surface (per role, both all-in-one and standalone binaries).**

```
serve  config set -n NAME [-t|--target [HOST:]PORT] [-p|--protocol tcp|udp]
access config set -n NAME [-l|--listen [HOST:]PORT] [-p|--protocol tcp|udp]
                          [-N|--node-id ID]
<role> config rm -n NAME
<role> config list
<role> config show [--reveal]
<role> config edit
<role> config path
<role> config check
<role> config keygen [-y|--yes]
<role> config keygen --print
```
Every `config` subcommand accepts `-c/--config FILE`.

**One argument vocabulary.** Quick mode and `config set` take exactly the same service arguments: the same long name, the same short letter, the same value syntax and the same help text. Turning a quick invocation into a stored entry means inserting `config set`:

```
it serve -n web -t 3000              # run now
it serve config set -n web -t 3000   # store it
```

**Short options.** Every frequently used flag has a short form. A letter means the same option everywhere:

| Short | Long | Where |
|---|---|---|
| `-c` | `--config` | every `config` subcommand, `run`, `service install` |
| `-n` | `--name` | quick mode, `config set`, `config rm` |
| `-t` | `--target` | serve quick mode, serve `config set` |
| `-l` | `--listen` | access quick mode, access `config set` |
| `-N` | `--node-id` | access quick mode, access `config set` |
| `-p` | `--protocol` | quick mode, `config set` |
| `-r` | `--relay-url` | quick mode, both roles (the node's relay for this run) |
| `-k` | `--key-file` | quick mode |
| `-y` | `--yes` | `config keygen` |

Rarely used or dangerous flags stay long-only: `--reveal`, `--print`, `--system`, `--user`.

**Addresses.** The service address is one value, `[HOST:]PORT`, where HOST is an IPv4 address, a bracketed IPv6 address or a hostname, everywhere, so there are no separate `--host`/`--port` flags. When `set` updates an existing entry, a bare `PORT` keeps the entry's current host, while `HOST:PORT` replaces both.

**Behaviour.**
- `-n` is required on `set` and `rm`.
- `set` on a new NAME creates the entry. The address (`-t` for serve, `-l` for access) is required for both roles, and `-N` is also required for access. Defaults are protocol `tcp` and host `127.0.0.1` (when the address is only `PORT`).
- `set` on an existing NAME changes only the attributes that were passed; defaults are never re-applied. The output lists each changed field as `old -> new`, or says nothing changed.
- `rm -n NAME` removes the entry. An unknown name is an error.
- `check` validates the file against the role schema and writes nothing. It reports OK with the path, or the validation error (exit 2).
- `keygen --print` prints a fresh secret key and node id and touches no file. Combining it with `-c` or `-y` is a usage error.
- Every existing guarantee is kept: atomic, locked and owner-only writes, comment preservation, redaction in `show`, secret-free diagnostics, `.rejected` sidecar for `edit`, and the missing-config guidance for read commands.

**Errors and help.**
- A *usage error* is an unknown command or flag, a missing or invalid argument, a conflicting flag, or a missing creation-time attribute in `set`. It prints `error: <what is wrong>` followed by the full help of that exact subcommand, and exits 2.
- Invoking a role or `config` with nothing after it prints that level's help instead of an error about a missing flag.
- *Runtime and config-file errors* (invalid file, lock timeout, read-only file, I/O) print only the error, as today, with the same exit codes.
- Help for every command and flag has a description, shows the default and the possible values (for example `tcp`, `udp`), and includes an Examples section for `set`, `rm`, `keygen` and quick mode.

**Quick mode is aligned in the same change.**
- `serve -n NAME -t TARGET` and `access -n NAME -l LISTEN -N NODE_ID` keep their flags. TARGET and LISTEN accept the same `[HOST:]PORT` as `config set`, hostnames included, with `PORT` meaning `127.0.0.1:PORT`.
- `--relay-url` gains the short `-r`; its meaning (the node's relay) is unchanged.
- `-p` accepts only `tcp` or `udp`, and the help says so.
- Missing quick-mode flags are usage errors with help.

**Goals.**
- Shorter, memorable config commands.
- Consistent requirements and defaults between `config set` and quick mode.
- Actionable errors with help.
- Self-explanatory `--help`.

**Non-goals.**
- Setting `[node]` values (relay URLs, relay token) from the CLI; `config edit` remains the path.
- `ls`/`rm`-style aliases beyond `rm` itself.
- Changes to `run`, `status`, `service` or `update` beyond the shared error/help behaviour.
- Changes to the config file format.

**Anti-goals.**
- A command that silently resets attributes the user did not mention.
- A usage error that dumps help for the wrong (parent) command.
- Help printed for genuine runtime failures, which would bury the real error.
- Any weakening of secret hygiene or the atomic/locked persistence guarantees.

**Compatibility.** This is a breaking change to the public CLI. `config add`, `config remove`, `--name` on those commands and `config keygen random` are removed outright, with no aliases. CHANGELOG gets a BREAKING entry with old-to-new examples. README, `docs/` and examples are updated to the new syntax.

## Alternatives Considered

### Alternative A: Keep flags, fix requirements, defaults and help only
- **Approach**: Keep `config add --name ... --port ...`. Make `--protocol` default to `tcp`, require `--node-id` for access at parse time, and add descriptions, examples and error-with-help.
- **Pros**: Least breaking. Smallest diff.
- **Cons**: Still verbose. Still no way to modify an entry without remove plus add. Serve and access still share one argument shape.
- **Why not chosen**: It fixes the symptoms but not the feeling that the commands are awkward, which is what the operator asked to fix.

### Alternative B: named flags, `set` as upsert (kubectl style) — chosen
- **Approach**: As in the Design Summary. The name was first planned as a positional; the operator chose `-n/--name` so that quick mode and `config` share one vocabulary (a positional cannot work in quick mode, where `serve run` would be ambiguous).
- **Pros**: No argument-order ambiguity. Edits in place. Matches a widely known pattern. Easy to extend with new flags.
- **Cons**: Breaking. Creation-time requirements cannot be expressed purely as parse-time rules, because they depend on whether the entry exists.
- **Why chosen**: See Agreed Approach.

### Alternative C: NAME and TARGET positional (tailscale/ngrok style)
- **Approach**: `config add web 3000`, `access config add web 8080 <NODE_ID>`.
- **Pros**: Shortest to type.
- **Cons**: Three positionals with different meanings on access are easy to misorder, which clig.dev warns against. There is still no in-place edit. It is the most breaking option.
- **Why not chosen**: Brevity at the cost of clarity, and no upsert.

## Agreed Approach

Alternative B, chosen by the operator. Every attribute, the name included, is a named flag with a visible default, and the same flags mean the same thing in quick mode and in `config`. Its upsert semantics remove the remove-then-add dance. The error-and-help behaviour and the quick-mode alignment apply across the whole surface, so the CLI feels consistent rather than having one fixed corner.

## Key Decisions

- **Error output (operator).** A usage error prints the error plus the full help of the failing subcommand, exit 2. Runtime and config-file errors print only the error.
- **Command shape (operator).** Alternative B: `config set -n NAME` as upsert, `config rm -n NAME`.
- **One vocabulary (operator).** Long names, short letters, value syntax and help text are identical wherever the same value is accepted. In particular the name is `-n/--name` everywhere, and addresses accept hostnames in quick mode too.
- **Scope addition (operator).** `config check`: validate only, no write.
- **Old syntax (operator).** `config add`, `config remove`, their `--name` flag and `keygen random` are removed outright, with a BREAKING CHANGELOG entry and no aliases.
- **Updating an existing entry (operator).** `set` reports each changed field `old -> new`.
- **Print-only keygen (operator).** `config keygen random` becomes `config keygen --print`. It conflicts with `-c` and `-y`.
- **Quick mode (operator).** Quick mode is aligned in this change. Target and listen accept `PORT` or `HOST:PORT`. Protocol is restricted to `tcp|udp` with possible values shown in help.
- **Short options (operator note; letter assignment is my call).** Every common flag has a short form with one meaning across quick mode and `config`, as in the Short options table.
- **Creation requirements (my call, accepted in the inbox recap).** The address, and `-N` for access, are required only when `set` creates an entry. A missing one is reported as a usage error with the `set` help.
- **Defaults on update (my call).** Defaults apply only on creation. An update changes only the flags given.
- **No relay flag on `config set` (my call).** In quick mode `-r` sets the node's relay, which lives in `[node]`, outside a service entry. Giving `config set` a `-r` that meant the access per-service override would break "one flag, one meaning". Relays in a config file stay in `config edit`, consistent with the `[node]` non-goal.
- **`-c` everywhere (my call).** `-c/--config` is accepted on every `config` subcommand with one description stating the platform default.
- **Bare role or `config` (my call).** Prints help, consistent with the error-with-help decision.
- **Short port target (my call).** `PORT` alone means `127.0.0.1:PORT` for both `-t` and `-l` when creating, and keeps the existing host when updating.

## Open Questions

- Whether `set` without any attribute flag on an existing entry is a no-op with a message or a usage error. Leaning no-op with a "nothing changed" message; to settle in design.
- Exact wording and layout of the `old -> new` change report and of the Examples sections; to settle in design and specs.
