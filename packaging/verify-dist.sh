#!/bin/sh
# Verify the Linux release artifacts GoReleaser put in dist/ (issue #94).
#
#   packaging/verify-dist.sh static  [DIST]   inspect packages and archives
#   packaging/verify-dist.sh runtime [DIST]   install each package alone, then
#                                             all three together, in clean
#                                             containers; run, then remove
#   packaging/verify-dist.sh images  [DIST]   run each role image alone
#   packaging/verify-dist.sh windows [DIST]   inspect the Windows x64/ARM64 role
#                                             archives, checksum and PE format
#                                             (cross-build evidence, not run)
#   packaging/verify-dist.sh macos   [DIST]   inspect the macOS arm64 role
#                                             archives, checksum and Mach-O
#                                             format; run them on a macOS
#                                             arm64 host
#   packaging/verify-dist.sh forward [DIST]   forward real bytes through the
#                                             Serve and Access role images and
#                                             through the installed packages
#
# static needs only ar/tar. runtime and images need a Docker daemon and only
# exercise the architecture the daemon runs natively; other architectures are
# reported as SKIP, never as passed. Run from the repository root.
#
# static, runtime and images prove package contents, ownership and that each
# executable starts alone. forward additionally sends a request through a
# Serve/Access pair, each in its own container with only that role installed,
# to an echo target. It needs Internet access: release binaries accept only
# https:// relays, so the pair meets through the public n0 relays. Because of
# that it is opt-in and not part of the snapshot workflow. Process-level
# forwarding for the same executables is also covered by tests/role_binaries.rs.

set -eu

MODE="${1:?usage: verify-dist.sh static|runtime|images|windows|macos|forward [DIST]}"
DIST=$(cd "${2:-dist}" && pwd)

# executable:alias, one per package/image. The package name equals the
# executable name.
EXECUTABLES="iroh-tunnel:it iroh-tunnel-serve:its iroh-tunnel-access:ita"
ALL_BINARIES="iroh-tunnel iroh-tunnel-serve iroh-tunnel-access"

FAILED=0
CASE_FAILED=0
TMP=$(mktemp -d)
# Containers and networks created by images/forward, removed on any exit.
CLEANUP_CONTAINERS="" CLEANUP_NETWORK=""
cleanup() {
    # shellcheck disable=SC2086 # space-separated names
    [ -z "$CLEANUP_CONTAINERS" ] || docker rm -f $CLEANUP_CONTAINERS >/dev/null 2>&1 || true
    [ -z "$CLEANUP_NETWORK" ] || docker network rm "$CLEANUP_NETWORK" >/dev/null 2>&1 || true
    rm -rf "$TMP"
}
trap cleanup EXIT
trap 'exit 130' INT TERM
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

# List a package's content members, one per line: "path" or "path -> target".
# Leading "./", package-manager metadata (apk dot-files) and the bare usr and
# usr/bin directories are dropped; any other directory (for example a stray
# /etc/iroh-tunnel/) stays in the listing so it fails the exact-member check. The leading `tar -tv` columns differ between GNU and BSD tar, so keep
# only the trailing path (and link target).
list_package() {
    case "$1" in
    *.deb) raw=$(ar p "$1" data.tar.gz > "$TMP/data.tar.gz" && tar -tvzf "$TMP/data.tar.gz") ;;
    *.apk) raw=$(tar -tvzf "$1") ;;
    esac || return 1
    echo "$raw" |
        sed -E -e 's/^.* ([^ ]+ -> [^ ]+)$/\1/' -e t -e 's/^.* ([^ ]+)$/\1/' |
        sed -e 's#^\./##' | grep -v -E -e '^\.' -e '^usr/?$' -e '^usr/bin/?$' -e '^$' | sort
}

# The exact files a package must own: its binary and its alias link. Packages
# are binary-only, so a unit file or config directory would be a failure.
expected_members() {
    bin="$1" link="$2"
    {
        echo "usr/bin/$bin"
        echo "usr/bin/$link -> $bin"
    } | sort
}

