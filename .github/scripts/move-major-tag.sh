#!/usr/bin/env bash
# Move the major action tag (vX) to a stable release, only when that release is
# the highest stable version of its major.
#
#   move-major-tag.sh <tag>                     live: uses `gh api` (GH_REPO, GH_TOKEN)
#   move-major-tag.sh --dry-run <tag> <file>    print the decision from a tag list file
set -euo pipefail

# Highest stable tag of <major> from a tag list on stdin (one per line).
highest_stable() { # <major>
  grep -E "^v${1}\.[0-9]+\.[0-9]+\$" | grep -v -- '-' | sort -V | tail -1
}

# Force-move refs/tags/v<major> to <sha>; create it only when it does not exist.
# Any other PATCH error (403, 5xx, rate limit) is returned, not retried as POST.
move_ref() { # <major> <sha>
  local err
  if err="$(gh api -X PATCH "repos/$GH_REPO/git/refs/tags/v${1}" -f sha="$2" -F force=true 2>&1 >/dev/null)"; then
    return 0
  fi
  if [[ "$err" == *"HTTP 404"* || "$err" == *"Not Found"* ]]; then
    gh api -X POST "repos/$GH_REPO/git/refs" -f ref="refs/tags/v${1}" -f sha="$2" >/dev/null
  else
    echo "$err" >&2
    return 1
  fi
}

list_tags() { # <major>
  gh api --paginate "repos/$GH_REPO/git/matching-refs/tags/v${1}." --jq '.[].ref' \
    | sed 's#^refs/tags/##'
}

decide() { # <tag> <major> <highest>; prints the decision, returns 0 to move
  if [[ "$1" == *-* ]]; then
    echo "skip: pre-release"
    return 1
  fi
  if [[ "$1" != "$3" ]]; then
    echo "skip: $1 is not the highest stable v$2 release ($3)"
    return 1
  fi
  echo "move v$2 -> $1"
}

main() {
  local dry=false file=""
  if [[ "${1:-}" == "--dry-run" ]]; then
    dry=true
    shift
    file="${2:?usage: move-major-tag.sh --dry-run <tag> <file>}"
  fi
  local tag="${1:?usage: move-major-tag.sh [--dry-run] <tag> [file]}"
  local re='^v([0-9]+)\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$'
  if [[ ! "$tag" =~ $re ]]; then
    echo "tag $tag is not vX.Y.Z[-pre]" >&2
    exit 1
  fi
  local major="${BASH_REMATCH[1]}"

  if [[ "$tag" == *-* ]]; then
    echo "skip: pre-release"
    return 0
  fi

  local tags best sha
  if $dry; then
    tags="$(cat "$file")"
  elif ! tags="$(list_tags "$major")"; then
    manual "$major" "$tag"
  fi
  best="$(printf '%s\n' "$tags" | highest_stable "$major" || true)"
  if [[ -z "$best" ]]; then
    echo "no stable v$major tags found" >&2
    exit 1
  fi
  decide "$tag" "$major" "$best" || return 0
  $dry && return 0

  if ! sha="$(gh api "repos/$GH_REPO/commits/$tag" --jq .sha)" || ! move_ref "$major" "$sha"; then
    manual "$major" "$tag"
  fi
}

manual() { # <major> <tag>
  echo "move it by hand: git tag -f v${1} ${2}^{commit} && git push -f origin refs/tags/v${1} (this push re-triggers release.yml, whose preflight rejects v${1}; ignore that run)"
  exit 1
}

main "$@"
