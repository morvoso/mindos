#!/usr/bin/env python3
"""Real pacman dependency resolution against a disposable tiny repository.

Run in the build box (Python 3.14, pacman and repo-add). No packages are installed.
"""
import importlib.util
import io
import os
import shutil
import sys
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('install_packages', ROOT / 'packages/mindos-install/install-packages.py')
packages = importlib.util.module_from_spec(spec)
spec.loader.exec_module(packages)


@unittest.skipUnless(os.geteuid() == 0 and sys.version_info >= (3, 14) and shutil.which('repo-add'),
                     'Run with scripts/buildbox.sh --root (Python 3.14 and pacman)')
class PacmanResolutionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='mindos-pacman-test-')
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        archives = []
        for name in ['base', packages.PREBUILT, 'nvidia-utils', *packages.DKMS]:
            version = '2-1' if name == 'nvidia-utils' else '1-1'
            dependency = 'depend = nvidia-utils=1\n' if name == packages.PREBUILT else ''
            if name == 'nvidia-open-dkms':
                dependency = 'depend = nvidia-utils=2\ndepend = dkms\n'
            content = (f'pkgname = {name}\npkgver = {version}\npkgdesc = QA fixture\n'
                       f'arch = x86_64\nsize = 0\n{dependency}').encode()
            archive_path = root / f'{name}-{version}-x86_64.pkg.tar.zst'
            with tarfile.open(archive_path, 'w:zst') as archive:
                member = tarfile.TarInfo('.PKGINFO')
                member.size = len(content)
                archive.addfile(member, io.BytesIO(content))
            archives.append(str(archive_path))
        subprocess.run(['repo-add', '--quiet', str(root / 'qa.db.tar.zst'), *archives],
                       check=True, capture_output=True)
        self.config = root / 'pacman.conf'
        self.config.write_text(f'[options]\nArchitecture = x86_64\nSigLevel = Never\n'
                               f'[qa]\nServer = file://{root}\n')

    def test_real_version_mismatch_falls_back(self):
        result = packages.resolve(['base', packages.PREBUILT, 'nvidia-utils'], str(self.config))
        self.assertEqual(result['nvidia_modules'], 'dkms')
        self.assertIn('could not satisfy dependencies', result['fallback_reason'])
        self.assertNotIn(packages.PREBUILT, result['packages'])

    def test_real_pinned_failure_aborts(self):
        with self.assertRaisesRegex(ValueError, 'no disk changes'):
            packages.resolve([packages.PREBUILT], str(self.config), 'prebuilt')

    def test_real_missing_base_cannot_hide_behind_live_database(self):
        with self.assertRaisesRegex(ValueError, 'target not found'):
            packages.resolve(['mindos-absent-qa-package'], str(self.config))


if __name__ == '__main__':
    unittest.main()
