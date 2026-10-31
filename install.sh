#!/bin/sh
# Install the three iroh-tunnel binaries system-wide (issue #93).
#
#   curl -fsSL https://raw.githubusercontent.com/naicoi92/iroh-tunnel/main/install.sh | sudo sh
#
# Installs iroh-tunnel (alias it), iroh-tunnel-serve (alias its) and
# iroh-tunnel-access (alias ita) into /usr/local/bin. Aliases are symlinks.
#
# Options:
#   --version X      install release X instead of the latest stable release
#   --bin-dir DIR    install into DIR instead of /usr/local/bin
#   --profile-dir D  write the PATH snippet into D instead of /etc/profile.d
#   --yes            answer yes to every question
#   --dry-run        print what would be installed and change nothing
#   --help
#
# Environment:
#   IROH_TUNNEL_BASE_URL      release host, default
#                             https://github.com/naicoi92/iroh-tunnel/releases
#   IROH_TUNNEL_TARGET        override platform detection, e.g. linux-amd64 or
#                             linux-arm64-musl (used by tests)
#   IROH_TUNNEL_PROMPT_INPUT  read answers from this file, default /dev/tty
#
# Questions are asked before anything is changed. Replacing a different file
# and changing PATH both need a "y"; without an answer source the answer is no.
# Declining a replacement stops the install with nothing changed. Declining the
# PATH change installs the binaries and prints how to add the directory.
#
# Only checksums are verified. The checksum file comes from the same host as the
# archives, so it detects corruption but not a compromised host. Signature
# verification waits for the signed release artifacts (#100).
#
# The installer never registers, starts or changes a service.
#
# Linux only for now. Windows and macOS release binaries are tracked in #124;
# the PowerShell installer waits for them.

set -eu

DEFAULT_BASE_URL="https://github.com/naicoi92/iroh-tunnel/releases"
DEFAULT_BIN_DIR="/usr/local/bin"
DEFAULT_PROFILE_DIR="/etc/profile.d"
# executable:alias
ROLES="iroh-tunnel:it iroh-tunnel-serve:its iroh-tunnel-access:ita"

BASE_URL="${IROH_TUNNEL_BASE_URL:-$DEFAULT_BASE_URL}"
BASE_URL="${BASE_URL%/}"
VERSION=""
BIN_DIR="$DEFAULT_BIN_DIR"
PROFILE_DIR="$DEFAULT_PROFILE_DIR"
ASSUME_YES=0
DRY_RUN=0
TMP=""

say() { printf '%s\n' "$*"; }
warn() { printf '%s\n' "$*" >&2; }
die() { warn "error: $*"; exit 1; }

usage() {
    cat <<'USAGE'
Usage: install.sh [--version X] [--bin-dir DIR] [--profile-dir DIR] [--yes] [--dry-run]

Installs iroh-tunnel (alias it), iroh-tunnel-serve (alias its) and
iroh-tunnel-access (alias ita) into /usr/local/bin. Run with sudo.
Asks before replacing a different file or changing PATH.
See the comments at the top of the script for details.
USAGE
}

cleanup() { [ -z "$TMP" ] || rm -rf "$TMP"; }
trap cleanup EXIT
trap 'exit 130' INT TERM

while [ "$#" -gt 0 ]; do
    case "$1" in
        --version) [ "$#" -ge 2 ] || die "--version needs a value"; VERSION="${2#v}"; shift 2 ;;
        --bin-dir) [ "$#" -ge 2 ] || die "--bin-dir needs a value"; BIN_DIR="${2%/}"; shift 2 ;;
        --profile-dir) [ "$#" -ge 2 ] || die "--profile-dir needs a value"; PROFILE_DIR="${2%/}"; shift 2 ;;
        --yes|-y) ASSUME_YES=1; shift ;;
        --dry-run) DRY_RUN=1; shift ;;
        --help|-h) usage; exit 0 ;;
        *) die "unknown option: $1 (try --help)" ;;
    esac
done
[ -n "$BIN_DIR" ] || die "--bin-dir must not be empty"

# --- platform ---------------------------------------------------------------

