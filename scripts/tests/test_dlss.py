#!/usr/bin/env python3
"""Swapper data-integrity regressions. Temporary synthetic DLLs; no network/games."""
import importlib.machinery
import importlib.util
import contextlib
import io
import json
import os
from pathlib import Path
import stat
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
HELPER = ROOT / 'packages/mindos-gaming/mindos-dlss'
loader = importlib.machinery.SourceFileLoader('dlss_test_module', str(HELPER))
spec = importlib.util.spec_from_loader(loader.name, loader)
d = importlib.util.module_from_spec(spec)
loader.exec_module(d)


def dll(version, body=b'fixture'):
    a, b, c, e = map(int, version.split('.'))
    return b'MZ' + body + d.VS_KEY + d.VS_SIGNATURE + struct.pack('<III', 0x10000, a << 16 | b, c << 16 | e)


class SwapperTests(unittest.TestCase):
    def test_empty_library_cli_emits_only_json(self):
        output = io.StringIO()
        with patch.object(d, 'library', return_value=[]), patch.object(sys, 'argv', ['mindos-dlss', '--json', 'library']), contextlib.redirect_stdout(output):
            self.assertEqual(d.main(), 0)
        self.assertEqual(json.loads(output.getvalue()), [])

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        for name, value in [('DATA_DIR', self.root / 'data'), ('SWAPS_FILE', self.root / 'data/swaps.json'),
                            ('DRIVER_DIR', self.root / 'driver')]:
            p = patch.object(d, name, value)
            p.start()
            self.addCleanup(p.stop)
        self.game_dir = self.root / 'game'
        self.game_dir.mkdir()
        self.file = self.game_dir / 'nvngx_dlss.dll'
        self.original = dll('1.0.0.0')
        self.file.write_bytes(self.original)
        self.file.chmod(0o640)
        self.source = self.root / 'nvngx_dlss.dll'
        self.source.write_bytes(dll('2.0.0.0'))
        d.import_dll(self.source)
        self.backup = Path(str(self.file) + d.BACKUP_SUFFIX)

    def game(self):
        return {'id': 'fixture', 'name': 'Fixture', 'path': str(self.game_dir),
                'dlls': d.scan_dir(self.game_dir, d.load_json(d.SWAPS_FILE, {}))}

    def test_swap_and_restore_preserve_original_and_permissions(self):
        d.swap(self.game(), 'dlss', '2.0.0.0')
        self.assertEqual(self.file.read_bytes(), self.source.read_bytes())
        self.assertEqual(self.backup.read_bytes(), self.original)
        self.assertEqual(stat.S_IMODE(self.file.stat().st_mode), 0o640)
        self.assertTrue(self.game()['dlls'][0]['restorable'])
        d.restore(self.game())
        self.assertEqual(self.file.read_bytes(), self.original)
        self.assertFalse(self.game()['dlls'][0]['swapped'])
        self.assertEqual(json.loads(d.SWAPS_FILE.read_text()), {})

    def test_repeated_swaps_keep_first_original(self):
        d.swap(self.game(), 'dlss', '2.0.0.0')
        self.source.write_bytes(dll('3.0.0.0'))
        d.import_dll(self.source)
        d.swap(self.game(), 'dlss', '3.0.0.0')
        self.assertEqual(self.backup.read_bytes(), self.original)
        d.restore(self.game())
        self.assertEqual(self.file.read_bytes(), self.original)

    def test_update_is_not_overwritten_by_restore_and_becomes_new_original_on_reapply(self):
        d.swap(self.game(), 'dlss', '2.0.0.0')
        updated = dll('1.1.0.0', b'launcher update')
        self.file.write_bytes(updated)
        self.assertTrue(self.game()['dlls'][0]['changed'])
        self.assertFalse(self.game()['dlls'][0]['restorable'])
        with self.assertRaisesRegex(d.Fail, 'changed since the swap'):
            d.restore(self.game())
        self.assertEqual(self.file.read_bytes(), updated)
        d.swap(self.game(), 'dlss', '2.0.0.0')
        self.assertEqual(self.backup.read_bytes(), updated)
        self.assertEqual(len(list(self.game_dir.glob('*.mindos-orig.*'))), 1)
        self.assertEqual(next(self.game_dir.glob('*.mindos-orig.*')).read_bytes(), self.original)
        d.restore(self.game())
        self.assertEqual(self.file.read_bytes(), updated)

    def test_failed_staging_never_truncates_game_file(self):
        def fail_copy(src, out):
            out.write(b'partial')
            raise OSError('disk full')
        with patch.object(d.shutil, 'copyfileobj', side_effect=fail_copy):
            with self.assertRaisesRegex(d.Fail, 'disk full'):
                d.swap(self.game(), 'dlss', '2.0.0.0')
        self.assertEqual(self.file.read_bytes(), self.original)
        self.assertFalse(self.backup.exists())
        self.assertEqual(list(self.game_dir.glob('.*')), [])

    def test_failed_record_write_leaves_game_intact(self):
        with patch.object(d, 'save_swaps', side_effect=OSError('record write failed')):
            with self.assertRaisesRegex(d.Fail, 'record write failed'):
                d.swap(self.game(), 'dlss', '2.0.0.0')
        self.assertEqual(self.file.read_bytes(), self.original)
        self.assertEqual(self.backup.read_bytes(), self.original)

    def test_failure_between_record_and_replace_remains_restorable(self):
        replace = os.replace
        def fail_game(src, dst):
            if Path(dst) == self.file:
                raise OSError('replacement failed')
            return replace(src, dst)
        with patch.object(d.os, 'replace', side_effect=fail_game):
            with self.assertRaisesRegex(d.Fail, 'replacement failed'):
                d.swap(self.game(), 'dlss', '2.0.0.0')
        self.assertEqual(self.file.read_bytes(), self.original)
        self.assertFalse(self.game()['dlls'][0]['changed'])
        d.restore(self.game())
        self.assertEqual(self.file.read_bytes(), self.original)

    def test_restore_can_recover_after_its_record_write_failed(self):
        d.swap(self.game(), 'dlss', '2.0.0.0')
        with patch.object(d, 'save_swaps', side_effect=OSError('record write failed')):
            with self.assertRaises(d.Fail):
                d.restore(self.game())
        self.assertEqual(self.file.read_bytes(), self.original)
        self.assertFalse(self.game()['dlls'][0]['changed'])
        self.assertFalse(self.game()['dlls'][0]['swapped'])
        d.restore(self.game())

    def test_changed_backup_is_rejected(self):
        d.swap(self.game(), 'dlss', '2.0.0.0')
        self.backup.write_bytes(b'broken backup')
        with self.assertRaisesRegex(d.Fail, 'backup changed'):
            d.restore(self.game())
        self.assertEqual(self.file.read_bytes(), self.source.read_bytes())
        with self.assertRaisesRegex(d.Fail, 'backup changed'):
            d.swap(self.game(), 'dlss', '2.0.0.0')

    def test_missing_original_is_not_recreated_from_swapped_file(self):
        d.swap(self.game(), 'dlss', '2.0.0.0')
        self.backup.unlink()
        with self.assertRaisesRegex(d.Fail, 'backup is missing'):
            d.swap(self.game(), 'dlss', '2.0.0.0')
        self.assertFalse(self.backup.exists())
        self.assertEqual(self.file.read_bytes(), self.source.read_bytes())

    def test_game_and_backup_symlinks_are_not_written(self):
        target = self.root / 'unrelated'
        target.write_bytes(b'unrelated')
        game = self.game()
        self.file.unlink()
        self.file.symlink_to(target)
        with self.assertRaisesRegex(d.Fail, 'regular file'):
            d.swap(game, 'dlss', '2.0.0.0')
        self.file.unlink()
        self.file.write_bytes(self.original)
        self.backup.symlink_to(target)
        with self.assertRaisesRegex(d.Fail, 'regular file'):
            d.swap(self.game(), 'dlss', '2.0.0.0')
        self.assertEqual(target.read_bytes(), b'unrelated')

    def test_bad_version_cannot_escape_library(self):
        for value in ('../../elsewhere', '/tmp/data', '', '..', '1/2', None):
            with self.assertRaises(d.Fail):
                d.lib_dir('dlss', value)

    def test_changed_library_is_rejected(self):
        (d.lib_dir('dlss', '2.0.0.0') / 'nvngx_dlss.dll').write_bytes(b'corrupt')
        with self.assertRaisesRegex(d.Fail, 'Library DLL changed'):
            d.swap(self.game(), 'dlss', '2.0.0.0')
        self.assertEqual(self.file.read_bytes(), self.original)

    def test_cli_writer_lock_rejects_overlap(self):
        env = dict(os.environ, XDG_DATA_HOME=str(self.root / 'xdg'))
        with patch.object(d, 'DATA_DIR', self.root / 'xdg/mindos/dlss'):
            with d.mutation_lock():
                result = subprocess.run([sys.executable, str(HELPER), '--json', 'import', str(self.source)],
                                        env=env, capture_output=True, text=True, timeout=5)
        self.assertEqual(result.returncode, 1)
        self.assertIn('Another upscaler change', json.loads(result.stdout)['error'])

    def test_directory_sync_failure_reports_the_published_file(self):
        calls = 0
        original_sync = d.sync_directory
        def fail_sync(path):
            nonlocal calls
            if Path(path) == self.game_dir:
                calls += 1
                if calls == 2:
                    raise OSError('directory sync failed')
            original_sync(path)
        with patch.object(d, 'sync_directory', side_effect=fail_sync):
            with self.assertRaisesRegex(d.Fail, r'1 DLL\(s\) changed'):
                d.swap(self.game(), 'dlss', '2.0.0.0')
        self.assertEqual(self.file.read_bytes(), self.source.read_bytes())
        d.restore(self.game())
        self.assertEqual(self.file.read_bytes(), self.original)

    def test_legacy_record_uses_library_identity_for_restore(self):
        self.backup.write_bytes(self.original)
        self.file.write_bytes(self.source.read_bytes())
        d.save_swaps({str(self.file): {'kind': 'dlss', 'from_version': '1.0.0.0', 'to_version': '2.0.0.0'}})
        self.assertTrue(self.game()['dlls'][0]['restorable'])
        d.restore(self.game())
        self.assertEqual(self.file.read_bytes(), self.original)

    def test_corrupt_history_does_not_reset_original_tracking(self):
        d.swap(self.game(), 'dlss', '2.0.0.0')
        d.SWAPS_FILE.write_text('{broken')
        with self.assertRaisesRegex(d.Fail, 'Cannot read swap history'):
            d.swap(self.game(), 'dlss', '2.0.0.0')
        self.assertEqual(self.backup.read_bytes(), self.original)
        d.SWAPS_FILE.write_text(json.dumps({str(self.file): 'invalid record'}))
        with self.assertRaisesRegex(d.Fail, 'Invalid swap history'):
            d.load_swaps()

    def test_failure_on_second_dll_reports_and_tracks_first_change(self):
        second = self.game_dir / 'subdir/nvngx_dlss.dll'
        second.parent.mkdir()
        second.write_bytes(self.original)
        replace = os.replace
        def fail_second(src, dst):
            if Path(dst) == second:
                raise OSError('second DLL cannot be replaced')
            return replace(src, dst)
        with patch.object(d.os, 'replace', side_effect=fail_second):
            with self.assertRaisesRegex(d.Fail, r'1 DLL\(s\) changed'):
                d.swap(self.game(), 'dlss', '2.0.0.0')
        self.assertEqual(self.file.read_bytes(), self.source.read_bytes())
        self.assertEqual(second.read_bytes(), self.original)
        d.restore(self.game())
        self.assertEqual(self.file.read_bytes(), self.original)
        self.assertEqual(second.read_bytes(), self.original)

    def test_atomic_record_failure_preserves_previous_json(self):
        d.save_swaps({'before': 1})
        with patch.object(d.os, 'replace', side_effect=OSError('failed replace')):
            with self.assertRaises(OSError):
                d.save_swaps({'after': 2})
        self.assertEqual(json.loads(d.SWAPS_FILE.read_text()), {'before': 1})

if __name__ == '__main__':
    unittest.main()
