#!/usr/bin/env python3
"""Write a suspend-capable disposable QA XML; never applies it to libvirt.

Save the original inactive XML first. Shut down the QA VM before defining this
fixture, and restore the original after testing. Virtiofs prevents suspend.
"""
import argparse
from pathlib import Path
import xml.etree.ElementTree as ET

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('original', type=Path)
parser.add_argument('output', type=Path)
args = parser.parse_args()
assert args.original.resolve() != args.output.resolve(), 'Preserve the original XML'
tree = ET.parse(args.original)
root = tree.getroot()
assert root.findtext('name', '').startswith('mindos-qa-'), 'Disposable QA domains only'
devices = root.find('devices')
video = devices.findall('video')
assert len(video) == 1 and video[0].find('model').get('type') == 'virtio', 'Requires one virtio GPU'
for filesystem in list(devices.findall('filesystem')):
    devices.remove(filesystem)
ports = [int(c.get('index')) for c in devices.findall('controller') if c.get('model') == 'pcie-root-port']
occupied = {int(a.get('bus', '0'), 0) for a in devices.findall('./*/address') if a.get('type') == 'pci'}
port = next((p for p in ports if p not in occupied), None)
assert port is not None, 'Requires an unused PCIe root port (removing virtiofs usually frees one)'
address = video[0].find('address')
assert address is not None and address.get('type') == 'pci'
address.set('bus', hex(port)); address.set('slot', '0x00'); address.set('function', '0x0')
# QEMU exposes PCI PM only behind a root port. Merely setting the property on
# the root-bus VGA device does not create the capability the kernel checks.
ns = 'http://libvirt.org/schemas/domain/qemu/1.0'
ET.register_namespace('qemu', ns)
override = root.find(f'{{{ns}}}override')
if override is None: override = ET.SubElement(root, f'{{{ns}}}override')
alias = video[0].find('alias')
name = alias.get('name') if alias is not None else 'video0'
device = next((d for d in override if d.get('alias') == name), None)
if device is None: device = ET.SubElement(override, f'{{{ns}}}device', alias=name)
front = device.find(f'{{{ns}}}frontend')
if front is None: front = ET.SubElement(device, f'{{{ns}}}frontend')
for prop in list(front):
    if prop.get('name') == 'x-pcie-pm-no-soft-reset': front.remove(prop)
ET.SubElement(front, f'{{{ns}}}property', name='x-pcie-pm-no-soft-reset', type='bool', value='true')
pm = root.find('pm')
if pm is None: pm = ET.SubElement(root, 'pm')
mem = pm.find('suspend-to-mem')
if mem is None: mem = ET.SubElement(pm, 'suspend-to-mem')
mem.set('enabled', 'yes')
tree.write(args.output, encoding='unicode')
print(f'Wrote {args.output}: virtio GPU behind root port {port}, no-reset enabled, virtiofs removed, S3 exposed')
