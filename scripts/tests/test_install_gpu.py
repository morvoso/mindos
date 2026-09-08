#!/usr/bin/env python3
"""Graphics selection uses disposable PCI trees and a small product-table fixture."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('install_gpu', ROOT / 'packages/mindos-install/install-gpu.py')
gpu = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gpu)
TABLE = '''<p>the 610.57.04 driver</p><a id="Current"></a><table><tbody>
<tr><td>Current GPU</td><td>2684</td><td>K</td></tr>
<tr><td>Board-specific GPU</td><td>1E78 10DE 13D8</td><td>J</td></tr>
<tr><td>Current mobile GPU</td><td>1F91</td><td>J</td></tr>
</tbody></table><a id="legacy_580.xx"></a><table>
<tr><td>Legacy GPU</td><td>1B80</td></tr>
</table><a id="legacy_470.xx"></a><table>
<tr><td>Older GPU</td><td>1180</td></tr></table>'''


def device(vendor=0x10de, dev=0x2684, driver='nvidia', **kwargs):
    return dict(address='0000:01:00.0', vendor=vendor, device=dev,
                subsystem_vendor=0x10de, subsystem_device=0, bound_driver=driver) | kwargs


class GraphicsTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.table = self.root / 'supportedchips.html'
        self.table.write_text(TABLE)

    def plan(self, devices, policy='auto'):
        return gpu.make_plan(devices, policy, self.table)

    def test_supported_nvidia_has_driver_cuda_and_32bit_stack(self):
        plan = self.plan([device()])
        self.assertEqual(plan['driver'], 'nvidia-open')
        self.assertIn('linux-mindos-nvidia-open', plan['packages'])
        self.assertIn('lib32-nvidia-utils', plan['packages'])
        self.assertNotIn('ggml-cuda', plan['packages'])
        self.assertNotIn('nouveau', plan['modules'])

    def test_legacy_and_unknown_ids_refuse_automatic_install(self):
        for chip, expected in [(0x1b80, '580.xx'), (0x1180, '470.xx'), (0xffff, 'not in the support table')]:
            with self.subTest(chip=chip), self.assertRaisesRegex(ValueError, expected):
                self.plan([device(dev=chip)])

    def test_subsystem_specific_entries_are_not_generalized(self):
        plan = self.plan([device(dev=0x1e78, subsystem_device=0x13d8)])
        self.assertEqual(plan['devices'][0]['name'], 'Board-specific GPU')
        with self.assertRaisesRegex(ValueError, 'not in the support table'):
            self.plan([device(dev=0x1e78, subsystem_device=0x0001)])

    def test_mixed_generations_cannot_silently_lose_a_display(self):
        with self.assertRaisesRegex(ValueError, 'Legacy GPU'):
            self.plan([device(), device(dev=0x1b80, address='0000:02:00.0')])

    def test_explicit_mesa_needs_no_vendor_table_and_keeps_32bit_vulkan(self):
        self.table.unlink()
        plan = self.plan([device(dev=0x1b80)], 'mesa')
        self.assertEqual(plan['driver'], 'mesa')
        self.assertEqual(plan['modules'], ['nouveau'])
        self.assertEqual(plan['packages'], ['vulkan-nouveau', 'lib32-vulkan-nouveau'])

    def test_amd_intel_and_virtual_outputs_need_no_nvidia_table(self):
        self.table.unlink()
        plan = self.plan([device(0x1002, 0x13c0, 'amdgpu'), device(0x8086, 0x1234, 'xe'),
                          device(0x1af4, 0x1050, 'virtio_gpu')])
        self.assertEqual(plan['driver'], 'mesa')
        self.assertEqual(plan['packages'], [])
        self.assertEqual(plan['modules'], ['amdgpu', 'xe'])

    def test_older_amd_keeps_its_actual_kernel_driver(self):
        self.assertEqual(self.plan([device(0x1002, 0x1234, 'radeon')])['modules'], ['radeon'])

    def test_hybrid_current_nvidia_retains_integrated_driver(self):
        plan = self.plan([device(), device(0x8086, 0x1234, 'i915')])
        self.assertEqual(plan['modules'][-1], 'i915')

    def test_no_display_is_valid_and_installs_no_extra_packages(self):
        self.assertEqual(self.plan([])['packages'], [])

    def test_missing_old_or_unparseable_metadata_fails_closed(self):
        for source in ['', TABLE.replace('610.57.04', '580.119.02'), TABLE.replace('id="Current"', 'id="Changed"')]:
            self.table.write_text(source)
            with self.assertRaises(ValueError):
                self.plan([device()])
        self.table.unlink()
        with self.assertRaisesRegex(ValueError, 'unavailable'):
            self.plan([device()])

    def test_unknown_policy_is_rejected(self):
        with self.assertRaises(ValueError):
            self.plan([], 'native')

    def test_numeric_pci_scan_filters_audio_usb_and_handles_3d_gpu(self):
        root = self.root / 'pci'
        root.mkdir()
        for index, cls in enumerate([0x030000, 0x030200, 0x040300, 0x0c0330]):
            node = root / f'0000:01:00.{index}'
            node.mkdir()
            for name, value in dict(vendor=0x10de, device=0x2684,
                                    subsystem_vendor=0x10de, subsystem_device=1, **{'class': cls}).items():
                (node / name).write_text(hex(value))
        self.assertEqual(len(gpu.read_devices(root)), 2)


if __name__ == '__main__':
    unittest.main()
