#!/usr/bin/python3
"""Read-only installer preflight, shared by interactive and unattended installs."""

import json
import grp
from pathlib import Path
import pwd
import re
import stat
import subprocess
import sys


def identity(hostname, username, timezone, *switches, zoneinfo=Path("/usr/share/zoneinfo")):
    if not re.fullmatch(r"[a-z_][a-z0-9_-]{0,31}", username):
        raise ValueError("invalid username (use lowercase letters, digits, underscores and hyphens)")
    for lookup, key in ((pwd.getpwnam, "pw_uid"), (grp.getgrnam, "gr_gid")):
        try:
            identifier = getattr(lookup(username), key)
        except KeyError:
            continue
        if identifier < 1000 or identifier == 65534:
            raise ValueError("username is reserved for a system account or group")
    label = r"[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?"
    if len(hostname) > 253 or not re.fullmatch(rf"{label}(?:\.{label})*", hostname):
        raise ValueError("invalid hostname")
    if (not re.fullmatch(r"[A-Za-z0-9_+/-]+", timezone) or timezone.startswith("/") or
            not (zoneinfo / timezone).is_file() or
            not (zoneinfo / timezone).resolve().is_relative_to(zoneinfo.resolve())):
        raise ValueError(f"unknown timezone {timezone}")
    if any(value not in ("0", "1") for value in switches):
        raise ValueError("gaming, developer and autologin options must be 0 or 1")


def read_json(*argv):
    return json.loads(subprocess.check_output(argv, text=True))


def check_device_tree(devices):
    if len(devices) != 1 or devices[0].get("type") != "disk":
        raise ValueError("install target must be a whole disk, not a partition or mapped device")

    def check(node):
        if node.get("ro"):
            raise ValueError("install target is read-only")
        # lsblk reports [SWAP] here as well as mounts; include every descendant
        # so a live medium, running root, encrypted volume or active swap fails.
        if any(node.get("mountpoints") or []):
            raise ValueError(f"{node['name']} is in use; unmount it or disable its swap before installing")
        for child in node.get("children", []):
            check(child)

    check(devices[0])


def check_mount_target(mounts, target="/mnt"):
    for mount in mounts:
        path = mount.get("target", "")
        if path == target or path.startswith(target + "/"):
            raise ValueError(f"{target} contains mounted filesystems; unmount them before installing")


def preflight(disk, hostname, username, timezone, gaming, developer, autologin):
    identity(hostname, username, timezone, gaming, developer, autologin)
    path = Path(disk).resolve(strict=True)
    if not re.fullmatch(r"/dev/[A-Za-z0-9_./+-]+", str(path)):
        raise ValueError("install target must have a normal /dev device path")
    if not stat.S_ISBLK(path.stat().st_mode):
        raise ValueError("install target is not a block device")
    tree = read_json("lsblk", "--json", "--paths", "--output", "NAME,TYPE,RO,MOUNTPOINTS", str(path))
    check_device_tree(tree["blockdevices"])
    mounts = read_json("findmnt", "--json", "--list", "--output", "TARGET")
    check_mount_target(mounts["filesystems"])


if __name__ == "__main__":
    try:
        if len(sys.argv) != 8:
            raise ValueError("expected disk, hostname, username, timezone, gaming, developer, autologin")
        preflight(*sys.argv[1:])
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        sys.exit(f"mindos-install: {error}")
