#!/usr/bin/env python3
"""Exercise the real session wrapper with isolated command stubs, never host systemd."""
import os
from pathlib import Path
import subprocess
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]
SESSION = ROOT / 'packages/mindos-session/mindos-session'

class Lifecycle(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='mindos-session-test-')
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.trace = self.root / 'trace'
        self.pid = self.root / 'pid'
        self.env = dict(os.environ, PATH=f'{self.root}:{os.environ["PATH"]}',
                        XDG_CONFIG_HOME=str(self.root / 'config'),
                        SESSION_TEST_TRACE=str(self.trace), SESSION_TEST_PID=str(self.pid))
        self.stub('gsettings', 'exit 0')
        self.stub('systemctl', 'printf "systemctl %s\\n" "$*" >> "$SESSION_TEST_TRACE"')
        self.stub('systemd-cat', 'shift 2\nexec "$@"')

    def stub(self, name, body):
        path = self.root / name
        path.write_text('#!/bin/bash\n' + body + '\n')
        path.chmod(0o755)

    def test_compositor_exit_preserves_status_and_stops_session(self):
        self.stub('mindwm', 'printf "compositor\\n" >> "$SESSION_TEST_TRACE"\nexit 7')
        result = subprocess.run(['bash', str(SESSION)], env=self.env, timeout=5)
        self.assertEqual(result.returncode, 7)
        self.assertEqual(self.trace.read_text().splitlines(), [
            'systemctl --user stop mindos-session.target', 'compositor',
            'systemctl --user stop mindos-session.target'])

    def test_termination_reaches_compositor_and_cleans_up(self):
        self.stub('mindwm', '''echo $$ > "$SESSION_TEST_PID"
trap 'echo terminated >> "$SESSION_TEST_TRACE"; exit 0' TERM
while true; do sleep 0.05; done''')
        process = subprocess.Popen(['bash', str(SESSION)], env=self.env)
        self.addCleanup(lambda: process.poll() is None and process.kill())
        deadline = time.monotonic() + 3
        while not self.pid.exists() and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertTrue(self.pid.exists(), 'compositor did not start')
        process.terminate()
        self.assertEqual(process.wait(timeout=5), 143)
        lines = self.trace.read_text().splitlines()
        self.assertEqual(lines.count('systemctl --user stop mindos-session.target'), 2)
        self.assertIn('terminated', lines)
        with self.assertRaises(ProcessLookupError):
            os.kill(int(self.pid.read_text()), 0)

if __name__ == '__main__':
    unittest.main()
