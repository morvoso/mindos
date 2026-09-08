#!/usr/bin/env python3
"""Windows integration regression checks with fake Wine, plus a real GIO launch."""
import importlib.machinery
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / 'packages/mindos-gaming/mindos-win'
loader = importlib.machinery.SourceFileLoader('winopen', str(SCRIPT.with_name('mindos-win-open')))
spec = importlib.util.spec_from_loader(loader.name, loader)
winopen = importlib.util.module_from_spec(spec)
loader.exec_module(winopen)


class WindowsApps(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='mindos windows ')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.data = self.root / 'data'
        self.bin = self.root / 'bin'
        self.bin.mkdir()
        self.log = self.root / 'wine.jsonl'
        self.env = dict(os.environ, XDG_DATA_HOME=str(self.data), MINDOS_WIN_ROOT=str(self.data / 'mindos/win'),
                        MINDOS_WIN_THEME=str(SCRIPT.with_name('mindos-win-theme.reg')), QA_WIN_LOG=str(self.log),
                        PATH=str(self.bin) + ':' + os.environ['PATH'])
        fake = self.bin / 'wine'
        fake.write_text('''#!/usr/bin/python3
import json, os, pathlib, sys
with open(os.environ['QA_WIN_LOG'], 'a') as f: f.write(json.dumps(dict(args=sys.argv[1:], prefix=os.environ.get('WINEPREFIX'))) + '\\n')
if 'wineboot' in sys.argv: pathlib.Path(os.environ['WINEPREFIX'], 'system.reg').touch()
''')
        fake.chmod(0o755)
        (self.bin / 'wineserver').symlink_to(fake)

    def run_cli(self, *args):
        return subprocess.run(['bash', str(SCRIPT), *args], env=self.env, text=True, capture_output=True)

    def test_names_metadata_and_prefix_validation(self):
        result = self.run_cli('create', 'My "Game" 1.2')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), 'my-game-1-2')
        prefix = self.data / 'mindos/win/my-game-1-2'
        self.assertEqual(json.loads((prefix / 'mindos-win.json').read_text())['name'], 'My "Game" 1.2')
        calls = self.log.read_text()
        self.assertNotEqual(self.run_cli('remove', '../../escape').returncode, 0)
        self.assertEqual(self.log.read_text(), calls, 'invalid prefixes cannot reach Wine or deletion')
        self.assertNotEqual(self.run_cli('theme', 'my-game-1-2', '--dpi', 'bad').returncode, 0)

    def test_quoted_wine_prefix_shortcuts_are_listed_and_removed(self):
        self.assertEqual(self.run_cli('create', 'Example').returncode, 0)
        prefix = self.data / 'mindos/win/example'
        entries = self.data / 'applications/wine/Programs'
        entries.mkdir(parents=True)
        entry = entries / 'Example.desktop'
        entry.write_text('[Desktop Entry]\nType=Application\nName=Example\nExec=env ' +
                         winopen.desktop_quote('WINEPREFIX=' + str(prefix)) + ' wine notepad.exe\n')
        self.assertIn('\tExample', self.run_cli('list').stdout)
        portable = winopen.shortcut('Example', 'example', self.root / 'game.exe', self.data)
        self.assertEqual(self.run_cli('remove', 'example').returncode, 0)
        self.assertFalse(entry.exists())
        self.assertFalse(portable.exists())
        self.assertFalse(prefix.exists())

    @unittest.skipUnless(shutil.which('gio'), 'needs GIO desktop-entry launcher')
    def test_portable_shortcut_round_trips_special_paths_through_gio(self):
        capture = self.root / 'argv.json'
        fake = self.bin / 'mindos-win'
        fake.write_text('#!/usr/bin/python3\nimport json, pathlib, sys\n' +
                        f'pathlib.Path({str(capture)!r}).write_text(json.dumps(sys.argv[1:]))\n')
        fake.chmod(0o755)
        program = self.root / 'a  b "quote" $value `tick` 100% back\\slash.exe'
        program.touch()
        entry = winopen.shortcut('Portable', 'portable', program, self.data)
        result = subprocess.run(['gio', 'launch', str(entry)], env=self.env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        for _ in range(100):
            if capture.exists():
                break
            time.sleep(.02)
        self.assertEqual(json.loads(capture.read_text()), ['run', 'portable', str(program)])


if __name__ == '__main__':
    unittest.main()
