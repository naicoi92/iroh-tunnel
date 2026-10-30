#!/usr/bin/env python3
"""CLI-ALIAS-01: build actual Unix producer snapshot, inspect, extract, run.

Requires Python 3.12+, PyYAML, GoReleaser and producer's Rust/zig toolchain.
Default fixture enables test-utils for local relay only. Never distribute it.
--production preserves every producer stage and never enables test-utils or
runs forwarding. Both modes use --snapshot: no publication, no secrets needed.
Output (snapshot config, extracted archives and digests) stays in --output.
Run modes in separate clean source checkouts with initially empty target/:
GoReleaser's Rust builder copies from that default target path.
"""

import argparse
import hashlib
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import tarfile

import yaml


ALIASES = {"it": "iroh-tunnel", "its": "iroh-tunnel-serve", "ita": "iroh-tunnel-access"}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def validate_archive_set(config_path, archives):
    matrices = {
        ".goreleaser.yaml": {"linux-amd64", "linux-arm64", "linux-amd64-musl",
                               "linux-arm64-musl", "linux-riscv64-musl"},
        ".goreleaser.macos.yaml": {"darwin-arm64"},
    }
    expected = matrices.get(config_path.name)
    if expected is None:
        raise ValueError(f"unsupported producer config: {config_path}")
    pattern = re.compile(r"^iroh-tunnel-(.+)-(linux-(?:amd64|arm64|riscv64)(?:-musl)?|darwin-arm64)\.tar\.gz$")
    found = {}
    unexpected = []
    for archive in archives:
        match = pattern.fullmatch(archive.name)
        if not match or match.group(2) not in expected or match.group(2) in found:
            unexpected.append(archive.name)
        else:
            found[match.group(2)] = match.group(1)
    missing = expected - found.keys()
    if missing or unexpected or len(set(found.values())) > 1:
        raise ValueError(f"archive matrix mismatch: missing={sorted(missing)}, "
                         f"unexpected={unexpected}, versions={sorted(set(found.values()))}")


def linux_libc():
    # Unknown libc is not proof that either producer flavour can execute.
    name, _ = platform.libc_ver()
    if name == "glibc":
        return "gnu"
    if name == "musl":
        return "musl"
    try:
        result = subprocess.run(["ldd", "--version"], capture_output=True, text=True, check=False)
    except OSError:
        return None
    text = (result.stdout + result.stderr).lower()
    if "musl" in text:
        return "musl"
    if "glibc" in text or "gnu libc" in text:
        return "gnu"
    return None


def host_compatible(archive):
    host_os = {"Darwin": "darwin", "Linux": "linux"}.get(platform.system())
    host_arch = {"arm64": "arm64", "aarch64": "arm64", "x86_64": "amd64",
                 "riscv64": "riscv64"}.get(platform.machine())
    if not host_os or not host_arch:
        return False
    flavour = "-musl" if archive.name.endswith("-musl.tar.gz") else ""
    if not archive.name.endswith(f"-{host_os}-{host_arch}{flavour}.tar.gz"):
        return False
    if host_os == "linux":
        return linux_libc() == ("musl" if flavour else "gnu")
    return not flavour


def require_runtime_coverage(count):
    if not count:
        raise RuntimeError("no host-compatible archive executed alias forwarding; runtime coverage blocked")



def isolated_cargo_environment(checkout, environment):
    """Force Cargo and GoReleaser to the same initially empty checkout target."""
    checkout = checkout.resolve()
    target = checkout / "target"
    if target.exists() or target.is_symlink():
        raise ValueError(f"target already exists: {target}")
    env = dict(environment)
    env.pop("CARGO_BUILD_TARGET_DIR", None)
    env["CARGO_TARGET_DIR"] = str(target)
    return env


