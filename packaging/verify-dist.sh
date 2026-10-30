#!/bin/sh
# Verify the Linux release artifacts GoReleaser put in dist/ (issue #94).
#
#   packaging/verify-dist.sh static  [DIST]   inspect packages and archives
#   packaging/verify-dist.sh runtime [DIST]   install each package alone in a
#                                             clean container and run its role
#   packaging/verify-dist.sh images  [DIST]   run each role image alone
#
# static needs only ar/tar. runtime and images need a Docker daemon and only
# exercise the architecture the daemon runs natively; other architectures are
# reported as SKIPPED, never as passed. Run from the repository root.

set -eu

MODE="${1:?usage: verify-dist.sh static|runtime|images [DIST]}"
DIST="${2:-dist}"

# role-name | binary | alias
ROLES="iroh-tunnel:iroh-tunnel:it iroh-tunnel-serve:iroh-tunnel-serve:its iroh-tunnel-access:iroh-tunnel-access:ita"
ALL_BINARIES="iroh-tunnel iroh-tunnel-serve iroh-tunnel-access"

FAILED=0
CASE_FAILED=0
fail() { echo "FAIL: $*" >&2; FAILED=1; CASE_FAILED=1; }
ok() { echo "ok:   $*"; }
skip() { echo "SKIP: $*"; }

# Print the single file in DIST matching a glob, or fail.
one() {
    set -- $1
    if [ "$#" -ne 1 ] || [ ! -e "$1" ]; then
        return 1
    fi
    echo "$1"
}

# List the data members of a package (deb or apk), one path per line, with
# symlinks shown as "path -> target".
list_package() {
    case "$1" in
    *.deb) ar p "$1" data.tar.gz 2>/dev/null | tar -tvzf - ;;
    *.apk) tar -tvzf "$1" 2>/dev/null ;;
    esac | awk '{ for (i = 1; i <= 8; i++) $i = ""; sub(/^ +/, ""); print }'
}

check_package() {
    pkg="$1" name="$2" bin="$3" alias="$4"
    CASE_FAILED=0
    listing=$(list_package "$pkg")
    # The package owns its binary and its alias link, and no other role binary.
    echo "$listing" | grep -Eq "(^|/)usr/(local/)?bin/$bin\$" ||
        fail "$pkg: does not contain $bin"
    echo "$listing" | grep -Eq "(^|/)usr/(local/)?bin/$alias -> $bin\$" ||
        fail "$pkg: alias $alias is not a package-owned link to $bin"
    for other in $ALL_BINARIES; do
        [ "$other" = "$bin" ] && continue
        if echo "$listing" | grep -Eq "(^|/)usr/(local/)?bin/$other( |\$)"; then
            fail "$pkg: contains unrelated binary $other"
        fi
    done
    if [ "$CASE_FAILED" -eq 0 ]; then ok "$pkg contains only $bin and alias $alias"; fi
}

check_archive() {
    tgz="$1" bin="$2"
    CASE_FAILED=0
    members=$(tar -tzf "$tgz")
    echo "$members" | grep -qx "$bin" || fail "$tgz: does not contain $bin"
    for other in $ALL_BINARIES; do
        [ "$other" = "$bin" ] && continue
        if echo "$members" | grep -qx "$other"; then
            fail "$tgz: contains unrelated binary $other"
        fi
    done
    if [ "$CASE_FAILED" -eq 0 ]; then ok "$tgz contains only $bin"; fi
}

static() {
    [ -d "$DIST" ] || { fail "$DIST not found; run goreleaser first"; return; }
    for arch in amd64 arm64; do
        for entry in $ROLES; do
            IFS=: read -r name bin alias <<EOF
$entry
EOF
            deb=$(one "$DIST/${name}_*_${arch}.deb") ||
                { fail "no $name .deb for $arch"; continue; }
            check_package "$deb" "$name" "$bin" "$alias"
        done
    done
    for arch in amd64 arm64; do
        for entry in $ROLES; do
            IFS=: read -r name bin alias <<EOF
$entry
EOF
            apk=$(one "$DIST/${name}_*_${arch}.apk") ||
                { fail "no $name .apk for $arch"; continue; }
            check_package "$apk" "$name" "$bin" "$alias"
        done
    done
    for entry in $ROLES; do
        IFS=: read -r name bin alias <<EOF
$entry
EOF
        for flavour in "" "-musl"; do
            tgz=$(one "$DIST/${name}-[0-9]*-linux-arm64${flavour}.tar.gz") ||
                { fail "no $name archive for linux-arm64${flavour}"; continue; }
            check_archive "$tgz" "$bin"
        done
    done
}

docker_arch() {
    case "$(docker info --format '{{.Architecture}}')" in
    aarch64 | arm64) echo arm64 ;;
    x86_64 | amd64) echo amd64 ;;
    esac
}

# Install ONE package into an empty container and run its role via the alias.
runtime() {
    arch=$(docker_arch)
    for flavour in deb apk; do
        case "$flavour" in
        deb) image=debian:12-slim ;;
        apk) image=alpine:3.21 ;;
        esac
        for entry in $ROLES; do
            IFS=: read -r name bin alias <<EOF
$entry
EOF
            pkg=$(one "$DIST/${name}_*_${arch}.$flavour") ||
                { fail "no $name .$flavour for $arch"; continue; }
            case "$flavour" in
            deb) install="dpkg -i /pkg.deb" ;;
            apk) install="apk add --allow-untrusted /pkg.apk" ;;
            esac
            if out=$(docker run --rm -v "$PWD/$pkg:/pkg.$flavour:ro" "$image" sh -ec "
                $install >/dev/null
                for other in $ALL_BINARIES; do
                    [ \$other = $bin ] && continue
                    command -v \$other && { echo \"unexpected \$other\"; exit 1; }
                done
                command -v $bin >/dev/null
                test \"\$(readlink \$(command -v $alias))\" = $bin
                $alias --help
            " 2>&1); then
                ok "$pkg installs alone on $image and $alias --help runs"
            else
                fail "$pkg on $image: $out"
            fi
        done
    done
    skip "runtime for architectures other than $arch (no native runner)"
}

images() {
    arch=$(docker_arch)
    for entry in $ROLES; do
        IFS=: read -r name bin alias <<EOF
$entry
EOF
        img=$(docker image ls --format '{{.Repository}}:{{.Tag}}' "ghcr.io/naicoi92/$name" |
            grep -- "-$arch\$" | head -n1 || true)
        [ -n "$img" ] || { fail "no local $name image for $arch"; continue; }
        entry_point=$(docker image inspect --format '{{json .Config.Entrypoint}}' "$img")
        case "$entry_point" in
        *"/$bin\""* | *"\"$bin\""*) ;;
        *) fail "$img: entrypoint $entry_point does not run $bin"; continue ;;
        esac
        docker run --rm "$img" --help >/dev/null 2>&1 &&
            ok "$img runs $bin alone" || fail "$img: --help failed"
    done
}

case "$MODE" in
static) static ;;
runtime) runtime ;;
images) images ;;
*) echo "unknown mode $MODE" >&2; exit 2 ;;
esac

exit "$FAILED"
