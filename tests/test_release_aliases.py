"""Coverage for producer matrix and non-vacuous runtime selection."""

import hashlib
import io
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import release_aliases as fixture


class ReleaseAliasesTest(unittest.TestCase):
    def test_linux_archive_matrix_rejects_missing_and_extra_archives(self):
        names = [f"iroh-tunnel-1.0-linux-{suffix}.tar.gz" for suffix in (
            "amd64", "arm64", "amd64-musl", "arm64-musl", "riscv64-musl")]
        fixture.validate_archive_set(Path('.goreleaser.yaml'), list(map(Path, names)))
        with self.assertRaisesRegex(ValueError, "missing.*arm64-musl"):
            fixture.validate_archive_set(Path('.goreleaser.yaml'), list(map(Path, names[:3] + names[4:])))
        with self.assertRaisesRegex(ValueError, "unexpected"):
            fixture.validate_archive_set(Path('.goreleaser.yaml'), list(map(Path, names + ["extra.tar.gz"])))

    def test_macos_archive_matrix_rejects_missing_archive(self):
        with self.assertRaisesRegex(ValueError, "missing"):
            fixture.validate_archive_set(Path('.goreleaser.macos.yaml'), [])

    def test_foreign_only_archives_cannot_pass_runtime_coverage(self):
        archive = Path('iroh-tunnel-1.0-darwin-arm64.tar.gz')
        with patch.object(fixture.platform, 'system', return_value='Linux'), \
             patch.object(fixture.platform, 'machine', return_value='x86_64'), \
             patch.object(fixture, 'linux_libc', return_value='gnu'):
            self.assertFalse(fixture.host_compatible(archive))
            with self.assertRaisesRegex(RuntimeError, 'no host-compatible archive'):
                fixture.require_runtime_coverage(0)

    def test_foreign_only_complete_producer_run_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            config = root / '.goreleaser.macos.yaml'
            config.write_text('builds: []\n')
            output = root / 'output'

            def produce(*args, **kwargs):
                dist = output / 'dist'
                dist.mkdir()
                archive = dist / 'iroh-tunnel-1-darwin-arm64.tar.gz'
                with tarfile.open(archive, 'w:gz') as tar:
                    for alias, binary in fixture.ALIASES.items():
                        data = b'binary'
                        built = root / 'target/aarch64-apple-darwin/release' / binary
                        built.parent.mkdir(parents=True, exist_ok=True)
                        built.write_bytes(data)
                        member = tarfile.TarInfo(binary)
                        member.size = len(data)
                        tar.addfile(member, io.BytesIO(data))
                        link = tarfile.TarInfo(alias)
                        link.type = tarfile.SYMTYPE
                        link.linkname = binary
                        tar.addfile(link)
                sha = hashlib.sha256(archive.read_bytes()).hexdigest()
                (dist / 'checksums.txt').write_text(f'{sha}  {archive.name}\n')

            with patch.object(fixture.Path, 'cwd', return_value=root), \
                 patch.object(sys, 'argv', ['release_aliases.py', '--config', str(config),
                                           '--output', str(output)]), \
                 patch.object(fixture.subprocess, 'run', side_effect=produce) as run, \
                 patch.object(fixture.platform, 'system', return_value='Linux'), \
                 patch.object(fixture.platform, 'machine', return_value='x86_64'):
                with self.assertRaisesRegex(RuntimeError, 'no host-compatible archive'):
                    fixture.main()
                self.assertEqual(run.call_count, 1)  # producer only; no forwarding

    def test_linux_libc_and_arch_must_match(self):
        with patch.object(fixture.platform, 'system', return_value='Linux'), \
             patch.object(fixture.platform, 'machine', return_value='x86_64'), \
             patch.object(fixture, 'linux_libc', return_value='gnu'):
            self.assertTrue(fixture.host_compatible(Path('iroh-tunnel-1-linux-amd64.tar.gz')))
            self.assertFalse(fixture.host_compatible(Path('iroh-tunnel-1-linux-amd64-musl.tar.gz')))
            self.assertFalse(fixture.host_compatible(Path('iroh-tunnel-1-linux-arm64.tar.gz')))
        with patch.object(fixture.platform, 'system', return_value='Linux'), \
             patch.object(fixture.platform, 'machine', return_value='x86_64'), \
             patch.object(fixture, 'linux_libc', return_value='musl'):
            self.assertTrue(fixture.host_compatible(Path('iroh-tunnel-1-linux-amd64-musl.tar.gz')))
            self.assertFalse(fixture.host_compatible(Path('iroh-tunnel-1-linux-amd64.tar.gz')))


if __name__ == '__main__':
    unittest.main()