check_package() {
    pkg="$1" bin="$2" link="$3"
    CASE_FAILED=0
    listing=$(list_package "$pkg") || { fail "$pkg: cannot list members"; return; }
    if [ "$listing" != "$(expected_members "$bin" "$link")" ]; then
        fail "$pkg: members differ from the expected set
--- found
$listing
--- expected
$(expected_members "$bin" "$link")"
    fi
    if [ "$CASE_FAILED" -eq 0 ]; then pass "$pkg owns exactly $bin and link $link"; fi
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
        bin=${entry%%:*} link=${entry##*:}
        # .deb: glibc build; .apk: musl build (riscv64 ships .apk only).
        # GoReleaser spells the riscv64 arch "riscv64gc" up to 2.18 and
        # "riscv64" from 2.19, so the globs accept either.
        for pkg_glob in "_*_amd64.deb" "_*_arm64.deb" "_*_amd64.apk" \
            "_*_arm64.apk" "_*_riscv64*.apk"; do
            pkg=$(only_match "$DIST/${bin}${pkg_glob}") ||
                { fail "no single $bin package matching ${pkg_glob}"; continue; }
            check_package "$pkg" "$bin" "$link"
        done
        for target in linux-amd64 linux-arm64 linux-amd64-musl \
            linux-arm64-musl "linux-riscv64*-musl"; do
            tgz=$(only_match "$DIST/${bin}-[0-9]*-${target}.tar.gz") ||
                { fail "no single $bin archive for $target"; continue; }
            check_archive "$tgz" "$bin"
        done
    done
}

# Windows and macOS role archives (issue #124). Each archive holds exactly one
# executable, is listed in a checksum file with the right digest, and the
# executable has the format for its platform. Format checks prove a
# cross-build, not that the program runs. Windows archives are checked against checksums.txt (Linux job);
# macOS archives against checksums-darwin.txt (macOS job). A missing archive
# fails.
check_platform_archive() {
    archive="$1" bin="$2" exe="$3" want="$4" sumfile="$5"
    CASE_FAILED=0
    name=$(basename "$archive")
    case "$archive" in
    *.zip) members=$(unzip -Z1 "$archive") || { fail "$name: cannot list"; return; } ;;
    *) members=$(tar -tzf "$archive") || { fail "$name: cannot list"; return; } ;;
    esac
    echo "$members" | grep -qx "$exe" || fail "$name: does not contain $exe"
    for other in $ALL_BINARIES; do
        [ "$other" = "$bin" ] && continue
        if echo "$members" | grep -qx "$other\(.exe\)\{0,1\}"; then
            fail "$name: contains unrelated binary $other"
        fi
    done
    got=$(awk -v n="$name" '$2 == n { print $1 }' "$sumfile")
    [ -n "$got" ] || fail "$name: not listed in $(basename "$sumfile")"
    if [ -n "$got" ] && [ "$got" != "$(sha256_file "$archive")" ]; then
        fail "$name: checksum differs from $(basename "$sumfile")"
    fi
    mkdir -p "$TMP/x/$name"
    case "$archive" in
    *.zip) unzip -q -o "$archive" -d "$TMP/x/$name" ;;
    *) tar -xzf "$archive" -C "$TMP/x/$name" ;;
    esac
    magic=$(file -b "$TMP/x/$name/$exe")
    case "$want" in
    windows-x64) echo "$magic" | grep -q "PE32+ .*x86-64" || fail "$name: not a Windows x64 executable: $magic" ;;
    windows-arm64) echo "$magic" | grep -q "PE32+ .*Aarch64" || fail "$name: not a Windows ARM64 executable: $magic" ;;
    macos-arm64) echo "$magic" | grep -q "Mach-O 64-bit .*arm64" || fail "$name: not a macOS arm64 executable: $magic" ;;
    esac
    if [ "$CASE_FAILED" -eq 0 ]; then pass "$name holds only $exe ($want), digest matches"; fi
}

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}'
    else shasum -a 256 "$1" | awk '{print $1}'; fi
}