detect_target() {
    if [ -n "${IROH_TUNNEL_TARGET:-}" ]; then
        echo "$IROH_TUNNEL_TARGET"
        return
    fi
    os=$(uname -s)
    case "$os" in
        Linux) ;;
        Darwin) die "macOS role binaries are not published yet (tracked in #124). Use the Homebrew cask for the all-in-one binary." ;;
        *) die "unsupported operating system: $os. Windows role binaries are tracked in #124." ;;
    esac
    case "$(uname -m)" in
        x86_64|amd64) arch=amd64 ;;
        aarch64|arm64) arch=arm64 ;;
        riscv64) arch=riscv64gc ;;
        *) die "unsupported CPU architecture: $(uname -m)" ;;
    esac
    libc=gnu
    if [ "$arch" = riscv64gc ]; then
        libc=musl # only a musl build is published for riscv64
    elif (ldd --version 2>&1 || true) | grep -qi musl; then
        libc=musl
    fi
    if [ "$libc" = musl ]; then echo "linux-$arch-musl"; else echo "linux-$arch"; fi
}

# --- helpers ----------------------------------------------------------------

have() { command -v "$1" >/dev/null 2>&1; }

fetch() { # url dest
    if have curl; then
        curl -fsSL --retry 2 -o "$2" "$1" || die "download failed: $1"
    elif have wget; then
        wget -q -O "$2" "$1" || die "download failed: $1"
    else
        die "curl or wget is required"
    fi
}

sha256_of() {
    if have sha256sum; then sha256sum "$1" | awk '{print $1}'
    elif have shasum; then shasum -a 256 "$1" | awk '{print $1}'
    else die "sha256sum or shasum is required"; fi
}

# Answers come from IROH_TUNNEL_PROMPT_INPUT or the terminal (stdin is the
# script itself under `curl | sh`). No readable source means "no".
PROMPT_OPEN=0
confirm() {
    [ "$ASSUME_YES" -eq 1 ] && return 0
    if [ "$PROMPT_OPEN" -eq 0 ]; then
        src="${IROH_TUNNEL_PROMPT_INPUT:-/dev/tty}"
        if ( : <"$src" ) 2>/dev/null; then
            exec 3<"$src"
            PROMPT_OPEN=1
        else
            PROMPT_OPEN=-1
        fi
    fi
    printf '%s [y/N] ' "$1" >&2
    if [ "$PROMPT_OPEN" -lt 0 ]; then
        warn "no terminal to ask; assuming no (use --yes to accept)"
        return 1
    fi
    ans=""
    read -r ans <&3 || true
    printf '\n' >&2
    case "$ans" in y|Y|yes|YES|Yes) return 0 ;; *) return 1 ;; esac
}

# --- plan -------------------------------------------------------------------

TARGET=$(detect_target)

# Release base URL for the requested version.
if [ -n "$VERSION" ]; then
    DOWNLOAD_URL="$BASE_URL/download/v$VERSION"
else
    DOWNLOAD_URL="$BASE_URL/latest/download"
fi

if [ "$DRY_RUN" -eq 1 ]; then
    say "Would install for $TARGET from $DOWNLOAD_URL into $BIN_DIR:"
    for role in $ROLES; do
        say "  ${role%%:*}  (alias ${role##*:})"
    done
    say "Would ask before replacing a different file or changing PATH. No service is touched."
    exit 0
fi

have tar || die "tar is required"

# Fail on permissions before downloading anything.
if [ -d "$BIN_DIR" ]; then
    [ -w "$BIN_DIR" ] || die "cannot write to $BIN_DIR. Rerun with sudo, or pass --bin-dir DIR."
else
    parent=$BIN_DIR
    while [ ! -d "$parent" ]; do parent=$(dirname "$parent"); done
    [ -w "$parent" ] || die "cannot create $BIN_DIR. Rerun with sudo, or pass --bin-dir DIR."
fi

TMP=$(mktemp -d)
fetch "$DOWNLOAD_URL/checksums.txt" "$TMP/checksums.txt"

if [ -z "$VERSION" ]; then
    VERSION=$(sed -n "s/^[0-9a-fA-F]\{64\} [ *]*iroh-tunnel-\([0-9][^ ]*\)-$TARGET\.tar\.gz\$/\1/p" "$TMP/checksums.txt" | head -n 1)
    [ -n "$VERSION" ] || die "the latest release has no iroh-tunnel archive for $TARGET"
fi

# --- download and verify (nothing installed yet) ------------------------------

expected_sum() { awk -v n="$1" '{ f=$2; sub(/^\*/, "", f) } f == n { print $1; exit }' "$TMP/checksums.txt"; }

