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


printf 'v0.9.0\nv0.10.0\n' > "$work/order.txt"
check "v0.10.0 outranks v0.9.0" \
  "move v0 -> v0.10.0" \
  "$(bash "$script" --dry-run v0.10.0 "$work/order.txt")"
check "v0.9.0 is skipped behind v0.10.0" \
  "skip: v0.9.0 is not the highest stable v0 release (v0.10.0)" \
  "$(bash "$script" --dry-run v0.9.0 "$work/order.txt")"

printf 'v0.6.0\nv0.7.0-rc.1\n' > "$work/prelist.txt"
check "pre-release in the list is ignored for a stable tag" \
  "move v0 -> v0.6.0" \
  "$(bash "$script" --dry-run v0.6.0 "$work/prelist.txt")"

rc=0
out="$(bash "$script" --dry-run banana "$work/stable.txt" 2>&1)" || rc=$?
check "malformed tag exits 1 with a message" \
  "1|tag banana is not vX.Y.Z[-pre]" "$rc|$out"

: > "$work/empty.txt"
rc=0
out="$(bash "$script" --dry-run v0.6.0 "$work/empty.txt" 2>&1)" || rc=$?
check "empty tag list exits 1 with a message" \
  "1|no stable v0 tags found" "$rc|$out"

# Stateful gh shim for the live path. PATCH_MODE: ok | 404 | 403.
cat > "$work/bin/gh" <<'SHIM'
#!/bin/sh
echo "$*" >> "$GH_LOG"
case "$*" in
  *matching-refs*) printf 'refs/tags/v0.5.0\nrefs/tags/v0.6.0\n' ;;
  *"commits/v0.6.0"*) echo abc123 ;;
  *"-X PATCH"*)
    case "$PATCH_MODE" in
      ok) ;;
      404) echo "gh: Not Found (HTTP 404)" >&2; exit 1 ;;
      *) echo "gh: Resource not accessible by integration (HTTP 403)" >&2; exit 1 ;;
    esac ;;
  *"-X POST"*) ;;
  *) exit 2 ;;
esac
SHIM
chmod +x "$work/bin/gh"

live() { # <mode>; prints "rc|calls|output-has-manual"
  local rc=0 out
  : > "$work/log"
  out="$(PATH="$work/bin:$PATH" GH_REPO=o/r GH_LOG="$work/log" PATCH_MODE="$1" bash "$script" v0.6.0 2>&1)" || rc=$?
  local hint=no
  [[ "$out" == *"move it by hand"* ]] && hint=yes
  echo "$rc|$(grep -c 'X PATCH' "$work/log") patch,$(grep -c 'X POST' "$work/log") post|manual=$hint"
}
check "live: PATCH succeeds, no POST" "0|1 patch,0 post|manual=no" "$(live ok)"
check "live: PATCH 404 falls back to POST" "0|1 patch,1 post|manual=no" "$(live 404)"
check "live: PATCH 403 makes no POST and prints the manual hint" "1|1 patch,0 post|manual=yes" "$(live 403)"

echo "$passed passed, $failed failed"
[[ "$failed" == 0 ]]