windows() {
    for entry in $EXECUTABLES; do
        bin=${entry%%:*}
        for want in windows-x64:windows-amd64 windows-arm64:windows-arm64; do
            label=${want%%:*} os_arch=${want##*:}
            zip=$(only_match "$DIST/${bin}-[0-9]*-${os_arch}.zip") ||
                { fail "no single $bin archive for $label"; continue; }
            [ -f "$DIST/checksums.txt" ] || { fail "no checksums.txt in $DIST"; continue; }
            check_platform_archive "$zip" "$bin" "$bin.exe" "$label" "$DIST/checksums.txt"
        done
    done
    skip "Windows binaries are cross-built: format and digest checked, never run here"
}

macos() {
    for entry in $EXECUTABLES; do
        bin=${entry%%:*}
        tgz=$(only_match "$DIST/${bin}-[0-9]*-darwin-arm64.tar.gz") ||
            { fail "no single $bin archive for macos-arm64"; continue; }
        [ -f "$DIST/checksums-darwin.txt" ] || { fail "no checksums-darwin.txt in $DIST"; continue; }
        check_platform_archive "$tgz" "$bin" "$bin" macos-arm64 "$DIST/checksums-darwin.txt"
        # Run the role only where it can run: a macOS arm64 host.
        if [ "$(uname -s)-$(uname -m)" = "Darwin-arm64" ]; then
            if "$TMP/x/$(basename "$tgz")/$bin" --help >/dev/null 2>&1; then
                pass "$bin runs on this macOS arm64 host"
            else
                fail "$bin did not run on this macOS arm64 host"
            fi
        else
            skip "$bin not run: this host is not macOS arm64"
        fi
    done
}

docker_arch() {
    case "$(docker info --format '{{.Architecture}}')" in
    aarch64 | arm64) echo arm64 ;;
    x86_64 | amd64) echo amd64 ;;
    *) echo "unsupported Docker architecture" >&2; return 1 ;;
    esac
}

local_image() {
    docker image ls --format '{{.Repository}}:{{.Tag}}' "ghcr.io/naicoi92/$1" |
        grep -- "-$2\$" | head -n1 || true
}

# Install ONE package into an empty container, run its role via the alias, then
# remove it and check nothing is left behind.
runtime() {
    arch=$(docker_arch)
    for format in deb apk; do
        case "$format" in
        deb) image=debian:12-slim install="dpkg -i" remove="dpkg -r" ;;
        apk) image=alpine:3.21 install="apk add --allow-untrusted" remove="apk del" ;;
        esac
        for entry in $EXECUTABLES; do
            bin=${entry%%:*} link=${entry##*:}
            # The package manager's own database must name this package as the
            # owner of the binary and of the alias link.
            case "$format" in
            deb) owned="dpkg -S" own_re="^$bin: " ;;
            apk) owned="apk info -W" own_re="owned by $bin-[0-9]" ;;
            esac
            pkg=$(only_match "$DIST/${bin}_*_${arch}.$format") ||
                { fail "no $bin .$format for $arch"; continue; }
            if out=$(docker run --rm -v "$pkg:/pkg.$format:ro" "$image" sh -ec "
                $install /pkg.$format >/dev/null
                for other in $ALL_BINARIES; do
                    [ \$other = $bin ] && continue
                    if command -v \$other; then echo unexpected \$other; exit 1; fi
                done
                test \"\$(readlink \$(command -v $link))\" = $bin
                for path in /usr/bin/$bin /usr/bin/$link; do
                    $owned \$path | grep -q \"$own_re\"
                done
                $link --help >/dev/null
                $remove $bin >/dev/null
                hash -r
                if command -v $bin || command -v $link; then echo left behind; exit 1; fi
            " 2>&1); then
                pass "$pkg installs alone on $image, $link --help runs, removal is clean, package database owns binary and alias"
            else
                fail "$pkg on $image: $out"
            fi
        done
        coinstall "$format" "$image" "$install" "$remove" "$arch"
    done
    skip "runtime for architectures other than $arch (no native runner)"
}

