#!/usr/bin/env bash
# Render the Homebrew cask for a release to stdout.
#   usage: homebrew-cask.sh <version> <sha256-of-darwin-arm64-tarball> [combined|serve|access]
# Omitting the role preserves the combined cask.
set -euo pipefail

if (( $# < 2 || $# > 3 )); then
  echo "usage: $0 <version> <sha256> [combined|serve|access]" >&2
  exit 1
fi
version="${1:?version required}"
sha256="${2:?sha256 required}"
role="${3-combined}"

case "$role" in
  combined) name=iroh-tunnel; alias=it ;;
  serve) name=iroh-tunnel-serve; alias=its ;;
  access) name=iroh-tunnel-access; alias=ita ;;
  *) echo "unsupported cask role: $role" >&2; exit 1 ;;
esac

# Reject malformed values before interpolating them into Ruby or release URLs.
version_pattern='^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$'
if [[ ! "$version" =~ $version_pattern ]]; then
  echo "invalid version: expected X.Y.Z[-prerelease][+build]" >&2
  exit 1
fi
if [[ ! "$sha256" =~ ^[0-9a-fA-F]{64}$ ]]; then
  echo "invalid sha256: expected 64 hexadecimal characters" >&2
  exit 1
fi

cat <<EOF
cask "${name}" do
  version "${version}"
  sha256 "${sha256}"

  url "https://github.com/naicoi92/iroh-tunnel/releases/download/v#{version}/${name}-#{version}-darwin-arm64.tar.gz"
  name "${name}"
  desc "P2P port-forwarding tunnel (TCP/UDP) via Iroh."
  homepage "https://github.com/naicoi92/iroh-tunnel"

  depends_on arch: :arm64

  binary "${name}"

  postflight do
    # Remove the quarantine attribute so Gatekeeper does not block the
    # unsigned (cosign is deferred) command-line binary.
    system_command "/usr/bin/xattr",
                   args: ["-dr", "com.apple.quarantine", "#{staged_path}/${name}"]

    # \`${alias}\` convenience alias. Runs after the \`binary\` stanza has linked
    # ${name} into Homebrew's bin dir, so the relative target resolves.
    system_command "/bin/ln",
                   args: ["-sf", "${name}", "#{HOMEBREW_PREFIX}/bin/${alias}"]
  end

  # Remove the \`${alias}\` alias again so uninstall does not leave it dangling.
  uninstall_postflight do
    system_command "/bin/rm", args: ["-f", "#{HOMEBREW_PREFIX}/bin/${alias}"]
  end
end
EOF
