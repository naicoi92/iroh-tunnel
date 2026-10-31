#!/usr/bin/env bash
# Render the Homebrew cask for a release to stdout.
#   usage: homebrew-cask.sh <version> <sha256-of-darwin-arm64-tarball>
#
# Replaces GoReleaser's `homebrew_casks` block (ADR-0002).
set -euo pipefail

version="${1:?version required}"
sha256="${2:?sha256 required}"

cat <<EOF
cask "iroh-tunnel" do
  version "${version}"
  sha256 "${sha256}"

  url "https://github.com/naicoi92/iroh-tunnel/releases/download/v#{version}/iroh-tunnel-#{version}-darwin-arm64.tar.gz"
  name "iroh-tunnel"
  desc "P2P port-forwarding tunnel (TCP/UDP) via Iroh."
  homepage "https://github.com/naicoi92/iroh-tunnel"

  depends_on arch: :arm64

  binary "iroh-tunnel"

  postflight do
    # Remove the quarantine attribute so Gatekeeper does not block the
    # unsigned (cosign is deferred) command-line binary.
    system_command "/usr/bin/xattr",
                   args: ["-dr", "com.apple.quarantine", "#{staged_path}/iroh-tunnel"]

    # \`it\` convenience alias. Runs after the \`binary\` stanza has linked
    # iroh-tunnel into Homebrew's bin dir, so the relative target resolves.
    system_command "/bin/ln",
                   args: ["-sf", "iroh-tunnel", "#{HOMEBREW_PREFIX}/bin/it"]
  end
end
EOF