# Install all three packages, remove one, and check the other two still work.
coinstall() {
    format="$1" image="$2" install="$3" remove="$4" arch="$5"
    mounts="" files=""
    for entry in $EXECUTABLES; do
        bin=${entry%%:*}
        pkg=$(only_match "$DIST/${bin}_*_${arch}.$format") ||
            { fail "no $bin .$format for $arch"; return; }
        mounts="$mounts -v $pkg:/$bin.$format:ro"
        files="$files /$bin.$format"
    done
    # shellcheck disable=SC2086 # mounts and files are space-separated on purpose
    if out=$(docker run --rm $mounts "$image" sh -ec "
        $install $files >/dev/null
        it --help >/dev/null; its --help >/dev/null; ita --help >/dev/null
        $remove iroh-tunnel-serve >/dev/null
        hash -r
        if command -v its || command -v iroh-tunnel-serve; then echo serve left; exit 1; fi
        it --help >/dev/null; ita --help >/dev/null
    " 2>&1); then
        pass "all three .$format packages co-install on $image; removing one leaves the others"
    else
        fail "co-install .$format on $image: $out"
    fi
}

images() {
    arch=$(docker_arch)
    for entry in $EXECUTABLES; do
        bin=${entry%%:*}
        img=$(local_image "$bin" "$arch")
        [ -n "$img" ] || { fail "no local $bin image for $arch"; continue; }
        entry_point=$(docker image inspect --format '{{json .Config.Entrypoint}}' "$img")
        case "$entry_point" in
        *"\"$bin\""*) ;;
        *) fail "$img: entrypoint $entry_point does not run $bin"; continue ;;
        esac
        # The image holds exactly one executable: list what it ships in bin dirs.
        cid=$(docker create "$img")
        CLEANUP_CONTAINERS="$cid"
        shipped=$(docker export "$cid" | tar -tf - | grep -E '(^|/)usr/(local/)?bin/[^/]+$' || true)
        docker rm "$cid" >/dev/null
        CLEANUP_CONTAINERS=""
        if [ "$shipped" != "usr/local/bin/$bin" ]; then
            fail "$img: bin dirs hold [$shipped], expected only usr/local/bin/$bin"
            continue
        fi
        if out=$(docker run --rm "$img" --help 2>&1); then
            pass "$img holds only $bin and runs it alone"
        else
            fail "$img: --help failed: $out"
        fi
    done
    skip "images for architectures other than $arch (no native runner)"
}

# Poll a container's log until it contains a string; fail after ~60s.
wait_log() {
    name="$1" needle="$2" tries=0
    until docker logs "$name" 2>&1 | grep -q -- "$needle"; do
        tries=$((tries + 1))
        [ "$tries" -lt 120 ] || return 1
        sleep 0.5
    done
}

# Start one role detached. SPEC is either an image reference (the entrypoint
# is the role) or "deb:<path>:<alias>" / "apk:<path>:<alias>", which installs
# that single package into a clean container and runs its alias.
start_role() {
    name="$1" net="$2" spec="$3"
    shift 3
    case "$spec" in
    deb:* | apk:*)
        format=${spec%%:*} rest=${spec#*:}
        pkg=${rest%:*} alias=${rest##*:}
        if [ "$format" = deb ]; then
            image=debian:12-slim install="dpkg -i /pkg.deb"
        else
            image=alpine:3.21 install="apk add --allow-untrusted /pkg.apk"
        fi
        docker run -d --name "$name" --network "$net" -v "$pkg:/pkg.$format:ro" "$image" \
            sh -c "$install >/dev/null && exec $alias \"\$@\"" sh "$@" >/dev/null
        ;;
    *) docker run -d --name "$name" --network "$net" "$spec" "$@" >/dev/null ;;
    esac
}

# One Serve and one Access, each started from its own SPEC (see start_role;
# optional 5th/6th arguments are the role subcommands of the all-in-one),
# reaching an echo target on a private Docker network through the public
# relays. The client must get its own bytes back. Everything is removed on
# return.
forward_case() {
    net="verify-dist-$$"
    docker network create "$net" >/dev/null
    CLEANUP_NETWORK="$net"
    CLEANUP_CONTAINERS="$net-target $net-serve $net-access"
    forward_exchange "$net" "$@"
    docker rm -f "$net-target" "$net-serve" "$net-access" >/dev/null 2>&1 || true
    docker network rm "$net" >/dev/null 2>&1 || true
    CLEANUP_NETWORK="" CLEANUP_CONTAINERS=""
}

