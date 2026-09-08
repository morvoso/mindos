#!/usr/bin/env python3
"""Repository-install regressions with fake package managers, never host installs."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class RepositoryInstallTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.log = self.root / 'calls.jsonl'
        self.state = self.root / 'installed.json'
        self.state.write_text('{}')
        source = (ROOT / 'packages/mindos-mind/mindos-pkg').read_text()
        self.script = self.root / 'mindos-pkg'
        # Only bypass the root check in this disposable copy. Every package
        # manager on its PATH below is a fake that writes inside this temp dir.
        self.script.write_text(source.replace('[[ $EUID -eq 0 ]]', 'true'))
        self.env = dict(os.environ, PATH=f'{self.root}:{os.environ["PATH"]}',
                        QA_PKG_LOG=str(self.log), QA_PKG_STATE=str(self.state),
                        MINDOS_PKG_CONF=str(self.root / 'pkg.conf'))
        fake = self.root / 'pacman'
        fake.write_text('''#!/usr/bin/env python3
import json, os, pathlib, sys
a = sys.argv[1:]
with open(os.environ['QA_PKG_LOG'], 'a') as log: log.write(json.dumps([pathlib.Path(sys.argv[0]).name, *a]) + '\\n')
if pathlib.Path(sys.argv[0]).name != 'pacman': sys.exit(77)
p = pathlib.Path(os.environ['QA_PKG_STATE'])
installed = json.loads(p.read_text())
if a[0] == '-Q':
    name = a[-1]
    if name not in installed: sys.exit(1)
    print(name, installed[name])
elif a[0] == '-Si': print('Repository : mindos')
elif a[0] == '-Syu':
    if os.environ.get('QA_PKG_FAIL'):
        print('mirror unavailable', file=sys.stderr); sys.exit(1)
    for name in a[a.index('--') + 1:]: installed[name] = '2-1'
    p.write_text(json.dumps(installed))
else: sys.exit(88)
''')
        fake.chmod(0o755)
        for name in ('flatpak', 'paru', 'curl'):
            (self.root / name).symlink_to(fake)

    def run_install(self, *args):
        return subprocess.run(['bash', str(self.script), 'install', *args], env=self.env,
                              capture_output=True, text=True)

    def calls(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []

    def test_repository_packages_share_one_full_upgrade_transaction(self):
        result = self.run_install('--repo-only', 'nodejs', 'npm')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        writes = [call for call in self.calls() if call[1] == '-Syu']
        self.assertEqual(writes, [['pacman', '-Syu', '--needed', '--noconfirm', '--', 'nodejs', 'npm']])
        self.assertTrue(all(call[0] == 'pacman' for call in self.calls()))
        self.assertEqual(json.loads(self.state.read_text()), {'nodejs': '2-1', 'npm': '2-1'})

    def test_already_installed_packages_do_not_trigger_an_upgrade(self):
        self.state.write_text('{"nodejs":"1-1"}')
        result = self.run_install('--repo-only', 'nodejs')
        self.assertEqual(result.returncode, 0)
        self.assertIn('already installed (1-1)', result.stdout)
        self.assertFalse(any(call[1].startswith('-S') for call in self.calls()))

    def test_failed_transaction_does_not_fall_back_to_other_sources(self):
        self.env['QA_PKG_FAIL'] = '1'
        result = self.run_install('--repo-only', '--aur', 'mindos-gaming')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('mirror unavailable', result.stdout)
        self.assertEqual(json.loads(self.state.read_text()), {})
        self.assertTrue(all(call[0] == 'pacman' for call in self.calls()))

    def test_validate_entire_request_before_starting_transaction(self):
        result = self.run_install('--repo-only', 'nodejs', '../bad')
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.calls())

    def test_regular_repository_resolution_also_uses_full_upgrade(self):
        result = self.run_install('nodejs')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual([call for call in self.calls() if call[1].startswith('-S')],
                         [['pacman', '-Si', '--', 'nodejs'],
                          ['pacman', '-Syu', '--needed', '--noconfirm', '--', 'nodejs'],
                          ['pacman', '-Si', '--', 'nodejs']])


if __name__ == '__main__':
    unittest.main(verbosity=2)
