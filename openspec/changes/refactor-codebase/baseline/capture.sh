#!/usr/bin/env bash
# Usage: capture.sh <outdir> [--workspace]  — records pre-refactor baseline inventories.
set -euo pipefail
out="${1:?usage: capture.sh <outdir> [--workspace]}"
ws="${2:-}"
[ -z "$ws" ] || [ "$ws" = --workspace ] || { echo "unknown flag: $ws" >&2; exit 2; }
cd "$(git rev-parse --show-toplevel)"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: > "$out/capture.log"

{ cargo +1.91 --version; git rev-parse HEAD; } > "$out/meta.txt"

# <target>::<test path>; target = lib | bins/<name> | <integration test stem> | doc,
# derived from cargo's "Running ..." headers. Non-header, non-test output goes to capture.log.
list_tests() { # list_tests <extra libtest args...>
  cargo +1.91 test --all-features --locked ${ws:+"$ws"} -- --list --format terse "$@" 2>&1 | awk -v log="$out/capture.log" '
    /^[ \t]*Running / {
      p = $2
      if (p == "unittests") p = $3
      if (p ~ /^src\/lib\.rs$/) t = "lib"
      else if (p ~ /^src\/.*\.rs$/) { e = $0; sub(/^.*target\/[^ ]*deps\//, "", e); sub(/-[0-9a-f]+\)?$/, "", e); t = "bins/" e }
      else if (p ~ /^tests\//) { t = p; sub(/^tests\//, "", t); sub(/\.rs$/, "", t) }
      else t = p
      next }
    /^[ \t]*Doc-tests / { t = "doc"; next }
    /: test$/ { sub(/: test$/, ""); print t "::" $0; next }
    { print > log }' | LC_ALL=C sort
}
list_tests > "$out/tests.txt"
list_tests --ignored > "$out/tests-ignored.txt"

grep -E '^name = "' Cargo.lock | sed -E 's/^name = "(.*)"$/\1/' | grep -vx 'iroh-tunnel' | sort -u > "$out/external-crates.txt"

cargo +1.91 tree -e features -e normal --locked > "$out/features-normal.txt"
cargo +1.91 tree -e features --all-features --locked > "$out/features-all.txt"

cargo +1.91 metadata --no-deps --format-version 1 --locked \
  | jq -r '.packages[].targets[] | select(.kind | index("bin")) | .name' | sort -u > "$out/binaries.txt"

bash "$here/public-api.sh" > "$out/public-api.txt"
