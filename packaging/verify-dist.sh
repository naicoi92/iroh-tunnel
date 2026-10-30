#!/bin/sh
# Verify the Linux release artifacts GoReleaser put in dist/ (issue #94).
#
#   packaging/verify-dist.sh static  [DIST]   inspect packages and archives
#   packaging/verify-dist.sh runtime [DIST]   install each package alone in a
#                                             clean container, run its --help
#   packaging/verify-dist.sh images  [DIST]   run each role image alone
#
# static needs only ar/tar. runtime and images need a Docker daemon and only
# exercise the architecture the daemon runs natively; other architectures are
# reported as SKIP, never as passed. Run from the repository root.
#
# These checks prove package contents, ownership and that each executable
# starts alone. Forwarding bytes between peers is covered by the process-level
# tests in tests/role_binaries.rs, which run the same executables.

set -eu

MODE="${1:?usage: verify-dist.sh static|runtime|images [DIST]}"
DIST=$(cd "${2:-dist}" && pwd)

# executable:alias, one per package/image. The package name equals the
# executable name.
EXECUTABLES="iroh-tunnel:it iroh-tunnel-serve:its iroh-tunnel-access:ita"
ALL_BINARIES="iroh-tunnel iroh-tunnel-serve iroh-tunnel-access"

FAILED=0
CASE_FAILED=0
fail() { echo "FAIL: $*" >&2; FAILED=1; CASE_FAILED=1; }
pass() { echo "ok:   $*"; }
skip() { echo "SKIP: $*"; }

# Print the only file matching a glob; fail on none or several.
only_match() {
    # shellcheck disable=SC2086 # the glob must expand here
    set -- $1
    if [ "$#" -ne 1 ] || [ ! -e "$1" ]; then
        return 1
    fi
    echo "$1"
}

# List a package's members, one per line: "path" or "path -> target".
# The leading `tar -tv` columns differ between GNU and BSD tar, so keep only
# the trailing path (and link target).
list_package() {
    case "$1" in
    *.deb) ar p "$1" data.tar.gz | tar -tvzf - ;;
    *.apk) tar -tvzf "$1" ;;
    esac | sed -E -e 's/^.* ([^ ]+ -> [^ ]+)$/\1/' -e t -e 's/^.* ([^ ]+)$/\1/'
}

check_package() {
    pkg="$1" bin="$2" alias="$3"
    CASE_FAILED=0
    listing=$(list_package "$pkg") || { fail "$pkg: cannot list members"; return; }
    # The package owns its binary and its alias link, and no other executable.
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
    if [ "$CASE_FAILED" -eq 0 ]; then pass "$pkg contains only $bin and alias $alias"; fi
}

check_archive() {
    tgz="$1" bin="$2"
    CASE_FAILED=0
    members=$(tar -tzf "$tgz") || { fail "$tgz: cannot list members"; return; }
    echo "$members" | grep -qx "$bin" || fail "$tgz: does not contain $bin"
    for other in $ALL_BINARIES; do
        [ "$other" = "$bin" ] && continue
        if echo "$members" | grep -qx "$other"; then
            fail "$tgz: contains unrelated binary $other"
        fi
    done
    if [ "$CASE_FAILED" -eq 0 ]; then pass "$tgz contains only $bin"; fi
}

static() {
    for entry in $EXECUTABLES; do
        bin=${entry%%:*} alias=${entry##*:}
        # .deb: glibc build; .apk: musl build (riscv64gc ships .apk only).
        for pkg_glob in "_*_amd64.deb" "_*_arm64.deb" "_*_amd64.apk" \
            "_*_arm64.apk" "_*_riscv64gc.apk"; do
            pkg=$(only_match "$DIST/${bin}${pkg_glob}") ||
                { fail "no single $bin package matching ${pkg_glob}"; continue; }
            check_package "$pkg" "$bin" "$alias"
        done
        for target in linux-amd64 linux-arm64 linux-amd64-musl \
            linux-arm64-musl linux-riscv64gc-musl; do
            tgz=$(only_match "$DIST/${bin}-[0-9]*-${target}.tar.gz") ||
                { fail "no single $bin archive for $target"; continue; }
            check_archive "$tgz" "$bin"
        done
    done
}

docker_arch() {
    case "$(docker info --format '{{.Architecture}}')" in
    aarch64 | arm64) echo arm64 ;;
    x86_64 | amd64) echo amd64 ;;
    *) echo "unsupported Docker architecture" >&2; return 1 ;;
    esac
}

# Install ONE package into an empty container and run its role via the alias.
runtime() {
    arch=$(docker_arch)
    for format in deb apk; do
        case "$format" in
        deb) image=debian:12-slim install="dpkg -i /pkg.deb" ;;
        apk) image=alpine:3.21 install="apk add --allow-untrusted /pkg.apk" ;;
        esac
        for entry in $EXECUTABLES; do
            bin=${entry%%:*} alias=${entry##*:}
            pkg=$(only_match "$DIST/${bin}_*_${arch}.$format") ||
                { fail "no $bin .$format for $arch"; continue; }
            if out=$(docker run --rm -v "$pkg:/pkg.$format:ro" "$image" sh -ec "
                $install >/dev/null
                for other in $ALL_BINARIES; do
                    [ \$other = $bin ] && continue
                    if command -v \$other; then echo unexpected \$other; exit 1; fi
                done
                command -v $bin >/dev/null
                test \"\$(readlink \$(command -v $alias))\" = $bin
                $alias --help
            " 2>&1); then
                pass "$pkg installs alone on $image and $alias --help runs"
            else
                fail "$pkg on $image: $out"
            fi
        done
    done
    skip "runtime for architectures other than $arch (no native runner)"
}

images() {
    arch=$(docker_arch)
    for entry in $EXECUTABLES; do
        bin=${entry%%:*}
        img=$(docker image ls --format '{{.Repository}}:{{.Tag}}' "ghcr.io/naicoi92/$bin" |
            grep -- "-$arch\$" | head -n1 || true)
        [ -n "$img" ] || { fail "no local $bin image for $arch"; continue; }
        entry_point=$(docker image inspect --format '{{json .Config.Entrypoint}}' "$img")
        case "$entry_point" in
        *"\"$bin\""*) ;;
        *) fail "$img: entrypoint $entry_point does not run $bin"; continue ;;
        esac
        if docker run --rm "$img" --help >/dev/null 2>&1; then
            pass "$img runs $bin alone"
        else
            fail "$img: --help failed"
        fi
    done
    skip "images for architectures other than $arch (no native runner)"
}

case "$MODE" in
static) static ;;
runtime) runtime ;;
images) images ;;
*) echo "unknown mode $MODE" >&2; exit 2 ;;
esac

exit "$FAILED"
