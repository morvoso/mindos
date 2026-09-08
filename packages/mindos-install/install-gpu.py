#!/usr/bin/python3
"""Read-only graphics plan for installation; never load drivers or change devices."""
import argparse
from html.parser import HTMLParser
import json
from pathlib import Path
import re
import sys

NVIDIA_PACKAGES = ['linux-mindos-nvidia-open', 'nvidia-utils', 'lib32-nvidia-utils',
                   'nvidia-settings']
NVIDIA_MODULES = ['nvidia', 'nvidia_modeset', 'nvidia_uvm', 'nvidia_drm']
MESA_MODULES = {'amdgpu', 'radeon', 'i915', 'xe', 'nouveau'}
VENDORS = {0x10de: 'NVIDIA', 0x1002: 'AMD', 0x8086: 'Intel', 0x1af4: 'Virtio'}


class SupportTable(HTMLParser):
    """Keep NVIDIA's current and legacy product tables separate."""
    def __init__(self):
        super().__init__()
        self.branch = None
        self.row = None
        self.cells = []
        self.cell = None
        self.products = {}

    def handle_starttag(self, tag, attributes):
        attrs = dict(attributes)
        anchor = attrs.get('id', attrs.get('name', ''))
        if tag == 'a' and anchor == 'Current':
            self.branch = 'current'
        elif tag == 'a' and anchor.startswith('legacy_'):
            self.branch = anchor.removeprefix('legacy_')
        elif tag == 'tr' and self.branch:
            self.row, self.cells = self.branch, []
        elif tag == 'td' and self.row:
            self.cell = []

    def handle_data(self, data):
        if self.cell is not None:
            self.cell.append(data)

    def handle_endtag(self, tag):
        if tag == 'td' and self.cell is not None:
            self.cells.append(' '.join(''.join(self.cell).split()))
            self.cell = None
        elif tag == 'tr' and self.row:
            if len(self.cells) >= 2:
                ids = self.cells[1].split()
                if len(ids) in (1, 3) and all(re.fullmatch(r'[0-9A-Fa-f]{4}', v) for v in ids):
                    key = tuple(int(v, 16) for v in ids)
                    self.products[key] = {'name': self.cells[0], 'branch': self.row}
            self.row = None

    def lookup(self, device):
        key = tuple(device[k] for k in ('device', 'subsystem_vendor', 'subsystem_device'))
        return self.products.get(key, self.products.get(key[:1]))


def read_devices(root=Path('/sys/bus/pci/devices')):
    devices = []
    for node in sorted(root.iterdir()):
        def number(name):
            return int((node / name).read_text().strip(), 16)
        try:
            # Numeric PCI class: VGA, 3D and other display controllers. NVIDIA
            # audio/USB devices must not trigger a graphics-driver install.
            if number('class') >> 16 != 0x03:
                continue
            devices.append(dict(address=node.name, vendor=number('vendor'),
                device=number('device'), subsystem_vendor=number('subsystem_vendor'),
                subsystem_device=number('subsystem_device'),
                bound_driver=(node / 'driver').resolve().name if (node / 'driver').exists() else ''))
        except FileNotFoundError:
            # PCI hot-unplug during discovery; repeat detection before formatting.
            continue
    return devices


def make_plan(devices, policy='auto', support_path=Path('/usr/share/doc/nvidia/html/supportedchips.html')):
    if policy not in ('auto', 'mesa'):
        raise ValueError('MINDOS_GPU_DRIVER must be auto or mesa')
    nvidia = [d for d in devices if d['vendor'] == 0x10de]
    table = None
    version = None
    if nvidia and policy == 'auto':
        try:
            source = support_path.read_text()
        except OSError as error:
            raise ValueError('NVIDIA support data is unavailable; use an up-to-date MindOS live image') from error
        version_match = re.search(r'the\s+(\d+\.\d+(?:\.\d+)?)\s+driver', source)
        if not version_match or int(version_match[1].split('.')[0]) < 590:
            raise ValueError('NVIDIA support data predates the open-driver-only product list; use a newer live image')
        version = version_match[1]
        table = SupportTable()
        table.feed(source)
        if not any(p['branch'] == 'current' for p in table.products.values()):
            raise ValueError('NVIDIA support data contains no current GPUs; use a newer live image')

    details, unsupported = [], []
    for device in devices:
        product = table.lookup(device) if table and device['vendor'] == 0x10de else None
        detail = dict(device, name=product['name'] if product else
                      f"{VENDORS.get(device['vendor'], 'PCI')} display ({device['vendor']:04x}:{device['device']:04x})",
                      branch=product['branch'] if product else None)
        details.append(detail)
        if table and device['vendor'] == 0x10de and (not product or product['branch'] != 'current'):
            unsupported.append(detail)
    if unsupported:
        names = '; '.join(f"{d['name']} ({d['address']}, " +
                         (f"legacy driver {d['branch']}" if d['branch'] else 'not in the support table') + ')'
                         for d in unsupported)
        raise ValueError(f'NVIDIA open driver {version} does not support: {names}. '
            'Automatic installation stopped before disk changes. Use a supported GPU or a tested legacy-driver image. '
            'For an explicit Mesa/Nouveau compatibility install, run '
            'sudo env MINDOS_GPU_DRIVER=mesa mindos-install; older NVIDIA gaming performance may be limited.')

    use_nvidia = bool(nvidia) and policy == 'auto'
    modules = list(NVIDIA_MODULES) if use_nvidia else []
    for device in devices:
        driver = device['bound_driver']
        if driver in MESA_MODULES and not (use_nvidia and device['vendor'] == 0x10de):
            if driver not in modules:
                modules.append(driver)
    if nvidia and not use_nvidia and 'nouveau' not in modules:
        modules.append('nouveau')
    packages = list(NVIDIA_PACKAGES) if use_nvidia else []
    if nvidia and not use_nvidia:
        packages += ['vulkan-nouveau', 'lib32-vulkan-nouveau']
    return dict(driver='nvidia-open' if use_nvidia else 'mesa', policy=policy,
                support_version=version, devices=details, modules=modules, packages=packages)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--driver', choices=('auto', 'mesa'), default='auto')
    parser.add_argument('--json', action='store_true')
    args = parser.parse_args()
    try:
        plan = make_plan(read_devices(), args.driver)
    except (OSError, ValueError) as error:
        sys.exit(f'mindos-install: {error}')
    if args.json:
        print(json.dumps(plan))
    else:
        print(f"Graphics: {plan['driver']}")
        for device in plan['devices']:
            print(f"  {device['address']}  {device['name']}")
        print('Packages: ' + (' '.join(plan['packages']) or 'Mesa stack from mindos-base'))
        if args.driver == 'mesa' and any(d['vendor'] == 0x10de for d in plan['devices']):
            print('Nouveau compatibility selected; this does not establish legacy NVIDIA gaming performance.')


if __name__ == '__main__':
    main()
