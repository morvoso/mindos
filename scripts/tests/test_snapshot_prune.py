#!/usr/bin/env python3
"""Root-only integration tests on a fresh temporary Btrfs image, never /.

Run in the development VM: python3 ~/mindos/scripts/tests/test_snapshot_prune.py
Requires btrfs-progs (including its Python bindings), mount and loop support.
"""

import importlib.machinery
import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import btrfsutil

sys.dont_write_bytecode = True  # The VM sees the shared source tree as root.
SOURCE = Path(__file__).resolve().parents[2] / "packages/mindos-base/snapshot-prune"
loader = importlib.machinery.SourceFileLoader("snapshot_prune", str(SOURCE))
spec = importlib.util.spec_from_loader(loader.name, loader)
pruner = importlib.util.module_from_spec(spec)
loader.exec_module(pruner)


class SnapshotPruneTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if os.geteuid() != 0:
            raise unittest.SkipTest("requires root for a disposable loop-mounted Btrfs image")
        cls.temp = tempfile.TemporaryDirectory(prefix="mindos-btrfs-test-", dir="/var/tmp")
        cls.addClassCleanup(cls.temp.cleanup)
        temp = Path(cls.temp.name)
        image = temp / "disposable.img"
        with image.open("wb") as out:
            out.truncate(512 * 1024 * 1024)
        subprocess.run(["mkfs.btrfs", "-q", str(image)], check=True)
        cls.mount = temp / "fs"
        cls.mount.mkdir()
        subprocess.run(["mount", "-o", "loop,nodev,nosuid", str(image), str(cls.mount)], check=True)
        cls.addClassCleanup(subprocess.run, ["umount", str(cls.mount)], check=True)

    def setUp(self):
        parent = self.mount / self._testMethodName
        parent.mkdir()
        self.snapshot = parent / "snapshot"
        btrfsutil.create_subvolume(self.snapshot)
        (parent / "info.xml").write_text(
            '<snapshot><type>single</type><description>the system before restoring #1</description></snapshot>'
        )
        (self.snapshot / "var/lib").mkdir(parents=True)
        (self.snapshot / "system-file").write_text("preserve until Snapper deletes this snapshot")
        self.children = [self.snapshot / "var/lib" / name for name in ("machines", "portables")]
        for child in self.children:
            btrfsutil.create_subvolume(child)
        btrfsutil.set_subvolume_read_only(self.snapshot, True)

    def assert_protected(self):
        self.assertTrue(btrfsutil.get_subvolume_read_only(self.snapshot))
        self.assertTrue(all(child.exists() for child in self.children))
        self.assertTrue(all(not btrfsutil.get_subvolume_read_only(child) for child in self.children))
        self.assertTrue((self.snapshot / "system-file").is_file())

    def test_legacy_read_only_root_becomes_deletable(self):
        with self.assertRaises(btrfsutil.BtrfsUtilError):
            btrfsutil.delete_subvolume(self.snapshot)
        self.assertEqual(pruner.prune(self.snapshot), 2)
        self.assertTrue(btrfsutil.get_subvolume_read_only(self.snapshot))
        self.assertTrue((self.snapshot / "system-file").is_file())
        self.assertEqual(pruner.prune(self.snapshot), 0)
        btrfsutil.delete_subvolume(self.snapshot)
        self.assertFalse(self.snapshot.exists())

    def test_data_in_any_child_preserves_every_child(self):
        data = self.children[1] / "container.img"
        data.write_text("important data")
        with self.assertRaisesRegex(RuntimeError, "contains data"):
            pruner.prune(self.snapshot)
        self.assertEqual(data.read_text(), "important data")
        self.assert_protected()

    def test_other_subvolumes_are_not_pruned(self):
        btrfsutil.set_subvolume_read_only(self.snapshot, False)
        btrfsutil.create_subvolume(self.snapshot / "other")
        btrfsutil.set_subvolume_read_only(self.snapshot, True)
        with self.assertRaisesRegex(RuntimeError, "other nested"):
            pruner.prune(self.snapshot)
        self.assert_protected()

    def test_writable_root_waits_for_reboot(self):
        btrfsutil.set_subvolume_read_only(self.snapshot, False)
        with self.assertRaisesRegex(RuntimeError, "writable"):
            pruner.prune(self.snapshot)
        self.assertFalse(btrfsutil.get_subvolume_read_only(self.snapshot))
        self.assertTrue(all(child.exists() for child in self.children))

    def test_seal_then_prune_after_restore(self):
        btrfsutil.set_subvolume_read_only(self.snapshot, False)
        self.assertTrue(pruner.seal(self.snapshot))
        self.assert_protected()
        self.assertFalse(pruner.seal(self.snapshot))
        self.assertEqual(pruner.prune(self.snapshot), 2)
        btrfsutil.delete_subvolume(self.snapshot)

    def test_seal_preserves_nested_data_and_child_flags(self):
        btrfsutil.set_subvolume_read_only(self.snapshot, False)
        data = self.children[0] / "container.img"
        data.write_text("keep this data")
        self.assertTrue(pruner.seal(self.snapshot))
        self.assert_protected()
        self.assertEqual(data.read_text(), "keep this data")
        with self.assertRaisesRegex(RuntimeError, "contains data"):
            pruner.prune(self.snapshot)
        self.assert_protected()

    def test_seal_defers_mounted_root_even_after_rename(self):
        btrfsutil.set_subvolume_read_only(self.snapshot, False)
        target = self.snapshot.parent / "mounted"
        target.mkdir()
        subprocess.run(["mount", "--bind", str(self.snapshot), str(target)], check=True)
        renamed = self.snapshot.parent / "retained"
        self.snapshot.rename(renamed)
        try:
            self.assertFalse(pruner.seal(renamed))
            self.assertFalse(btrfsutil.get_subvolume_read_only(renamed))
            (target / "still-running").write_text("root remains writable")
        finally:
            subprocess.run(["umount", str(target)], check=True)
        self.assertTrue(pruner.seal(renamed))
        self.assertEqual((renamed / "still-running").read_text(), "root remains writable")

    def test_seal_defers_mounted_child(self):
        btrfsutil.set_subvolume_read_only(self.snapshot, False)
        target = self.snapshot.parent / "mounted"
        target.mkdir()
        subprocess.run(["mount", "--bind", str(self.children[0]), str(target)], check=True)
        try:
            self.assertFalse(pruner.seal(self.snapshot))
            self.assertFalse(btrfsutil.get_subvolume_read_only(self.snapshot))
        finally:
            subprocess.run(["umount", str(target)], check=True)

    def test_seal_defers_default_root(self):
        btrfsutil.set_subvolume_read_only(self.snapshot, False)
        btrfsutil.set_default_subvolume(self.snapshot)
        try:
            self.assertFalse(pruner.seal(self.snapshot))
            self.assertFalse(btrfsutil.get_subvolume_read_only(self.snapshot))
        finally:
            btrfsutil.set_default_subvolume(self.mount, 5)

    def test_seal_ignores_user_snapshot(self):
        btrfsutil.set_subvolume_read_only(self.snapshot, False)
        (self.snapshot.parent / "info.xml").write_text('<snapshot><type>single</type><description>my backup</description></snapshot>')
        self.assertFalse(pruner.seal(self.snapshot))
        self.assertFalse(btrfsutil.get_subvolume_read_only(self.snapshot))

    def test_seal_rejects_symlink(self):
        btrfsutil.set_subvolume_read_only(self.snapshot, False)
        alias = self.snapshot.parent / "alias"
        alias.symlink_to(self.snapshot)
        with self.assertRaisesRegex(RuntimeError, "symlink"):
            pruner.seal(alias)
        self.assertFalse(btrfsutil.get_subvolume_read_only(self.snapshot))

    def test_prune_checks_writable_root_without_children(self):
        self.assertEqual(pruner.prune(self.snapshot), 2)
        btrfsutil.set_subvolume_read_only(self.snapshot, False)
        with self.assertRaisesRegex(RuntimeError, "writable"):
            pruner.prune(self.snapshot)

    def test_regular_user_snapshot_is_untouched(self):
        (self.snapshot.parent / "info.xml").write_text('<snapshot><type>single</type><description>my backup</description></snapshot>')
        self.assertEqual(pruner.prune(self.snapshot), 0)
        self.assert_protected()

    def test_mounted_snapshot_is_untouched(self):
        target = self.snapshot.parent / "mounted"
        target.mkdir()
        subprocess.run(["mount", "--bind", str(self.snapshot), str(target)], check=True)
        try:
            with self.assertRaisesRegex(RuntimeError, "mounted"):
                pruner.prune(self.snapshot)
            self.assert_protected()
        finally:
            subprocess.run(["umount", str(target)], check=True)

    def test_mounted_child_is_untouched(self):
        target = self.snapshot.parent / "mounted"
        target.mkdir()
        subprocess.run(["mount", "--bind", str(self.children[0]), str(target)], check=True)
        try:
            with self.assertRaisesRegex(RuntimeError, "mounted"):
                pruner.prune(self.snapshot)
            self.assert_protected()
        finally:
            subprocess.run(["umount", str(target)], check=True)

    def test_default_subvolume_is_untouched(self):
        btrfsutil.set_default_subvolume(self.snapshot)
        try:
            with self.assertRaisesRegex(RuntimeError, "default"):
                pruner.prune(self.snapshot)
            self.assert_protected()
        finally:
            btrfsutil.set_default_subvolume(self.mount, 5)

    def test_symlink_path_is_refused(self):
        alias = self.snapshot.parent / "alias"
        alias.symlink_to(self.snapshot)
        with self.assertRaisesRegex(RuntimeError, "symlink"):
            pruner.prune(alias)
        self.assert_protected()

    def test_failure_restores_read_only_parent_and_surviving_child_flags(self):
        delete = btrfsutil.delete_subvolume
        calls = 0

        def fail_second(path):
            nonlocal calls
            calls += 1
            if calls == 2:
                raise OSError("injected deletion failure")
            delete(path)

        with patch.object(btrfsutil, "delete_subvolume", fail_second):
            with self.assertRaisesRegex(OSError, "injected"):
                pruner.prune(self.snapshot)
        self.assertTrue(btrfsutil.get_subvolume_read_only(self.snapshot))
        remaining = [child for child in self.children if child.exists()]
        self.assertEqual(len(remaining), 1)
        self.assertFalse(btrfsutil.get_subvolume_read_only(remaining[0]))
        self.assertTrue((self.snapshot / "system-file").is_file())


if __name__ == "__main__":
    unittest.main(verbosity=2)
