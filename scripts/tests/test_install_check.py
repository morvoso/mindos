#!/usr/bin/env python3
"""Installer validation tests; all disk/mount inputs are data, never devices."""

import importlib.util
from pathlib import Path
import tempfile
import sys
import unittest

sys.dont_write_bytecode = True
SOURCE = Path(__file__).resolve().parents[2] / "packages/mindos-install/install-check.py"
spec = importlib.util.spec_from_file_location("install_check", SOURCE)
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)


class InstallCheckTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="mindos-install-test-")
        self.addCleanup(self.tmp.cleanup)
        self.zones = Path(self.tmp.name) / "zones"
        (self.zones / "Etc").mkdir(parents=True)
        (self.zones / "Etc/UTC").write_bytes(b"timezone fixture")
        (self.zones / "UTC").symlink_to("Etc/UTC")

    def identity(self, host="mindos-dev", user="player_test", tz="UTC", switches=("1", "1", "0")):
        return check.identity(host, user, tz, *switches, zoneinfo=self.zones)

    def test_valid_identity_and_timezone_alias(self):
        self.identity()
        self.identity(host="gaming.example.org", tz="Etc/UTC", switches=("0", "0", "1"))

    def test_hostnames_cannot_inject_shell_configuration(self):
        for host in ('bad"; touch /tmp/x', "$(id)", "-bad", "bad-", "a..b", "a" * 64, "a\nb"):
            with self.subTest(host=host), self.assertRaises(ValueError):
                self.identity(host=host)

    def test_invalid_or_reserved_username(self):
        for user in ("root", "nobody", "-user", "bad;id", "two words", "x" * 33, "User"):
            with self.subTest(user=user), self.assertRaises(ValueError):
                self.identity(user=user)

    def test_timezone_must_stay_inside_zoneinfo(self):
        outside = Path(self.tmp.name) / "outside"
        outside.write_bytes(b"not a timezone")
        (self.zones / "escape").symlink_to(outside)
        for tz in ("../outside", str(outside), "UTC;id", "$(id)", "escape", "Not/A_Zone"):
            with self.subTest(tz=tz), self.assertRaises(ValueError):
                self.identity(tz=tz)

    def test_boolean_options_are_strict(self):
        for value in ("yes", "2", "", "1;id"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                self.identity(switches=(value, "0", "1"))

    def disk(self, **changes):
        return {"name": "/dev/vda", "type": "disk", "ro": False, "mountpoints": [None], **changes}

    def test_empty_disk_and_unused_partitions_pass(self):
        check.check_device_tree([self.disk(children=[self.disk(name="/dev/vda1", type="part")])])

    def test_partition_or_mapper_cannot_be_whole_disk_target(self):
        for kind in ("part", "crypt", "lvm", "loop", "rom"):
            with self.subTest(kind=kind), self.assertRaises(ValueError):
                check.check_device_tree([self.disk(type=kind)])

    def test_read_only_disk_is_refused(self):
        with self.assertRaisesRegex(ValueError, "read-only"):
            check.check_device_tree([self.disk(ro=True)])

    def test_live_root_and_swap_are_refused(self):
        for mount in ("/run/archiso/bootmnt", "/", "[SWAP]", "/media/game disk"):
            with self.subTest(mount=mount), self.assertRaisesRegex(ValueError, "in use"):
                check.check_device_tree([self.disk(children=[self.disk(type="part", mountpoints=[mount])])])

    def test_encrypted_descendant_mount_is_refused(self):
        crypt = self.disk(type="crypt", mountpoints=["/home"])
        part = self.disk(type="part", children=[crypt])
        with self.assertRaisesRegex(ValueError, "in use"):
            check.check_device_tree([self.disk(children=[part])])

    def test_mount_target_must_be_unused(self):
        for mount in ("/mnt", "/mnt/boot", "/mnt/other device"):
            with self.subTest(mount=mount), self.assertRaisesRegex(ValueError, "mounted"):
                check.check_mount_target([{"target": mount}])
        check.check_mount_target([{"target": "/"}, {"target": "/mnt-other"}])


if __name__ == "__main__":
    unittest.main(verbosity=2)
