#!/usr/bin/env python3
"""Build the release repository from the newest package archives.

Run in the Arch build box: needs Python 3.14, repo-add and vercmp. Historical
archives stay in build/packages; only selected packages go into the ISO.
"""

import argparse
import ctypes
from dataclasses import dataclass
import fcntl
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile


@dataclass(frozen=True)
class Package:
    path: Path
    name: str
    version: str
    arch: str


def package_info(path):
    with tarfile.open(path, 'r|zst') as archive:
        for member in archive:
            if member.name.removeprefix('./') != '.PKGINFO':
                continue
            if not member.isfile() or member.size > 1024 * 1024:
                raise ValueError(f'{path.name}: invalid package metadata')
            fields = {}
            for line in archive.extractfile(member).read().decode().splitlines():
                key, sep, value = line.partition(' = ')
                if sep and key in ('pkgname', 'pkgver', 'arch'):
                    if key in fields or not value.strip():
                        raise ValueError(f'{path.name}: invalid {key}')
                    fields[key] = value
            if fields.keys() != {'pkgname', 'pkgver', 'arch'}:
                raise ValueError(f'{path.name}: missing package metadata')
            return Package(path, fields['pkgname'], fields['pkgver'], fields['arch'])
    raise ValueError(f'{path.name}: missing .PKGINFO')


def select_packages(source, arch):
    selected = {}
    archives = sorted(source.glob('*.pkg.tar.zst'))
    for path in archives:
        package = package_info(path)
        if package.arch not in ('any', arch):
            continue
        old = selected.get(package.name)
        if old:
            comparison = int(subprocess.check_output(
                ['vercmp', package.version, old.version], text=True).strip())
            if comparison == 0:
                raise ValueError(f'{package.name} {package.version}: duplicate archives {old.path.name}, {path.name}')
            if comparison < 0:
                continue
        selected[package.name] = package
    if not selected:
        raise ValueError(f'no packages for {arch} in {source}')
    return [selected[name] for name in sorted(selected)], archives


def publish(staged, destination):
    if not destination.exists():
        staged.rename(destination)
        return
    # Both directories share a parent/filesystem. Linux's exchange is atomic:
    # readers see the old complete repo or the new one, with no missing path.
    libc = ctypes.CDLL(None, use_errno=True)
    exchange = libc.renameat2
    exchange.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
    exchange.restype = ctypes.c_int
    if exchange(-100, os.fsencode(staged), -100, os.fsencode(destination), 2):
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error), str(destination))


def build(source, destination, arch='x86_64'):
    source = source.resolve(strict=True)
    destination = destination.absolute()
    if destination.is_symlink():
        raise ValueError('repository destination must not be a symlink')
    if destination.exists() and not destination.is_dir():
        raise ValueError('repository destination must be a directory')
    destination = destination.resolve()
    if source.is_relative_to(destination) or destination.is_relative_to(source):
        raise ValueError('package archive and repository directories must not overlap')
    destination.parent.mkdir(parents=True, exist_ok=True)
    # Concurrent builders must not replace each other's publication. This lock
    # lives outside the directory being exchanged and is never copied to media.
    with (destination.parent / f'.{destination.name}.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        selected, archives = select_packages(source, arch)
        with tempfile.TemporaryDirectory(prefix=f'.{destination.name}-', dir=destination.parent) as temp:
            stage = Path(temp) / 'repo'
            stage.mkdir()
            for package in selected:
                shutil.copy2(package.path, stage / package.path.name)
                copied = package_info(stage / package.path.name)
                if (copied.name, copied.version, copied.arch) != (package.name, package.version, package.arch):
                    raise ValueError(f'{package.path.name}: package changed during staging')
                signature = Path(str(package.path) + '.sig')
                if signature.is_file():
                    shutil.copy2(signature, stage / signature.name)
            subprocess.run(['repo-add', '--quiet', '--nocolor', str(stage / 'mindos.db.tar.zst'),
                            *(str(stage / package.path.name) for package in selected)], check=True)
            # repo-add's backup databases describe intermediate states and are
            # build debris, not files clients should receive.
            for backup in stage.glob('*.old'):
                backup.unlink()
            publish(stage, destination)
        before = sum(path.stat().st_size for path in archives)
        after = sum(package.path.stat().st_size for package in selected)
        print(f'Repository: {len(selected)} selected packages, {after:,} archive bytes; '
              f'{before - after:,} obsolete/other-architecture bytes excluded', flush=True)
        return selected


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, default=Path('build/packages'))
    parser.add_argument('--output', type=Path, default=Path('build/repo'))
    parser.add_argument('--arch', default='x86_64')
    args = parser.parse_args()
    try:
        build(args.source, args.output, args.arch)
    except (OSError, ValueError, tarfile.TarError, subprocess.CalledProcessError) as error:
        parser.exit(1, f'build-repo: {error}\n')


if __name__ == '__main__':
    main()