mkdir -p "$TMP/stage"
for role in $ROLES; do
    exe="${role%%:*}"
    archive="$exe-$VERSION-$TARGET.tar.gz"
    want=$(expected_sum "$archive")
    [ -n "$want" ] || die "release $VERSION does not publish $archive"
    fetch "$DOWNLOAD_URL/$archive" "$TMP/$archive"
    got=$(sha256_of "$TMP/$archive")
    [ "$got" = "$want" ] || die "checksum mismatch for $archive (expected $want, got $got). Nothing was installed."
    mkdir -p "$TMP/stage/$exe"
    tar -xzf "$TMP/$archive" -C "$TMP/stage/$exe" || die "cannot extract $archive"
    [ -f "$TMP/stage/$exe/$exe" ] && [ ! -L "$TMP/stage/$exe/$exe" ] \
        || die "$archive does not contain the $exe executable"
done

# --- conflicts and PATH (still nothing installed) -----------------------------

CONFLICTS=""
for role in $ROLES; do
    exe="${role%%:*}"
    alias="${role##*:}"
    for path in "$BIN_DIR/$exe" "$BIN_DIR/$alias"; do
        [ -e "$path" ] || [ -L "$path" ] || continue
        [ ! -d "$path" ] || [ -L "$path" ] || die "$path is a directory; remove it and rerun."
        if [ "$path" = "$BIN_DIR/$alias" ]; then
            # An alias that already points at our binary is not a conflict.
            [ "$(readlink "$path" 2>/dev/null || true)" = "$exe" ] && continue
        else
            [ ! -L "$path" ] && [ "$(sha256_of "$path")" = "$(sha256_of "$TMP/stage/$exe/$exe")" ] && continue
        fi
        CONFLICTS="$CONFLICTS $path"
    done
done

PATH_SNIPPET="$PROFILE_DIR/iroh-tunnel.sh"
SNIPPET_BODY="export PATH=\"\$PATH:$BIN_DIR\""
CHANGE_PATH=0
case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) CHANGE_PATH=1 ;;
esac
if [ "$CHANGE_PATH" -eq 1 ] && [ -e "$PATH_SNIPPET" ] && [ "$(cat "$PATH_SNIPPET")" != "$SNIPPET_BODY" ]; then
    CONFLICTS="$CONFLICTS $PATH_SNIPPET"
fi

if [ -n "$CONFLICTS" ]; then
    say "These files already exist and differ from what this install would write:"
    for path in $CONFLICTS; do say "  $path"; done
    confirm "Replace them?" || die "declined; nothing was changed."
fi

DO_PATH=0
if [ "$CHANGE_PATH" -eq 1 ]; then
    if [ -d "$PROFILE_DIR" ] && [ -w "$PROFILE_DIR" ]; then
        say "$BIN_DIR is not on PATH."
        if confirm "Add it for all login shells by writing $PATH_SNIPPET?"; then DO_PATH=1; fi
    else
        say "$BIN_DIR is not on PATH and $PROFILE_DIR is not writable."
    fi
fi

# --- install ----------------------------------------------------------------

mkdir -p "$BIN_DIR"
for role in $ROLES; do
    exe="${role%%:*}"
    alias="${role##*:}"
    tmpbin="$BIN_DIR/.$exe.new.$$"
    cp "$TMP/stage/$exe/$exe" "$tmpbin"
    chmod 755 "$tmpbin"
    mv -f "$tmpbin" "$BIN_DIR/$exe"
    tmpln="$BIN_DIR/.$alias.new.$$"
    ln -s "$exe" "$tmpln"
    mv -f "$tmpln" "$BIN_DIR/$alias"
    say "installed $BIN_DIR/$exe (alias $alias)"
done

if [ "$DO_PATH" -eq 1 ]; then
    printf '%s\n' "$SNIPPET_BODY" >"$PATH_SNIPPET"
    say "wrote $PATH_SNIPPET; open a new login shell to use it."
elif [ "$CHANGE_PATH" -eq 1 ]; then
    say "PATH was not changed. Add $BIN_DIR to PATH yourself, e.g. export PATH=\"\$PATH:$BIN_DIR\""
fi

say "iroh-tunnel $VERSION installed. No service was registered or changed."
say "Note: only checksums were verified; signature verification is not available yet."
