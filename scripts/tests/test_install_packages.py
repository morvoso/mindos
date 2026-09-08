#!/usr/bin/env python3
"""Exercise clean-target resolution and NVIDIA fallback without touching pacman."""
import importlib.util
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('install_packages', ROOT / 'packages/mindos-install/install-packages.py')
packages = importlib.util.module_from_spec(spec)
spec.loader.exec_module(packages)


class ResolutionTests(unittest.TestCase):
    def resolver(self, results):
        calls, roots = [], []
        def run(command, **kwargs):
            root = Path(command[command.index('--dbpath') + 1])
            self.assertTrue(root.name.startswith('mindos-install-packages-'))
            self.assertEqual(list((root / 'local').iterdir()), [])
            self.assertEqual(command[command.index('--logfile') + 1], str(root / 'pacman.log'))
            self.assertEqual(command[command.index('--cachedir') + 1], str(root / 'cache'))
            self.assertNotIn('-Syu', command)
            calls.append(command)
            roots.append(root)
            code, error = results[len(calls) - 1]
            return subprocess.CompletedProcess(command, code, stdout='', stderr=error)
        return run, calls, roots

    def test_matching_prebuilt_and_cleanup(self):
        run, calls, roots = self.resolver([(0, ''), (0, '')])
        result = packages.resolve(['base', packages.PREBUILT, 'base'], run=run)
        self.assertEqual(result['packages'], ['base', packages.PREBUILT])
        self.assertEqual(result['nvidia_modules'], 'prebuilt')
        self.assertIsNone(result['fallback_reason'])
        self.assertFalse(roots[0].exists())
        self.assertIn('-Sp', calls[1])

    def test_mismatch_uses_dkms_with_headers_and_compiler(self):
        run, calls, roots = self.resolver([(0, ''), (1, 'nvidia-utils version mismatch'), (0, '')])
        result = packages.resolve(['base', packages.PREBUILT, 'nvidia-utils'], run=run)
        self.assertEqual(result['nvidia_modules'], 'dkms')
        self.assertNotIn(packages.PREBUILT, result['packages'])
        self.assertTrue(set(packages.DKMS).issubset(result['packages']))
        self.assertIn('version mismatch', result['fallback_reason'])
        self.assertFalse(roots[0].exists())

    def test_mesa_never_adds_nvidia_even_with_dkms_policy(self):
        run, calls, _ = self.resolver([(0, ''), (0, '')])
        result = packages.resolve(['base', 'vulkan-nouveau'], policy='dkms', run=run)
        self.assertEqual(result['packages'], ['base', 'vulkan-nouveau'])
        self.assertEqual(result['nvidia_modules'], 'none')
        self.assertEqual(len(calls), 2)

    def test_explicit_dkms_skips_prebuilt_attempt(self):
        run, calls, _ = self.resolver([(0, ''), (0, '')])
        result = packages.resolve([packages.PREBUILT], policy='dkms', run=run)
        self.assertEqual(result['nvidia_modules'], 'dkms')
        self.assertNotIn(packages.PREBUILT, calls[1])

    def test_explicit_prebuilt_failure_does_not_fall_back(self):
        run, calls, roots = self.resolver([(0, ''), (1, 'mismatch')])
        with self.assertRaisesRegex(ValueError, 'no disk changes'):
            packages.resolve([packages.PREBUILT], policy='prebuilt', run=run)
        self.assertEqual(len(calls), 2)
        self.assertFalse(roots[0].exists())

    def test_failed_fallback_aborts_and_cleans_up(self):
        run, calls, roots = self.resolver([(0, ''), (1, 'mismatch'), (1, 'missing base')])
        with self.assertRaisesRegex(ValueError, 'missing base'):
            packages.resolve([packages.PREBUILT], run=run)
        self.assertFalse(roots[0].exists())

    def test_refresh_failure_aborts_without_resolution(self):
        run, calls, roots = self.resolver([(1, 'offline')])
        with self.assertRaisesRegex(ValueError, 'offline'):
            packages.resolve(['base'], run=run)
        self.assertEqual(len(calls), 1)
        self.assertFalse(roots[0].exists())

    def test_invalid_input_does_not_invoke_pacman(self):
        run, calls, _ = self.resolver([])
        for names, policy in [([], 'auto'), (['--root'], 'auto'), (['base'], 'invalid')]:
            with self.assertRaises(ValueError):
                packages.resolve(names, policy=policy, run=run)
        self.assertEqual(calls, [])


if __name__ == '__main__':
    unittest.main()