forward_exchange() {
    net="$1" label="$2" serve_spec="$3" access_spec="$4" ok=0 out=""
    # The all-in-one executable selects its role with a subcommand.
    serve_role="${5:-}" access_role="${6:-}"
    docker run -d --name "$net-target" --network "$net" alpine:3.21 \
        sh -c 'nc -lk -p 9000 -e cat' >/dev/null
    target=$(docker inspect -f "{{(index .NetworkSettings.Networks \"$net\").IPAddress}}" "$net-target")
    # shellcheck disable=SC2086 # the role prefix is empty or one word
    start_role "$net-serve" "$net" "$serve_spec" $serve_role -n verify -t "$target:9000"
    if ! wait_log "$net-serve" "NodeId: "; then
        fail "$label: serve did not start: $(docker logs "$net-serve" 2>&1 | tail -n 5)"
        return
    fi
    node=$(docker logs "$net-serve" 2>&1 | sed -n 's/^NodeId: //p' | head -n1)
    # shellcheck disable=SC2086
    start_role "$net-access" "$net" "$access_spec" $access_role -n verify -l 0.0.0.0:7000 -N "$node"
    if ! wait_log "$net-access" "listening for local clients"; then
        fail "$label: access did not start: $(docker logs "$net-access" 2>&1 | tail -n 5)"
        return
    fi
    # The first dial can race relay registration; retry the whole exchange.
    for _ in 1 2 3 4 5 6; do
        out=$(docker run --rm --network "$net" alpine:3.21 \
            sh -c "echo verify-dist-$label | nc -w 10 $net-access 7000" 2>&1 || true)
        if [ "$out" = "verify-dist-$label" ]; then ok=1; break; fi
        sleep 2
    done
    if [ "$ok" -eq 1 ]; then
        pass "$label: client bytes returned unchanged through Serve and Access"
    else
        fail "$label: no echo through the tunnel (got [$out]); serve: $(docker logs "$net-serve" 2>&1 | tail -n 3)"
    fi
}

forward() {
    arch=$(docker_arch)
    serve_img=$(local_image iroh-tunnel-serve "$arch")
    access_img=$(local_image iroh-tunnel-access "$arch")
    if [ -n "$serve_img" ] && [ -n "$access_img" ]; then
        forward_case "role-images" "$serve_img" "$access_img"
    else
        fail "no local role images for $arch"
    fi
    aio_img=$(local_image iroh-tunnel "$arch")
    if [ -n "$aio_img" ]; then
        forward_case "all-in-one-image" "$aio_img" "$aio_img" serve access
    else
        fail "no local iroh-tunnel image for $arch"
    fi
    for format in deb apk; do
        aio_pkg=$(only_match "$DIST/iroh-tunnel_*_${arch}.$format") ||
            { fail "no all-in-one .$format for $arch"; continue; }
        forward_case "all-in-one-package-$format" "$format:$aio_pkg:it" "$format:$aio_pkg:it" serve access
    done
    for format in deb apk; do
        serve_pkg=$(only_match "$DIST/iroh-tunnel-serve_*_${arch}.$format") &&
            access_pkg=$(only_match "$DIST/iroh-tunnel-access_*_${arch}.$format") ||
            { fail "no role .$format packages for $arch"; continue; }
        forward_case "role-packages-$format" "$format:$serve_pkg:its" "$format:$access_pkg:ita"
    done
    skip "forward for architectures other than $arch (no native runner)"
}

case "$MODE" in
static) static ;;
runtime) runtime ;;
images) images ;;
forward) forward ;;
windows) windows ;;
macos) macos ;;
*) echo "unknown mode $MODE" >&2; exit 2 ;;
esac

exit "$FAILED"
