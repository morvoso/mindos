#!/usr/bin/env python3
"""Native suspend, visible lock, password recovery and application survival QA.

Requires the disposable fixture from prepare_suspend_vm.py, guest agent,
qatest (password mindos), and an unlocked desktop. Never reboots the guest.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import shlex
import subprocess
import time
from PIL import Image

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--dom', required=True)
parser.add_argument('--mode', choices=('s2idle', 'deep'), required=True)
args = parser.parse_args()
assert args.dom.startswith('mindos-qa-'), 'Disposable QA domains only'
root = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('vdrive', root / 'scripts/vm/vdrive.py')
v = importlib.util.module_from_spec(spec); spec.loader.exec_module(v); v.DOM = args.dom
prefix = f'qa-suspend-{args.mode}'
unit = f'mindos-{prefix}-terminal'

def guest(command):
    rc, out, err = v.guest_exec(command)
    assert rc == 0, (command, out, err)
    return out.strip()

def qmp(command):
    result = subprocess.run(['virsh', '-c', 'qemu:///system', 'qemu-monitor-command', args.dom, json.dumps({'execute': command})], text=True, capture_output=True, check=True)
    reply = json.loads(result.stdout)
    assert 'error' not in reply, reply
    return reply['return']

def agent_ready():
    return subprocess.run(['virsh', '-c', 'qemu:///system', 'qemu-agent-command', args.dom, '--timeout', '2', '{"execute":"guest-ping"}'], capture_output=True).returncode == 0

def request(kind):
    payload = json.dumps({'type': kind}) + '\n'
    code = ('import socket; s=socket.socket(socket.AF_UNIX); s.settimeout(5); '
            "s.connect('/run/user/1000/mindwm-wayland-1.sock'); "
            f's.sendall({payload.encode()!r}); print(s.makefile().readline())')
    result = json.loads(guest('python3 -c ' + shlex.quote(code)))
    assert result['ok'], result
    return result.get('result')

def wait(predicate, message, seconds=15):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = predicate()
        if value: return value
        time.sleep(.2)
    raise AssertionError(message)

def shot(label):
    path = root / f'build/shots/{prefix}-{label}.png'
    subprocess.run(['python3', str(root / 'scripts/vm/vdrive.py'), '--dom', args.dom, 'shot', str(path)], check=True, capture_output=True)
    picture = Image.open(path).convert('RGB')
    return picture

def visible(picture, fraction=.05):
    small = picture.resize((160,90))
    pixels = small.load()
    return sum(max(pixels[x,y]) > 40 for y in range(90) for x in range(160)) > 160*90*fraction

assert not request('get_idle')['locked'], 'Start with the QA desktop unlocked'
v.send_keys(['esc'])
wait(lambda: request('get_idle')['stage'] == 'active', 'Desktop did not wake')
boot_id = guest('cat /proc/sys/kernel/random/boot_id')
wm_pid = guest('pgrep -x mindwm')
prefs = guest('sha256sum /home/qatest/.local/state/mindos/mindwm.json')
original_mode = next(s[1:-1] for s in guest('cat /sys/power/mem_sleep').split() if s.startswith('['))
try:
    guest(f'echo {args.mode} > /sys/power/mem_sleep')
    existing_windows = {w['id'] for w in request('get_windows')['windows']}
    guest(f'systemd-run --user -M qatest@ --collect --quiet --unit={unit} /usr/bin/kitty --title={prefix} /bin/sh -c ' + shlex.quote('printf "MindOS suspend recovery: this terminal stays open.\\n"; exec bash'))
    window = wait(lambda: next((w for w in request('get_windows')['windows'] if w['app_id'] == 'kitty' and w['id'] not in existing_windows), None), 'Terminal did not map')
    app_pid = guest(f'systemctl --user -M qatest@ show {unit}.service -p MainPID --value')
    print(f'Starting {args.mode}: boot={boot_id} compositor={wm_pid} application={app_pid}', flush=True)
    time.sleep(.5)
    before = shot('before'); assert visible(before)
    cursor = guest('journalctl -n 0 --show-cursor --no-pager').split('-- cursor: ')[-1]
    guest(f'systemd-run --collect --unit=mindos-{prefix} --on-active=2s --timer-property=AccuracySec=100ms /usr/bin/systemctl suspend')
    if args.mode == 'deep':
        wait(lambda: qmp('query-status')['status'] == 'suspended', 'QEMU did not enter S3', 20)
    else:
        time.sleep(6)
        assert not agent_ready(), 'Guest did not suspend; refusing to send a power-button event'
    shot('asleep')
    qmp('system_wakeup' if args.mode == 'deep' else 'system_powerdown')
    wait(agent_ready, 'Guest agent did not return', 25)
    assert guest('cat /proc/sys/kernel/random/boot_id') == boot_id, 'Guest rebooted'
    assert guest('pgrep -x mindwm') == wm_pid, 'Compositor restarted'
    wait(lambda: request('get_idle')['locked'] and request('get_idle')['stage'] == 'active', 'Locked display did not wake')
    time.sleep(1)
    locked = shot('locked')
    # Dark savers can leave only the clock and authentication card illuminated.
    # The failed QEMU inactive-display screen is below this smaller threshold.
    assert locked.size == before.size and visible(locked, .005), 'Lock screen not visibly restored'
    v.type_text('mindos'); v.send_keys(['ret'])
    wait(lambda: not request('get_idle')['locked'], 'Password unlock failed')
    assert guest(f'systemctl --user -M qatest@ show {unit}.service -p MainPID --value') == app_pid
    assert any(w['id'] == window['id'] for w in request('get_windows')['windows']), 'Terminal window lost'
    v.type_text('printf "APPLICATION SURVIVED\\n"'); v.send_keys(['ret'])
    time.sleep(.5); assert visible(shot('unlocked'))
    journal = guest('journalctl --after-cursor=' + shlex.quote(cursor) + ' --no-pager')
    (root / f'build/logs/{prefix}-journal.log').write_text(journal)
    assert f'PM: suspend entry ({args.mode})' in journal and 'PM: suspend exit' in journal
    if args.mode == 'deep':
        print('Checking for delayed watchdog/reset recovery (55 seconds)', flush=True)
        time.sleep(55)
        assert agent_ready(), 'Guest agent unavailable after delayed-reset observation'
        assert guest('cat /proc/sys/kernel/random/boot_id') == boot_id, 'Delayed guest reset'
        assert guest('pgrep -x mindwm') == wm_pid, 'Delayed compositor restart'
    print(f'PASS {args.mode}: real suspend/resume, same boot/compositor/app, visible lock and password recovery', flush=True)
finally:
    if agent_ready():
        guest(f'systemctl --user -M qatest@ stop {unit}.service 2>/dev/null || true')
        guest(f'echo {original_mode} > /sys/power/mem_sleep')
        assert guest('sha256sum /home/qatest/.local/state/mindos/mindwm.json') == prefs
        print('PASS original sleep selection and preferences restored; test terminal removed', flush=True)
