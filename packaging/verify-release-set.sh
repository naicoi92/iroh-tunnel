#!/bin/sh
# Verify that DIST holds the complete, consistent release artifact set (#100):
# every role archive and package for the supported matrix, both checksum files,
# a signature bundle for each checksum file, every artifact listed in (and
# matching) its checksum file, and nothing else.
#
#   packaging/verify-release-set.sh DIST VERSION
#
# This checks completeness and checksums only. The signatures are verified
# against the pinned CI issuer and identity by the `signed_release_*` tests of
# tests/release_artifacts.rs.

set -eu

DIST=$(cd "${1:?usage: verify-release-set.sh DIST VERSION}" && pwd)
VERSION="${2:?usage: verify-release-set.sh DIST VERSION}"
ROLES="iroh-tunnel iroh-tunnel-serve iroh-tunnel-access"

FAILED=0
fail() { echo "FAIL: $*" >&2; FAILED=1; }

EXPECTED=$(mktemp)
SUMMED=$(mktemp)
trap 'rm -f "$EXPECTED" "$SUMMED"' EXIT

# checksums.txt covers Linux and Windows; darwin has its own file (release.yml).
: > "$EXPECTED"
for role in $ROLES; do
    for suffix in linux-amd64.tar.gz linux-arm64.tar.gz linux-amd64-musl.tar.gz \
        linux-arm64-musl.tar.gz linux-riscv64gc-musl.tar.gz windows-amd64.zip \
        windows-arm64.zip; do
        echo "checksums.txt ${role}-${VERSION}-${suffix}" >> "$EXPECTED"
    done
    for pkg in amd64.deb arm64.deb amd64.apk arm64.apk riscv64.apk; do
        echo "checksums.txt ${role}_${VERSION}_${pkg}" >> "$EXPECTED"
    done
    echo "checksums-darwin.txt ${role}-${VERSION}-darwin-arm64.tar.gz" >> "$EXPECTED"
done

for sums in checksums.txt checksums-darwin.txt; do
    for f in "$sums" "$sums.sigstore.json"; do
        [ -s "$DIST/$f" ] || fail "missing or empty: $f"
    done
done

while read -r sums name; do
    if [ ! -f "$DIST/$name" ]; then
        fail "missing artifact: $name"
        continue
    fi
    [ -f "$DIST/$sums" ] || continue
    want=$(awk -v n="$name" '$2 == n { print $1 }' "$DIST/$sums")
    if [ -z "$want" ]; then
        fail "$name is not listed in $sums"
        continue
    fi
    if command -v sha256sum >/dev/null 2>&1; then
        got=$(sha256sum "$DIST/$name" | cut -d' ' -f1)
    else
        got=$(shasum -a 256 "$DIST/$name" | cut -d' ' -f1)
    fi
    [ "$want" = "$got" ] || fail "checksum mismatch: $name (listed $want, actual $got)"
done < "$EXPECTED"

# Nothing outside the approved set may ride along.
{
    awk '{ print $2 }' "$EXPECTED"
    printf '%s\n' checksums.txt checksums.txt.sigstore.json \
        checksums-darwin.txt checksums-darwin.txt.sigstore.json
} | sort > "$SUMMED"
for f in "$DIST"/*; do
    [ -e "$f" ] || continue
    b=$(basename "$f")
    grep -qxF -- "$b" "$SUMMED" || fail "unexpected file: $b"
done

if [ "$FAILED" -eq 0 ]; then
    echo "ok:   complete release set for $VERSION in $DIST"
fi
exit "$FAILED"
