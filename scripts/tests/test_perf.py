#!/usr/bin/env python3
"""Exercise the real performance command against fake devices, never host knobs."""

import concurrent.futures
import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]


class PerformanceTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="mindos-perf-test-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.env = dict(os.environ, PATH=f"{self.root}/bin:{os.environ['PATH']}")
        self.env.pop("SUDO_USER", None)
        self.log = self.root / "calls"
        source = (ROOT / "packages/mindos-base/mindos-perf").read_text()
        for prefix in ("/etc/mindos", "/var/lib/mindos", "/run/mindos", "/sys/", "/proc/", "/dev/nvidiactl"):
            source = source.replace(prefix, str(self.root) + prefix)
        source = source.replace("w() {", "device_write() {", 1)
        marker = "# ----- commands"
        declarations, commands = source.split(marker, 1)
        # Model pstate rejecting a non-performance EPP while performance is set.
        overrides = f'''
need_root() {{ :; }}
w() {{
  printf '%s %s\\n' "$1" "$2" >> {shlex.quote(str(self.log))}
  if [[ "$1" = */energy_performance_preference && "$2" != performance ]] &&
     [[ "$(cat "${{1%/*}}/scaling_governor")" = performance ]]; then
    return 0
  fi
  device_write "$@"
}}
'''
        self.script = self.root / "mindos-perf"
        self.script.write_text(declarations + overrides + marker + commands)
        self.write("etc/mindos/perf.conf", 'SCX_SCHEDULER=""\n')
        self.write("var/lib/mindos/perf/mode", "balanced\n")
        self.write("dev/nvidiactl", "")
        self.policy = "sys/devices/system/cpu/cpufreq/policy0/"
        for name, value in {
            "scaling_available_governors": "performance powersave",
            "scaling_governor": "performance",
            "energy_performance_preference": "performance",
            "boost": "1",
        }.items():
            self.write(self.policy + name, value + "\n")
        for name in ("enabled", "defrag"):
            self.write("sys/kernel/mm/transparent_hugepage/" + name, "always\n")
        self.command("systemctl", "exit 1\n")
        self.command("logger", "exit 0\n")
        self.command("mind", f"printf '%s\\n' \"$*\" >> {shlex.quote(str(self.log))}\n")
        self.command("nvidia-smi", f'''
printf 'nvidia %s\\n' "$*" >> {shlex.quote(str(self.log))}
case "$*" in
  *uuid,power.max_limit*) printf 'GPU-A, 450.00\\nGPU-B, 200.00\\n';;
  *uuid,power.default_limit*) printf 'GPU-A, 350.00\\nGPU-B, 150.00\\n';;
  *name,persistence_mode,power.limit*) printf 'Test GPU, Disabled, 150.00\\n';;
esac
''')

    def write(self, path, value):
        target = self.root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(value)
        return target

    def command(self, name, body):
        self.write("bin/" + name, "#!/bin/bash\n" + body).chmod(0o755)

    def run_perf(self, *args, ok=True):
        result = subprocess.run(["bash", str(self.script), *args], env=self.env,
                                text=True, capture_output=True, timeout=15)
        if ok:
            self.assertEqual(result.returncode, 0, result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0)
        return result

    def read(self, path):
        return (self.root / path).read_text().strip()

    def test_balanced_restores_epp_after_leaving_performance_governor(self):
        self.run_perf("set", "balanced")
        self.assertEqual(self.read(self.policy + "scaling_governor"), "powersave")
        self.assertEqual(self.read(self.policy + "energy_performance_preference"), "balance_performance")

    def test_schedutil_is_used_when_supported(self):
        self.write(self.policy + "scaling_available_governors", "performance powersave schedutil\n")
        self.run_perf("set", "balanced")
        self.assertEqual(self.read(self.policy + "scaling_governor"), "schedutil")

    def test_concurrent_games_restore_only_after_last_exit(self):
        with concurrent.futures.ThreadPoolExecutor(max_workers=12) as pool:
            list(pool.map(lambda _: self.run_perf("game-start"), range(12)))
        self.assertEqual(self.read("run/mindos/perf/game"), "12")
        self.assertEqual(self.log.read_text().count("sleep on\n"), 1)
        self.run_perf("set", "quiet")
        self.assertEqual(self.read("run/mindos/perf/effective"), "performance")
        with concurrent.futures.ThreadPoolExecutor(max_workers=12) as pool:
            list(pool.map(lambda _: self.run_perf("game-end"), range(11)))
        self.assertEqual(self.read("run/mindos/perf/game"), "1")
        self.assertNotIn("sleep off\n", self.log.read_text())
        self.run_perf("game-end")
        self.assertEqual(self.read("run/mindos/perf/effective"), "quiet")
        self.assertEqual(self.log.read_text().count("sleep off\n"), 1)
        previous = self.log.read_text()
        self.run_perf("game-end")
        self.assertEqual(self.log.read_text(), previous)

    def test_apply_preserves_running_game_and_pending_preference(self):
        self.run_perf("game-start")
        self.run_perf("set", "quiet")
        self.run_perf("apply")
        self.assertEqual(self.read("run/mindos/perf/game"), "1")
        self.assertEqual(self.read("run/mindos/perf/effective"), "performance")
        self.run_perf("game-end")
        self.assertEqual(self.read("run/mindos/perf/effective"), "quiet")

    def test_sleep_preference_change_does_not_strand_the_mind_asleep(self):
        self.run_perf("game-start")
        self.run_perf("config", "MIND_SLEEPS_WHILE_GAMING", "0")
        self.run_perf("game-end")
        self.assertEqual(self.log.read_text().count("sleep off\n"), 1)
        self.assertFalse((self.root / "run/mindos/perf/mind-slept").exists())

    def test_enabling_sleep_mid_game_does_not_wake_a_manually_sleeping_mind(self):
        self.run_perf("config", "MIND_SLEEPS_WHILE_GAMING", "0")
        self.run_perf("game-start")
        self.run_perf("config", "MIND_SLEEPS_WHILE_GAMING", "1")
        self.run_perf("game-end")
        self.assertNotIn("sleep off\n", self.log.read_text())

    def test_end_reapplies_mode_after_gamemode_restores_stale_governor(self):
        self.run_perf("game-start")
        self.run_perf("set", "performance")
        self.write(self.policy + "scaling_governor", "powersave\n")
        self.run_perf("game-end")
        self.assertEqual(self.read(self.policy + "scaling_governor"), "performance")

    def test_each_gpu_gets_its_own_limit_and_quiet_disables_persistence(self):
        self.run_perf("config", "NVIDIA_POWER_LIMIT", "max")
        self.run_perf("set", "performance")
        calls = self.log.read_text()
        self.assertIn("nvidia -i GPU-A -pl 450.00\n", calls)
        self.assertIn("nvidia -i GPU-B -pl 200.00\n", calls)
        self.run_perf("set", "quiet")
        calls = self.log.read_text()
        self.assertIn("nvidia -i GPU-A -pl 350.00\n", calls)
        self.assertIn("nvidia -i GPU-B -pl 150.00\n", calls)
        self.assertIn("nvidia -pm 0\n", calls)

    def test_power_limit_edit_does_not_restart_cpu_or_scheduler(self):
        self.run_perf("set", "performance")
        self.log.write_text("")
        self.run_perf("config", "NVIDIA_POWER_LIMIT", "225.5")
        self.assertIn("nvidia -i GPU-A -pl 225.5\n", self.log.read_text())
        self.assertNotIn("scaling_governor", self.log.read_text())

    def test_configuration_rejects_partial_matches_and_preserves_literal_args(self):
        for key, value in (("SCX_SCHEDULER", "scx_ok;bad"), ("NVIDIA_POWER_LIMIT", "150bad")):
            self.run_perf("config", key, value, ok=False)
        value = "--name=a&b|c --mask=*"
        self.run_perf("config", "SCX_ARGS", value)
        result = subprocess.run(["bash", "-c", 'source "$1"; printf "%s" "$SCX_ARGS"',
                                 "test", str(self.root / "etc/mindos/perf.conf")],
                                text=True, capture_output=True, check=True)
        self.assertEqual(result.stdout, value)

    def test_performance_avoids_synchronous_thp_compaction(self):
        self.run_perf("set", "performance")
        self.assertEqual(self.read("sys/kernel/mm/transparent_hugepage/defrag"), "defer")

    @staticmethod
    def former_status(**raw):
        """The JSON and text mindos-perf produced with python before it printed them itself.

        mindshell parses the JSON and the Mind reads the text; both must stay byte-identical."""
        s = dict(raw)
        s['game'] = int(s['game'])
        s['mindSleeps'] = s['mindSleeps'] == '1'
        s['boost'] = None if s['boost'] == '' else s['boost'] == '1'
        for k in ('nvidia', 'persistence'):
            s[k] = s[k] == 'true'
        text = [f"mode:        {s['mode']}" + (f"  (game running: {s['effective']})" if s['game'] else ""),
                f"cpu:         {s['cpu']}",
                f"governor:    {s['governor']} / epp {s['epp'] or '-'} / boost {'on' if s['boost'] else 'off'} ({s['driver']})",
                f"scheduler:   {s['scheduler']}",
                f"huge pages:  {s['thp']}"]
        if s['platformProfile']: text.append(f"platform:    {s['platformProfile']}")
        if s['nvidia']: text.append(f"gpu:         {s['gpu']} (power limit {s['powerLimit']} W)")
        text.append(f"while gaming: {s['gameMode'] or 'unchanged'}" + (", Mind sleeps" if s['mindSleeps'] else ""))
        return json.dumps(s, separators=(',', ':')) + "\n", "\n".join(text) + "\n"

    def test_status_keeps_the_exact_former_json_and_text(self):
        cpu = 'Tëst "CPU" \\ \U0001F600 tab\t bell\x07 del\x7f end'
        self.write("proc/cpuinfo", f"processor : 0\nmodel name\t: {cpu}\n")
        self.write(self.policy + "scaling_driver", "amd-pstate-epp\n")
        self.write("sys/firmware/acpi/platform_profile", "balanced\n")
        self.write("sys/kernel/sched_ext/root/ops", "scx_lavd\n")
        self.run_perf("set", "performance")
        self.run_perf("game-start")
        self.write("sys/kernel/mm/transparent_hugepage/enabled", "always [madvise] never\n")
        base = dict(mode="performance", effective="performance", game="1", gameMode="performance", mindSleeps="1",
                    cpu=cpu, driver="amd-pstate-epp", governor="performance", epp="performance", boost="1",
                    platformProfile="balanced", thp="madvise", scheduler="scx_lavd", scx="",
                    nvidia="true", gpu="Test GPU", powerLimit="150.00", powerLimitPolicy="default", persistence="false")

        def check(**changes):
            expected_json, expected_text = self.former_status(**{**base, **changes})
            self.assertEqual(self.run_perf("status", "--json").stdout, expected_json)
            self.assertEqual(self.run_perf("status").stdout, expected_text)
        check()
        self.write("proc/cpuinfo", "model name\t: Brand: Model: X\n")
        check(cpu="X")  # everything up to the last ": " goes, as before
        (self.root / self.policy / "boost").unlink()
        self.write("sys/devices/system/cpu/intel_pstate/no_turbo", "1\n")
        check(cpu="X", boost="0")
        (self.root / "sys/devices/system/cpu/intel_pstate/no_turbo").unlink()
        check(cpu="X", boost="")
        self.write(self.policy + "energy_performance_preference", "")
        check(cpu="X", boost="", epp="")
        (self.root / "dev/nvidiactl").unlink()
        check(cpu="X", boost="", epp="", nvidia="false", gpu="", powerLimit="")
        self.write("dev/nvidiactl", "")
        self.write("var/lib/mindos/perf/mode", "  quiet \n")
        self.write("etc/mindos/perf.conf", 'SCX_SCHEDULER=""\nGAME_MODE=""\nMIND_SLEEPS_WHILE_GAMING=0\n')
        check(cpu="X", boost="", epp="", mode="quiet", gameMode="", mindSleeps="0")
        self.run_perf("game-end")  # re-applies quiet: governor, epp, huge pages and the GPU line change with it
        check(cpu="X", boost="", game="0", mode="quiet", effective="quiet", gameMode="", mindSleeps="0",
              governor="powersave", epp="power", thp="")

    def test_status_spawns_neither_python_nor_nvidia_smi(self):
        for name in ("python", "python3"):
            self.command(name, f"printf 'python %s\\n' \"$*\" >> {shlex.quote(str(self.log))}\n")
        self.write("proc/cpuinfo", "model name\t: CPU\n")
        self.run_perf("set", "balanced")  # remembers the GPU's name, persistence mode and power limit
        self.assertEqual(self.read("run/mindos/perf/nvidia"), "Test GPU, Disabled, 150.00")
        self.log.write_text("")
        status = json.loads(self.run_perf("status", "--json").stdout)
        self.run_perf("status")
        self.assertEqual(self.log.read_text(), "")
        self.assertEqual((status["gpu"], status["powerLimit"], status["nvidia"]), ("Test GPU", "150.00", True))
        (self.root / "run/mindos/perf/nvidia").unlink()  # nothing applied yet this boot
        self.run_perf("status", "--json")
        self.assertEqual(self.log.read_text(), "nvidia --query-gpu=name,persistence_mode,power.limit --format=csv,noheader,nounits\n")

    def test_status_separates_configuration_from_applied_state(self):
        self.write("proc/cpuinfo", 'model name : Test "CPU"\n')
        self.run_perf("config", "NVIDIA_POWER_LIMIT", "max")
        self.run_perf("game-start")
        self.run_perf("game-start")
        status = json.loads(self.run_perf("status", "--json").stdout)
        self.assertEqual(status["game"], 2)
        self.assertEqual(status["cpu"], 'Test "CPU"')
        self.assertEqual(status["powerLimitPolicy"], "max")
        self.assertEqual(status["powerLimit"], "150.00")
        self.assertTrue(status["nvidia"])
        self.assertFalse(status["persistence"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
