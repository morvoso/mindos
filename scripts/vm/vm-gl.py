#!/usr/bin/env python3
"""Turn virgl 3D acceleration on or off in a libvirt domain XML file.

Called by mindos-vm.sh: WANT=on|off NODE=<render node> vm-gl.py domain.xml
"""

import os
import sys
import xml.etree.ElementTree as ET

want = os.environ.get("WANT") == "on"
node = os.environ.get("NODE", "/dev/dri/renderD129")
path = sys.argv[1]

tree = ET.parse(path)
devices = tree.getroot().find("devices")
model = devices.find("./video/model")

# an explicit device= pins virtio-vga and libvirt then ignores accel3d
model.attrib.pop("device", None)
accel = model.find("acceleration")
if want:
    if accel is None:
        accel = ET.Element("acceleration")
        model.insert(0, accel)
    accel.set("accel3d", "yes")
elif accel is not None:
    model.remove(accel)

for old in devices.findall('./graphics[@type="egl-headless"]'):
    devices.remove(old)
if want:
    spice = devices.find('./graphics[@type="spice"]')
    headless = ET.Element("graphics", {"type": "egl-headless"})
    ET.SubElement(headless, "gl", {"rendernode": node})
    devices.insert(list(devices).index(spice) + 1, headless)

tree.write(path)