def verify_built_binaries(archive, extracted, checkout):
    triples = {
        "linux-amd64": "x86_64-unknown-linux-gnu",
        "linux-arm64": "aarch64-unknown-linux-gnu",
        "linux-amd64-musl": "x86_64-unknown-linux-musl",
        "linux-arm64-musl": "aarch64-unknown-linux-musl",
        "linux-riscv64-musl": "riscv64gc-unknown-linux-musl",
        "darwin-arm64": "aarch64-apple-darwin",
    }
    matches = [triple for suffix, triple in triples.items()
               if archive.name.endswith(f"-{suffix}.tar.gz")]
    if len(matches) != 1:
        raise ValueError(f"unknown archive target: {archive.name}")
    for binary in ALIASES.values():
        built = checkout / "target" / matches[0] / "release" / binary
        if not built.is_file() or digest(built) != digest(extracted / binary):
            raise ValueError(f"archive binary differs from checkout build: {archive.name}: {binary}")

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--production", action="store_true",
                        help="full unmodified producer except dist; inspect and --version only")
    args = parser.parse_args()
    checkout = Path.cwd().resolve()
    env = isolated_cargo_environment(checkout, os.environ)
    output = args.output.resolve()
    # Refuse reuse: no stale artifact can satisfy this test.
    output.mkdir(parents=True, exist_ok=False)
    config = yaml.safe_load(args.config.read_text())
    config["dist"] = str(output / "dist")
    if not args.production:
        for build in config["builds"]:
            build.setdefault("flags", ["--release"]).append("--features=test-utils")
    snapshot_config = output / "snapshot.yaml"
    snapshot_config.write_text(yaml.safe_dump(config, sort_keys=False))
    command = ["goreleaser", "release", "--snapshot", "--clean", "--config", str(snapshot_config),
               "--parallelism=1", "--timeout=1h"]
    if not args.production:
        command.append("--skip=before,homebrew,docker,nfpm,publish")
    print(f"producer cwd={Path.cwd()} command={command}", flush=True)
    subprocess.run(command, env=env, check=True)
    archives = sorted((output / "dist").glob("*.tar.gz"))
    validate_archive_set(args.config, archives)
    runtime_count = 0
    checksum_lines = []
    for checksum in (output / "dist").glob("checksums*.txt"):
        checksum_lines.extend(checksum.read_text().splitlines())
    for index, archive in enumerate(archives):
        sha = digest(archive)
        assert any(line.split() == [sha, archive.name] for line in checksum_lines), (
            f"producer checksum missing or incorrect: {archive.name}"
        )
        print(f"archive sha256={sha} {archive}", flush=True)
        extracted = output / f"extracted-{index}"
        with tarfile.open(archive) as tar:
            members = {member.name: member for member in tar.getmembers()}
            for alias, binary in ALIASES.items():
                assert alias in members, f"{archive.name}: missing alias {alias}"
                assert members[alias].issym(), f"{archive.name}: {alias} not symlink"
                assert members[alias].linkname == binary, f"{alias}: wrong relative target"
                assert members[binary].isfile(), f"{binary}: missing regular executable"
            tar.extractall(extracted, filter="data")
        verify_built_binaries(archive, extracted, checkout)
        fixture = output / f"isolated-{index}"
        for alias, binary in ALIASES.items():
            installed = fixture / alias
            installed.mkdir(parents=True)
            shutil.copy2(extracted / binary, installed / binary)
            # Preserve producer's symlink, never synthesize test-only aliases.
            shutil.copy2(extracted / alias, installed / alias, follow_symlinks=False)
            assert (installed / alias).is_symlink()
            assert (installed / alias).resolve() == installed / binary
            assert digest(installed / alias) == digest(extracted / binary)
            assert set(p.name for p in installed.iterdir()) == {alias, binary}
            print(f"{installed / alias} -> {os.readlink(installed / alias)} "
                  f"resolved sha256={digest(installed / alias)}", flush=True)
        if not host_compatible(archive):
            print(f"NOT RUN (foreign or unknown OS/architecture/libc): {archive.name}", flush=True)
            continue
        for alias, binary in ALIASES.items():
            installed = fixture / alias
            home = output / f"version-home-{index}-{alias}"
            home.mkdir()
            version = subprocess.check_output(
                [str(installed / alias), "--version"], cwd=installed,
                env=dict(os.environ, HOME=str(home), PATH=str(installed)), text=True,
            )
            assert version.startswith(f"{binary} "), f"{alias}: wrong binary version output"
            print(f"{alias} --version: {version.strip()}", flush=True)
        if args.production:
            continue
        fixture_env = dict(env, IROH_TUNNEL_ALIAS_FIXTURE=str(fixture))
        subprocess.run(
            ["cargo", "test", "--locked", "--test", "quick_run",
             "produced_archive_aliases_forward_tcp_independently", "--",
             "--ignored", "--exact", "--nocapture"],
            env=fixture_env, check=True,
        )
        runtime_count += 1
    if not args.production:
        require_runtime_coverage(runtime_count)
        print(f"alias forwarding PASS: {runtime_count} host-compatible archive(s)", flush=True)
    else:
        print("production snapshot and archive structure PASS; forwarding NOT RUN", flush=True)


if __name__ == "__main__":
    main()
