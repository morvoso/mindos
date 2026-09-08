#!/usr/bin/python3
"""Resolve a clean target's dependencies without changing the live package DB."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import tempfile

PREBUILT = 'linux-mindos-nvidia-open'
DKMS = ['nvidia-open-dkms', 'linux-mindos-headers', 'dkms', 'clang', 'llvm', 'lld']


def resolve(packages, config='/etc/mindos/pacman-install.conf', policy='auto', run=subprocess.run):
    if policy not in ('auto', 'prebuilt', 'dkms'):
        raise ValueError('MINDOS_NVIDIA_MODULES must be auto, prebuilt or dkms')
    packages = list(dict.fromkeys(packages))
    if not packages or any(p.startswith('-') for p in packages):
        raise ValueError('Expected package names')
    nvidia = PREBUILT in packages

    def dkms_packages():
        return list(dict.fromkeys([p for p in packages if p != PREBUILT] + DKMS))

    with tempfile.TemporaryDirectory(prefix='mindos-install-packages-') as scratch:
        root = Path(scratch)
        # Empty local DB: packages installed on the live ISO must not hide a
        # missing dependency on the new system. Never refresh the host's DB.
        for name in ('local', 'sync', 'cache'):
            (root / name).mkdir()
        command = ['pacman', '--config', config, '--dbpath', scratch,
                   '--logfile', str(root / 'pacman.log'), '--cachedir', str(root / 'cache'),
                   '--noconfirm']

        def invoke(arguments):
            try:
                result = run(command + arguments, capture_output=True, text=True, timeout=300)
            except (OSError, subprocess.TimeoutExpired) as error:
                raise ValueError(f'Package repository check failed: {error}') from error
            return result.returncode, result.stderr.strip()

        code, error = invoke(['-Sy'])
        if code:
            raise ValueError(f'Cannot refresh package repositories before installation: {error}')
        selected = dkms_packages() if nvidia and policy == 'dkms' else packages
        provider = ('dkms' if policy == 'dkms' else 'prebuilt') if nvidia else 'none'
        code, error = invoke(['-Sp', '--print-format', '%n %v', '--', *selected])
        fallback = None
        if code and nvidia and policy == 'auto':
            fallback = error or 'Prebuilt module dependencies could not be resolved'
            selected, provider = dkms_packages(), 'dkms'
            code, error = invoke(['-Sp', '--print-format', '%n %v', '--', *selected])
        if code:
            raise ValueError(f'Cannot resolve installation packages; no disk changes were made. {error}')
        return dict(packages=selected, nvidia_modules=provider, fallback_reason=fallback)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', default='/etc/mindos/pacman-install.conf')
    parser.add_argument('--nvidia-modules', choices=('auto', 'prebuilt', 'dkms'), default='auto')
    parser.add_argument('packages', nargs='+')
    args = parser.parse_args()
    try:
        print(json.dumps(resolve(args.packages, args.config, args.nvidia_modules)))
    except ValueError as error:
        sys.exit(f'mindos-install: {error}')


if __name__ == '__main__':
    main()
