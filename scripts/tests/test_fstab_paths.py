#!/usr/bin/env python3
"""Regression checks for mounts copied into a restored Btrfs root."""
import importlib.machinery
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
source = Path(__file__).resolve().parents[2] / "packages/mindos-base/fstab-paths"
spec = importlib.util.spec_from_loader("fstab_paths", importlib.machinery.SourceFileLoader("fstab_paths", str(source)))
fstab = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fstab)


class FstabPathsTests(unittest.TestCase):
    def test_rollback_replaces_id_but_preserves_path_and_options(self):
        text = "# root\nUUID=abc\t/ btrfs rw,subvolid=256,compress=zstd:1,subvol=/@ 0 0 # keep\n"
        expected = text.replace("subvolid=256,", "")
        self.assertEqual(fstab.normalize(text), expected)
        self.assertEqual(fstab.normalize(expected), expected)

    def test_id_only_custom_mount_and_other_filesystems_are_untouched(self):
        for line in ("UUID=a /data btrfs subvolid=42 0 0\n",
                     "UUID=a /data btrfs subvolid=42,subvol= 0 0\n",
                     "# UUID=a / btrfs subvol=/@,subvolid=42 0 0\n",
                     "UUID=a / ext4 defaults 0 1\n"):
            self.assertEqual(fstab.normalize(line), line)

    def test_whitespace_and_escaped_mount_paths_are_preserved(self):
        text = r"UUID=a /games\040library btrfs subvol=/@games,subvolid=82 0 0"
        self.assertEqual(fstab.normalize(text), text.replace(",subvolid=82", ""))

    def test_atomic_update_preserves_permissions_and_rejects_symlink(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "fstab"
            path.write_text("UUID=a / btrfs subvolid=9,subvol=/@ 0 0\n")
            path.chmod(0o640)
            fstab.update(path)
            self.assertEqual(path.read_text(), "UUID=a / btrfs subvol=/@ 0 0\n")
            self.assertEqual(path.stat().st_mode & 0o777, 0o640)
            link = Path(temp) / "link"
            link.symlink_to(path)
            with self.assertRaises(ValueError):
                fstab.update(link)


if __name__ == "__main__":
    unittest.main()
