#!/usr/bin/env python3
"""Verify every packaged module has a signature footer after compression.

Requires Python 3.14. This is a structural check; boot and module-loading
tests establish that the running kernel actually trusts those signatures.
"""

import argparse
from compression import zstd
import tarfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("package")
args = parser.parse_args()
count = 0
provides_ntsync = False
builtin_ntsync = False
with tarfile.open(args.package, "r|zst") as archive:
    for member in archive:
        if member.name.removeprefix('./') == '.PKGINFO':
            provides_ntsync = b'provides = NTSYNC-MODULE' in archive.extractfile(member).read().splitlines()
        if member.name.endswith('/modules.builtin'):
            builtin_ntsync = any(line.endswith(b'/ntsync.ko') for line in archive.extractfile(member).read().splitlines())
        if member.name.endswith("signing_key.pem"):
            raise SystemExit("private signing key must not be shipped")
        if member.isfile() and member.name.endswith(".ko.zst"):
            content = zstd.decompress(archive.extractfile(member).read())
            if not content.endswith(b"~Module signature appended~\n"):
                raise SystemExit(f"unsigned packaged module: {member.name}")
            count += 1
if not count:
    raise SystemExit("package contains no compressed modules")
if not provides_ntsync or not builtin_ntsync:
    raise SystemExit("kernel must advertise NTSYNC-MODULE and include the built-in ntsync driver")
print(f"PASS: {count} packaged modules retain their signature footer")
print("PASS: NTSYNC-MODULE provider matches the built-in driver")
