#!/usr/bin/env python3
"""CLI-ALIAS-01: build actual Unix producer snapshot, inspect, extract, run.

Requires Python 3.12+, PyYAML, GoReleaser and producer's Rust/zig toolchain.
Fixture enables test-utils for isolated local relay only. Never distribute it.
Output (including modified snapshot config and digests) stays in --output.
"""

import argparse
import hashlib
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tarfile

import yaml


ALIASES = {"it": "iroh-tunnel", "its": "iroh-tunnel-serve", "ita": "iroh-tunnel-access"}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    output = args.output.resolve()
    # Refuse reuse: no stale artifact can satisfy this test.
    output.mkdir(parents=True, exist_ok=False)
    config = yaml.safe_load(args.config.read_text())
    config["dist"] = str(output / "dist")
    for build in config["builds"]:
        build.setdefault("flags", ["--release"]).append("--features=test-utils")
    snapshot_config = output / "snapshot.yaml"
    snapshot_config.write_text(yaml.safe_dump(config, sort_keys=False))
    subprocess.run(
        ["goreleaser", "release", "--snapshot", "--clean", "--config", str(snapshot_config),
         "--skip=before,homebrew,docker,nfpm,publish"],
        check=True,
    )
    archives = sorted((output / "dist").glob("*.tar.gz"))
    assert archives, "producer emitted no tar archives"
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
        host_os = {"Darwin": "darwin", "Linux": "linux"}[platform.system()]
        host_arch = {"arm64": "arm64", "aarch64": "arm64", "x86_64": "amd64",
                     "riscv64": "riscv64"}[platform.machine()]
        if f"-{host_os}-{host_arch}" not in archive.name:
            print(f"NOT RUN (foreign OS/architecture): {archive.name}", flush=True)
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
        env = dict(os.environ, IROH_TUNNEL_ALIAS_FIXTURE=str(fixture))
        subprocess.run(
            ["cargo", "test", "--locked", "--test", "quick_run",
             "produced_archive_aliases_forward_tcp_independently", "--",
             "--ignored", "--exact", "--nocapture"],
            env=env, check=True,
        )


if __name__ == "__main__":
    main()
