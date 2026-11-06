#!/usr/bin/env bash
# Tests for move-major-tag.sh: three dry-run decisions and the API failure path.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
script="$here/move-major-tag.sh"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

passed=0
failed=0

check() { # <name> <expected> <actual>
  if [[ "$2" == "$3" ]]; then
    echo "PASS: $1"
    passed=$((passed + 1))
  else
    echo "FAIL: $1"
    echo "  expected: $2"
    echo "  actual:   $3"
    failed=$((failed + 1))
  fi
}

printf 'v0.5.0\nv0.6.0\n' > "$work/stable.txt"
check "stable release moves the major tag" \
  "move v0 -> v0.6.0" \
  "$(bash "$script" --dry-run v0.6.0 "$work/stable.txt")"

printf 'v0.5.0\nv0.6.0\nv0.5.9\n' > "$work/older.txt"
check "older patch is skipped" \
  "skip: v0.5.9 is not the highest stable v0 release (v0.6.0)" \
  "$(bash "$script" --dry-run v0.5.9 "$work/older.txt")"

printf 'v0.5.0\nv0.6.0\nv0.6.1-rc.1\n' > "$work/pre.txt"
check "pre-release is skipped" \
  "skip: pre-release" \
  "$(bash "$script" --dry-run v0.6.1-rc.1 "$work/pre.txt")"

# API failure: a gh shim on PATH that always fails.
mkdir "$work/bin"
printf '#!/bin/sh\nexit 1\n' > "$work/bin/gh"
chmod +x "$work/bin/gh"
rc=0
out="$(PATH="$work/bin:$PATH" GH_REPO=o/r bash "$script" v0.6.0 2>&1)" || rc=$?
check "API failure prints the manual command and exits 1" \
  "1|move it by hand: git tag -f v0 v0.6.0^{commit} && git push -f origin refs/tags/v0 (this push re-triggers release.yml, whose preflight rejects v0; ignore that run)" \
  "$rc|$out"

echo "$passed passed, $failed failed"
[[ "$failed" == 0 ]]
