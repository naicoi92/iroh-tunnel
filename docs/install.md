# Installing iroh-tunnel

iroh-tunnel ships three executables. Each one runs on its own; none needs
another at runtime.

| Executable | Alias | Roles |
|---|---|---|
| `iroh-tunnel` | `it` | Serve and Access (`it serve ...`, `it access ...`) |
| `iroh-tunnel-serve` | `its` | Serve only |
| `iroh-tunnel-access` | `ita` | Access only |

Release targets: Linux amd64/arm64 (glibc and musl), Linux riscv64gc (musl),
macOS arm64, Windows x64/ARM64. Intel macOS is not supported.

- [Shell installer (Linux, macOS)](#shell-installer-linux-macos)
- [PowerShell installer (Windows)](#powershell-installer-windows)
- [Homebrew (macOS)](#homebrew-macos)
- [Linux packages (`.deb`, `.apk`)](#linux-packages-deb-apk)
- [Container images](#container-images)
- [Release archives](#release-archives)
- [Build from source](#build-from-source)
- [Updating](#updating)
- [Verifying a release](#verifying-a-release)

## Shell installer (Linux, macOS)

```sh
curl -fsSL https://raw.githubusercontent.com/naicoi92/iroh-tunnel/main/install.sh | sudo sh
```

It installs all three executables and their alias symlinks into
`/usr/local/bin`. It detects glibc or musl on Linux and supports macOS on
Apple silicon.

| Option | Effect |
|---|---|
| `--version X` | Install release `X` instead of the latest stable release |
| `--bin-dir DIR` | Install into `DIR` (absolute path) |
| `--profile-dir DIR` | Write the `PATH` entry into `DIR` instead of `/etc/profile.d` (`/etc/paths.d` on macOS) |
| `--yes`, `-y` | Answer yes to every question |
| `--dry-run` | Print what would be installed and change nothing |

The installer asks before it replaces a different file or changes `PATH`.
Without a terminal the answer is no; pass `--yes` to accept. Declining a
replacement stops the install with nothing changed. To pass options through a
pipe, use `sh -s --`:

```sh
curl -fsSL https://raw.githubusercontent.com/naicoi92/iroh-tunnel/main/install.sh | sudo sh -s -- --version 0.5.0 --yes
```

The installer verifies each archive against the release's SHA-256 checksum
file. It does not verify the checksum file's signature (see
[Verifying a release](#verifying-a-release)). It never registers, starts or
changes a service.

## PowerShell installer (Windows)

From an elevated PowerShell:

```powershell
irm https://raw.githubusercontent.com/naicoi92/iroh-tunnel/main/install.ps1 -OutFile install.ps1
powershell -ExecutionPolicy Bypass -File install.ps1
```

It installs `iroh-tunnel.exe`, `iroh-tunnel-serve.exe` and
`iroh-tunnel-access.exe` into `$env:ProgramFiles\iroh-tunnel` (no short
aliases) and asks before adding that directory to the machine `PATH`.
Parameters: `-Version X`, `-BinDir DIR`, `-Yes`, `-DryRun`. Like the shell
installer it verifies checksums only and never touches a service. A running
executable cannot be replaced on Windows: stop it first.

## Homebrew (macOS)

```sh
brew tap naicoi92/tap https://github.com/naicoi92/homebrew-tap
brew install --cask iroh-tunnel
```

The cask installs the all-in-one `iroh-tunnel` and the `it` alias for Apple
silicon, and removes the quarantine attribute so Gatekeeper does not block
the unsigned binary. It is updated on stable releases only. Use
`brew upgrade --cask iroh-tunnel` to update.

## Linux packages (`.deb`, `.apk`)

Each executable has its own package. Packages are binary-only: an executable
and its alias link, with no service unit, config directory or maintainer
script. They share no files, so you can install any combination.

| Package | Built for | Architectures |
|---|---|---|
| `<executable>_<version>_<arch>.deb` | glibc (Debian, Ubuntu, ...) | `amd64`, `arm64` |
| `<executable>_<version>_<arch>.apk` | musl (Alpine) | `amd64`, `arm64`, `riscv64` |

`.deb` and `.apk` are not interchangeable: pick the one that matches the host
libc. The glibc builds need glibc 2.35 or newer.

```sh
VERSION=0.5.0
BASE=https://github.com/naicoi92/iroh-tunnel/releases/download/v$VERSION

# Debian / Ubuntu
curl -LO "$BASE/iroh-tunnel_${VERSION}_amd64.deb"
sudo dpkg -i "iroh-tunnel_${VERSION}_amd64.deb"

# Alpine
curl -LO "$BASE/iroh-tunnel_${VERSION}_amd64.apk"
sudo apk add --allow-untrusted "iroh-tunnel_${VERSION}_amd64.apk"
```

Replace `iroh-tunnel` with `iroh-tunnel-serve` or `iroh-tunnel-access` to
install a single role. To run a role at boot, register it with
[`iroh-tunnel <role> service install`](services.md).

## Container images

| Image | Entrypoint |
|---|---|
| `ghcr.io/naicoi92/iroh-tunnel` | `iroh-tunnel` |
| `ghcr.io/naicoi92/iroh-tunnel-serve` | `iroh-tunnel-serve` |
| `ghcr.io/naicoi92/iroh-tunnel-access` | `iroh-tunnel-access` |

```sh
docker run --rm ghcr.io/naicoi92/iroh-tunnel:v0.5.0 --version
```

Images are distroless, multi-arch (`linux/amd64`, `linux/arm64`) and built
from the same signed glibc binaries as the release archives. Tags:
`v0.5.0`, `0.5.0`, `v0.5`, and `latest` for stable releases.
Pre-release tags (`vX.Y.Z-rc.N`) never move `latest`.
See [Docker and Kubernetes](deployment.md) for running them.

## Release archives

Every release on the
[Releases page](https://github.com/naicoi92/iroh-tunnel/releases) has one
archive per executable and target, named
`<executable>-<version>-<target>.tar.gz` (`.zip` on Windows). Each archive
holds a single executable, the README and the licenses.

| Target | Platform |
|---|---|
| `linux-amd64`, `linux-arm64` | Linux, glibc |
| `linux-amd64-musl`, `linux-arm64-musl`, `linux-riscv64gc-musl` | Linux, static musl |
| `darwin-arm64` | macOS on Apple silicon |
| `windows-amd64`, `windows-arm64` | Windows |

The macOS and Windows binaries are not code-signed.

## Build from source

Requires Rust 1.91 or newer.

```sh
git clone https://github.com/naicoi92/iroh-tunnel && cd iroh-tunnel
cargo build --release
./target/release/iroh-tunnel --help
```

## Updating

Each executable can replace itself with a newer release:

```sh
iroh-tunnel update             # latest stable release
iroh-tunnel update --to 0.5.0  # a chosen release
its update                     # iroh-tunnel-serve
ita update                     # iroh-tunnel-access
```

`update` acts only on the executable you run; the all-in-one form is
`iroh-tunnel update`, not `iroh-tunnel serve update`. It:

- verifies the release's signed checksum file (keyless Sigstore, pinned to
  this repository's release workflow) and the archive's SHA-256 before it
  touches anything;
- stages the new file beside the old one and renames it into place, so a
  failure keeps the installed executable;
- never stops a running tunnel: restart it afterwards;
- does not downgrade unless you pass `--to`.

Installs owned by something else are sent back to their owner: Homebrew
(`brew upgrade`), a Linux package in `/usr/bin` (your package manager) and
containers (pull the new image). `--yes` replaces a package- or
Homebrew-owned file anyway, after a warning; containers are never updated in
place. `--insecure` (`-i`) skips signature and checksum verification and
prints a warning. A permission failure exits with code `3`.

## Verifying a release

Releases publish `checksums.txt` (Linux and Windows) and
`checksums-darwin.txt` (macOS), each signed keyless with cosign by the release
workflow. To check a download yourself:

```sh
VERSION=0.5.0
BASE=https://github.com/naicoi92/iroh-tunnel/releases/download/v$VERSION
curl -LO "$BASE/checksums.txt" -LO "$BASE/checksums.txt.sigstore.json"
cosign verify-blob checksums.txt \
  --bundle checksums.txt.sigstore.json \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  --certificate-identity "https://github.com/naicoi92/iroh-tunnel/.github/workflows/release.yml@refs/tags/v$VERSION"
sha256sum --ignore-missing -c checksums.txt
```

For macOS archives, use `checksums-darwin.txt` and
`checksums-darwin.txt.sigstore.json` the same way (`shasum -a 256 -c` on
macOS).
