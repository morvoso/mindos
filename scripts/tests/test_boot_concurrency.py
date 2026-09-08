#!/usr/bin/env python3
"""Exercise boot-menu writers in a disposable tree, without root or host changes."""
import fcntl
import os
from pathlib import Path
import subprocess
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]


class BootConcurrency(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='mindos-boot-test-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.esp = self.root / 'boot'
        self.esp.mkdir()
        (self.root / 'snapshots').mkdir()
        self.lock = self.root / 'boot.lock'
        source = (ROOT / 'packages/mindos-base/mindos-boot').read_text()
        functions, dispatch = source.split('cmd=${1:-status}; shift || true', 1)
        # Preserve the actual writers, locking and command dispatch. Substitute
        # hardware discovery and destination paths only; never inspect host disks.
        overrides = '''
ESP="$BOOT_TEST_ROOT/boot"
STORE="$ESP/mindos"
SNAPDIR="$BOOT_TEST_ROOT/snapshots"
BOOT_LOCK="$BOOT_TEST_ROOT/boot.lock"
THEME="$BOOT_TEST_ROOT/no-theme"
need_root() { :; }
btrfs_uuid() { echo test-uuid; }
kernel_cmdline() { echo quiet; }
kernel_version() { echo test-kernel; }
'''
        functions = functions.replace('[[ -r $CONF ]] && . "$CONF"', ': # no host configuration')
        self.script = self.root / 'mindos-boot'
        self.script.write_text(functions + overrides + '\ncmd=${1:-status}; shift || true' + dispatch)
        self.env = dict(os.environ, BOOT_TEST_ROOT=str(self.root))

    def start(self, command):
        process = subprocess.Popen(['bash', str(self.script), command], env=self.env,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        self.addCleanup(lambda: process.poll() is None and process.kill())
        return process

    def assert_ok(self, process):
        out, err = process.communicate(timeout=10)
        self.assertEqual(process.returncode, 0, (out + err).decode())

    def test_config_and_sync_wait_for_existing_writer(self):
        with self.lock.open('w') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            processes = [self.start(command) for command in ('config', 'sync')]
            time.sleep(0.15)
            self.assertTrue(all(p.poll() is None for p in processes))
            self.assertFalse((self.esp / 'limine.conf').exists())
            fcntl.flock(lock, fcntl.LOCK_UN)
        for process in processes:
            self.assert_ok(process)

    def test_concurrent_config_and_sync_publish_complete_menu(self):
        processes = [self.start(command) for command in ('config', 'sync') * 8]
        for process in processes:
            self.assert_ok(process)
        menu = (self.esp / 'limine.conf').read_text()
        self.assertEqual(menu.count('\n/MindOS\n'), 1)
        self.assertIn('root=UUID=test-uuid rootflags=subvol=@ rw quiet\n', menu)
        self.assertTrue(menu.endswith('module_path: boot():/initramfs-linux-mindos.img\n'))
        self.assertFalse((self.esp / 'limine.conf.new').exists())


if __name__ == '__main__':
    unittest.main()
