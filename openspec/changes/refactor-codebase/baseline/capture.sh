#!/usr/bin/env bash
# Usage: capture.sh <outdir>  — records pre-refactor baseline inventories.
set -euo pipefail
out="${1:?usage: capture.sh <outdir>}"
cd "$(git rev-parse --show-toplevel)"
mkdir -p "$out"
out="$(cd "$out" && pwd)"

cargo +1.91 test --all-features --locked -- --list --format terse 2>/dev/null | grep ': test$' | sort -u > "$out/tests.txt"
cargo +1.91 test --all-features --locked -- --list --format terse --ignored 2>/dev/null | grep ': test$' | sort -u > "$out/tests-ignored.txt"

grep -E '^name = "' Cargo.lock | sed -E 's/^name = "(.*)"$/\1/' | grep -vx 'iroh-tunnel' | sort -u > "$out/external-crates.txt"

cargo +1.91 tree -e features --locked > "$out/features-normal.txt"
cargo +1.91 tree -e features --all-features --locked > "$out/features-all.txt"

cargo +1.91 metadata --no-deps --format-version 1 --locked \
  | jq -r '.packages[].targets[] | select(.kind | index("bin")) | .name' | sort -u > "$out/binaries.txt"
