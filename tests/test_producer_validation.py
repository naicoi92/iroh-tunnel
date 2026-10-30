"""Nonpublishing production mode at GoReleaser/CLI boundary, not runtime proof."""

import io
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import yaml

import release_aliases as fixture


class ProducerValidationTest(unittest.TestCase):
    def test_production_snapshot_keeps_all_stages_and_never_forwards(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp).resolve() / 'production'
            checkout = Path(tmp).resolve() / 'checkout'
            checkout.mkdir()
            config = Path('.goreleaser.macos.yaml')
            original = yaml.safe_load(config.read_text())
            stale_target = Path(tmp) / 'stale-target'
            stale_target.mkdir()
            sentinel = stale_target / 'iroh-tunnel'
            sentinel.write_bytes(b'stale test-feature binary must never be copied')
            real_run = subprocess.run
            producer_commands = []

            def produce(command, **kwargs):
                if command[0] != 'goreleaser':
                    self.assertIn(Path(command[0]).name, fixture.ALIASES,
                                  'production must never invoke cargo forwarding')
                    return real_run(command, **kwargs)
                producer_commands.append(command)
                self.assertIn('--snapshot', command)
                self.assertFalse(any(arg.startswith('--skip') for arg in command))
                generated = yaml.safe_load((output / 'snapshot.yaml').read_text())
                self.assertEqual(generated, dict(original, dist=str(output / 'dist')))
                self.assertEqual(kwargs['env']['CARGO_TARGET_DIR'], str(checkout / 'target'))
                self.assertNotIn('CARGO_BUILD_TARGET_DIR', kwargs['env'])
                dist = output / 'dist'
                dist.mkdir()
                archive = dist / 'iroh-tunnel-1-darwin-arm64.tar.gz'
                with tarfile.open(archive, 'w:gz') as tar:
                    for alias, binary in fixture.ALIASES.items():
                        data = f'#!/bin/sh\nprintf "{binary} 1\\n"\n'.encode()
                        built = checkout / 'target/aarch64-apple-darwin/release' / binary
                        built.parent.mkdir(parents=True, exist_ok=True)
                        built.write_bytes(data)
                        member = tarfile.TarInfo(binary)
                        member.mode = 0o755
                        member.size = len(data)
                        tar.addfile(member, io.BytesIO(data))
                        link = tarfile.TarInfo(alias)
                        link.type = tarfile.SYMTYPE
                        link.linkname = binary
                        tar.addfile(link)
                (dist / 'checksums-darwin.txt').write_text(
                    f'{fixture.digest(archive)}  {archive.name}\n')
                return subprocess.CompletedProcess(command, 0)

            with patch.object(sys, 'argv', ['release_aliases.py', '--production',
                                           '--config', str(config), '--output', str(output)]), \
                 patch.dict(os.environ, CARGO_TARGET_DIR=str(stale_target), CARGO_BUILD_TARGET_DIR=str(stale_target)), \
                 patch.object(fixture.Path, 'cwd', return_value=checkout), \
                 patch.object(fixture.subprocess, 'run', side_effect=produce), \
                 patch.object(fixture.platform, 'system', return_value='Darwin'), \
                 patch.object(fixture.platform, 'machine', return_value='arm64'):
                fixture.main()
            self.assertEqual(len(producer_commands), 1)
            self.assertEqual(sentinel.read_bytes(), b'stale test-feature binary must never be copied')
            self.assertNotEqual(fixture.digest(output / 'isolated-0/it/iroh-tunnel'),
                                fixture.digest(sentinel))

    def test_target_override_and_config_cannot_redirect_cargo(self):
        with tempfile.TemporaryDirectory() as tmp:
            checkout = Path(tmp) / 'checkout'
            (checkout / '.cargo').mkdir(parents=True)
            for name in ('config', 'config.toml'):
                (checkout / '.cargo' / name).write_text('[build]\ntarget-dir = "/outside"\n')
            env = fixture.isolated_cargo_environment(checkout, {
                'CARGO_TARGET_DIR': '/outside', 'CARGO_BUILD_TARGET_DIR': '/outside'})
            self.assertEqual(env['CARGO_TARGET_DIR'], str(checkout.resolve() / 'target'))
            self.assertNotIn('CARGO_BUILD_TARGET_DIR', env)
            (checkout / 'target').symlink_to(Path(tmp), target_is_directory=True)
            with self.assertRaises(ValueError):
                fixture.isolated_cargo_environment(checkout, {})

    def test_actual_cargo_build_ignores_config_and_ambient_target(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp).resolve()
            checkout = root / 'checkout'
            (checkout / '.cargo').mkdir(parents=True)
            (checkout / 'src').mkdir()
            (checkout / 'Cargo.toml').write_text(
                '[package]\nname="isolation-check"\nversion="0.1.0"\nedition="2021"\n')
            (checkout / 'src/main.rs').write_text('fn main() {}\n')
            (checkout / '.cargo/config.toml').write_text(
                f'[build]\ntarget-dir = "{root / "config-target"}"\n')
            env = fixture.isolated_cargo_environment(checkout, dict(
                os.environ, CARGO_TARGET_DIR=str(root / 'ambient-target'),
                CARGO_BUILD_TARGET_DIR=str(root / 'build-target')))
            subprocess.run(['cargo', 'build', '--offline'], cwd=checkout, env=env,
                           check=True, capture_output=True)
            self.assertTrue((checkout / 'target/debug/isolation-check').is_file())
            for name in ('config-target', 'ambient-target', 'build-target'):
                self.assertFalse((root / name).exists())

    def test_archive_binary_contamination_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            extracted = root / 'extracted'
            extracted.mkdir()
            for binary in fixture.ALIASES.values():
                built = root / 'target/aarch64-apple-darwin/release' / binary
                built.parent.mkdir(parents=True, exist_ok=True)
                built.write_bytes(b'fresh')
                (extracted / binary).write_bytes(b'fresh')
            archive = Path('iroh-tunnel-1-darwin-arm64.tar.gz')
            fixture.verify_built_binaries(archive, extracted, root)
            (extracted / 'iroh-tunnel').write_bytes(b'stale')
            with self.assertRaises(ValueError):
                fixture.verify_built_binaries(archive, extracted, root)

    def test_production_stage_failure_cannot_be_reported_as_archive_only_success(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp).resolve() / 'production'
            with patch.object(fixture.Path, 'cwd', return_value=Path(tmp)), \
                 patch.object(sys, 'argv', ['release_aliases.py', '--production',
                                           '--config', '.goreleaser.yaml',
                                           '--output', str(output)]), \
                 patch.object(fixture.subprocess, 'run',
                              side_effect=subprocess.CalledProcessError(1, 'goreleaser')):
                with self.assertRaises(subprocess.CalledProcessError):
                    fixture.main()
            config = yaml.safe_load((output / 'snapshot.yaml').read_text())
            self.assertEqual(config, dict(yaml.safe_load(Path('.goreleaser.yaml').read_text()),
                                          dist=str(output / 'dist')))


if __name__ == '__main__':
    unittest.main()
