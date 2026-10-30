"""Nonpublishing production mode at GoReleaser/CLI boundary, not runtime proof."""

import io
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
            config = Path('.goreleaser.macos.yaml')
            original = yaml.safe_load(config.read_text())
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
                self.assertEqual(kwargs['env']['CARGO_TARGET_DIR'], str(output / 'target'))
                dist = output / 'dist'
                dist.mkdir()
                archive = dist / 'iroh-tunnel-1-darwin-arm64.tar.gz'
                with tarfile.open(archive, 'w:gz') as tar:
                    for alias, binary in fixture.ALIASES.items():
                        data = f'#!/bin/sh\nprintf "{binary} 1\\n"\n'.encode()
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
                 patch.object(fixture.subprocess, 'run', side_effect=produce), \
                 patch.object(fixture.platform, 'system', return_value='Darwin'), \
                 patch.object(fixture.platform, 'machine', return_value='arm64'):
                fixture.main()
            self.assertEqual(len(producer_commands), 1)

    def test_production_stage_failure_cannot_be_reported_as_archive_only_success(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp).resolve() / 'production'
            with patch.object(sys, 'argv', ['release_aliases.py', '--production',
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
