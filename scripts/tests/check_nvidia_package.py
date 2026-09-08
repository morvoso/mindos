#!/usr/bin/env python3
"""Check prebuilt NVIDIA ABI, version, and signatures against a released kernel.

Requires Python 3.14, kmod and OpenSSL. The certificate is public build output;
the private signing key is never read. Hardware initialization needs a GPU test.
"""
import argparse
from compression import zstd
from pathlib import Path
import struct
import re
import subprocess
import tarfile
import tempfile


def command(*args):
    return subprocess.check_output(args, text=True, stderr=subprocess.PIPE).strip()


def signed_payload(data):
    magic = b'~Module signature appended~\n'
    assert data.endswith(magic), 'Missing module signature'
    trailer = len(data) - len(magic) - 12
    length = struct.unpack('>I', data[trailer + 8:trailer + 12])[0]
    assert 0 < length < trailer, 'Invalid signature length'
    return data[:trailer - length], data[trailer - length:trailer]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('package')
    parser.add_argument('kernel')
    parser.add_argument('certificate')
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix='mindos-module-check-') as scratch:
        root = Path(scratch)
        cert = root / 'cert.pem'
        command('openssl', 'x509', '-inform', 'DER', '-in', args.certificate, '-out', str(cert))

        def verify(name, data):
            module = root / name
            module.write_bytes(data)
            payload, signature = signed_payload(data)
            (root / 'payload').write_bytes(payload)
            (root / 'signature').write_bytes(signature)
            # Explicit certificate only: no embedded certificate can substitute
            # a different signer. Verify the kernel's module the same way.
            command('openssl', 'cms', '-verify', '-binary', '-inform', 'DER',
                    '-in', str(root / 'signature'), '-content', str(root / 'payload'),
                    '-certfile', str(cert), '-nointern', '-noverify', '-out', '/dev/null')
            assert data[:4] == b'\x7fELF', 'Not an ELF module'
            supported = {'R_X86_64_NONE', 'R_X86_64_64', 'R_X86_64_32',
                         'R_X86_64_32S', 'R_X86_64_PC32', 'R_X86_64_PLT32', 'R_X86_64_PC64'}
            relocations = set(re.findall(r'R_X86_64_\w+', command('readelf', '-rW', str(module))))
            assert relocations <= supported, f'{name}: unsupported kernel relocations {relocations - supported}'
            return {field: command('modinfo', '-F', field, str(module))
                    for field in ('vermagic', 'version', 'sig_key', 'sig_hashalgo')}

        reference = None
        kernel_info = None
        with tarfile.open(args.kernel, 'r|zst') as archive:
            for member in archive:
                if member.name.removeprefix('./') == '.PKGINFO':
                    kernel_info = archive.extractfile(member).read().decode()
                if member.name.endswith('/virtio-gpu.ko.zst'):
                    reference = verify('virtio-gpu.ko', zstd.decompress(archive.extractfile(member).read()))
                    break
        assert reference and kernel_info, 'Released kernel lacks reference module/metadata'
        kernel_version = next(line.split(' = ', 1)[1] for line in kernel_info.splitlines() if line.startswith('pkgver = '))
        expected = {'nvidia', 'nvidia-modeset', 'nvidia-uvm', 'nvidia-drm', 'nvidia-peermem'}
        found, info, versions = set(), None, set()
        with tarfile.open(args.package, 'r|zst') as archive:
            for member in archive:
                name = member.name.removeprefix('./')
                if name == '.PKGINFO':
                    info = archive.extractfile(member).read().decode()
                if not member.isfile() or name.startswith('.'):
                    continue
                if name.endswith('.ko.zst'):
                    module_name = Path(name).name.removesuffix('.ko.zst')
                    assert module_name in expected and module_name not in found, name
                    result = verify(module_name + '.ko', zstd.decompress(archive.extractfile(member).read()))
                    assert result['vermagic'] == reference['vermagic'], name
                    assert result['sig_key'] == reference['sig_key'], name
                    assert result['sig_hashalgo'] == 'sha512', name
                    assert name.startswith('usr/lib/modules/' + result['vermagic'].split()[0] + '/extramodules/'), name
                    versions.add(result['version'])
                    found.add(module_name)
                else:
                    assert name == 'usr/share/licenses/linux-mindos-nvidia-open/COPYING', f'Unexpected payload: {name}'
        assert found == expected and len(versions) == 1 and info
        version = versions.pop()
        assert f'depend = linux-mindos={kernel_version}' in info.splitlines()
        assert f'depend = nvidia-utils={version}' in info.splitlines()
        assert 'provides = NVIDIA-MODULE' in info.splitlines()
        print(f'PASS: all five NVIDIA {version} modules match the released kernel ABI and signing certificate')
        print('PASS: SHA-512 CMS signatures verified cryptographically, including the released virtio reference')
        print('PASS: exact kernel/userspace dependencies; only modules and license shipped')
        print('PASS: ELF relocations are supported by the x86-64 kernel module loader')


if __name__ == '__main__':
    main()
