#!/usr/bin/env python3
"""Check the resolved kernel configuration, after olddefconfig, not the recipe.

Usage: check_kernel_config.py path/to/.config [--native]
"""

import argparse
from pathlib import Path
import re
import sys

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("config", type=Path)
parser.add_argument("--native", action="store_true")
args = parser.parse_args()
values = dict(re.findall(r"^CONFIG_([A-Z0-9_a-z]+)=(.*)$", args.config.read_text(), re.M))
required = {
    "X86_64": "y", "SCHED_BORE": "y", "SCHED_CLASS_EXT": "y", "HZ": "1000",
    "PREEMPT_DYNAMIC": "y", "NO_HZ_IDLE": "y", "NTSYNC": "y", "FUTEX": "y",
    "LTO_CLANG_THIN": "y", "MODULE_SIG": "y", "MODULE_SIG_ALL": "y",
    "X86_AMD_PSTATE": "y", "X86_INTEL_PSTATE": "y", "LRU_GEN_ENABLED": "y",
    "INPUT_TOUCHSCREEN": "y", "HID_MULTITOUCH": "m", "HID_SENSOR_HUB": "m",
    "IIO": "m", "REGULATOR": "y", "EXTCON": "y", "ACCESSIBILITY": "y",
    "SND_SOC": "m", "SOUNDWIRE": "m", "SND_SOC_SOF_PCI": "m",
    "SND_SOC_SOF_AMD_VANGOGH": "m", "SND_SOC_AMD_ACP": "m",
    "MEDIA_PCI_SUPPORT": "y", "DRM_AMDGPU": "m", "DRM_I915": "m", "DRM_XE": "m",
    "HID_STEAM": "m", "HID_NINTENDO": "m", "HID_PLAYSTATION": "m",
    "JOYSTICK_XPAD": "m", "SND_USB_AUDIO": "m", "BT_HCIBTUSB": "m",
    "DRM_VIRTIO_GPU": "m", "VIRTIO_FS": "m", "BTRFS_FS": "m",
}
required["X86_NATIVE_CPU"] = "y" if args.native else "n"
required["MODULE_SIG_FORCE"] = "n"
errors = [f"CONFIG_{key}: expected {want}, got {values.get(key, 'n')}"
          for key, want in required.items() if values.get(key, "n") != want]
if errors:
    sys.exit("\n".join(errors))
print(f"PASS: {len(required)} resolved kernel settings (gaming, device coverage, signatures, CPU target)")
