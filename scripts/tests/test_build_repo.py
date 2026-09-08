#!/usr/bin/env python3
"""Exercise repo-add/version selection with disposable package archives."""
import importlib.util
import io
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[1] / 'build-repo.py'
spec = importlib.util.spec_from_file_location('build_repo', SOURCE)
repo = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = repo
spec.loader.exec_module(repo)


@unittest.skipUnless(shutil.which('repo-add') and shutil.which('vercmp'), 'run inside the Arch build box')
class RepositoryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / 'archives'
        self.source.mkdir()
        self.output = self.root / 'repo'

    def package(self, name, version, *, filename=None, arch='any'):
        path = self.source / (filename or f'{name}-{version.replace(":", "_")}-{arch}.pkg.tar.zst')
        data = f'pkgname = {name}\npkgver = {version}\npkgdesc = QA fixture\narch = {arch}\nsize = 1\n'.encode()
        with tarfile.open(path, 'w:zst') as archive:
            member = tarfile.TarInfo('.PKGINFO')
            member.size = len(data)
            archive.addfile(member, io.BytesIO(data))
        return path

    def versions(self):
        result = {}
        with tarfile.open(self.output / 'mindos.db.tar.zst', 'r:zst') as archive:
            for member in archive:
                if member.name.endswith('/desc'):
                    sections = archive.extractfile(member).read().decode().split('\n\n')
                    fields = dict(section.strip().split('\n', 1) for section in sections if '\n' in section.strip())
                    result[fields['%NAME%']] = fields['%VERSION%']
                    self.assertTrue((self.output / fields['%FILENAME%']).is_file())
        return result

    def test_versions_use_pacman_semantics_and_keep_source_archives(self):
        for version in ('1.0-9', '1.0-10', '1.0-10.1'):
            self.package('shell', version)
        self.package('kernel', '99.0-1')
        self.package('kernel', '1:1.0-1')
        self.package('foreign', '1-1', arch='aarch64')
        before = {p.name: p.read_bytes() for p in self.source.iterdir()}
        repo.build(self.source, self.output)
        self.assertEqual(self.versions(), {'kernel': '1:1.0-1', 'shell': '1.0-10.1'})
        self.assertEqual(len(list(self.output.glob('*.pkg.tar.zst'))), 2)
        self.assertFalse(list(self.output.glob('*.old')))
        self.assertEqual(before, {p.name: p.read_bytes() for p in self.source.iterdir()})

    def test_success_replaces_previous_repository(self):
        self.package('example', '1-1')
        repo.build(self.source, self.output)
        self.package('example', '2-1')
        repo.build(self.source, self.output)
        self.assertEqual(self.versions(), {'example': '2-1'})
        self.assertEqual(len(list(self.output.glob('*.pkg.tar.zst'))), 1)
        self.assertFalse(list(self.root.glob('.repo-*/')))

    def test_failed_indexing_preserves_previous_repository(self):
        self.package('example', '1-1')
        repo.build(self.source, self.output)
        before = (self.output / 'mindos.db.tar.zst').read_bytes()
        self.package('example', '2-1')
        with patch.object(repo.subprocess, 'run', side_effect=subprocess.CalledProcessError(1, 'repo-add')):
            with self.assertRaises(subprocess.CalledProcessError):
                repo.build(self.source, self.output)
        self.assertEqual((self.output / 'mindos.db.tar.zst').read_bytes(), before)
        self.assertEqual(self.versions(), {'example': '1-1'})

    def test_corrupt_archive_preserves_previous_repository(self):
        self.package('example', '1-1')
        repo.build(self.source, self.output)
        (self.source / 'broken.pkg.tar.zst').write_bytes(b'not a package')
        with self.assertRaises(tarfile.TarError):
            repo.build(self.source, self.output)
        self.assertEqual(self.versions(), {'example': '1-1'})

    def test_duplicate_identity_is_rejected(self):
        self.package('example', '1-1', filename='a.pkg.tar.zst')
        self.package('example', '1-1', filename='b.pkg.tar.zst')
        with self.assertRaisesRegex(ValueError, 'duplicate archives'):
            repo.build(self.source, self.output)
        self.assertFalse(self.output.exists())

    def test_empty_input_does_not_replace_repository(self):
        self.output.mkdir()
        marker = self.output / 'existing'
        marker.write_text('keep')
        with self.assertRaisesRegex(ValueError, 'no packages'):
            repo.build(self.source, self.output)
        self.assertEqual(marker.read_text(), 'keep')


if __name__ == '__main__':
    unittest.main(verbosity=2)
