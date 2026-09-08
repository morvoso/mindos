#!/usr/bin/env python3
"""Exercise inactive-session GUI launches in an explicitly selected QA VM."""
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
parser.add_argument('--expect-blocked', action='store_true', help='Record the old video-group failure')
args = parser.parse_args()
assert args.dom.startswith('mindos-qa-'), 'Disposable QA guests only'
root = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('vdrive', root / 'scripts/vm/vdrive.py')
v = importlib.util.module_from_spec(spec)
spec.loader.exec_module(v)
v.DOM = args.dom
share = '/home/qatest/mindos'

def guest(command):
    rc, out, err = v.guest_exec(command)
    assert rc == 0, (command, out, err)
    return out.strip()

def request(kind, **kwargs):
    payload = json.dumps(dict(type=kind, **kwargs)) + '\n'
    program = ('import socket; s=socket.socket(socket.AF_UNIX); s.settimeout(5); '
               "s.connect('/run/user/1000/mindwm-wayland-1.sock'); "
               f's.sendall({payload.encode()!r}); print(s.makefile().readline())')
    result = json.loads(guest('python3 -c ' + shlex.quote(program)))
    assert result['ok'], result
    return result.get('result')

def windows(): return request('get_windows')['windows']

def wait(predicate, message):
    deadline = time.monotonic() + 12
    while time.monotonic() < deadline:
        result = predicate()
        if result: return result
        time.sleep(.1)
    raise AssertionError(message)

def clients():
    return guest('cat /sys/kernel/debug/dri/*/clients')

def screenshot(name):
    path = root / f'build/shots/qa-session-graphics-{name}.png'
    subprocess.run(['python3', str(root / 'scripts/vm/vdrive.py'), '--dom', args.dom, 'shot', str(path)], check=True, capture_output=True)
    return Image.open(path).convert('RGB')

assert not windows(), 'Start with an empty QA desktop'
prefs = guest('sha256sum /home/qatest/.local/state/mindos/mindwm.json')
wm_pid = guest('pgrep -x mindwm')
try:
    v.key_hold('esc', True); v.key_hold('esc', False)
    guest(f'systemd-run --user -M qatest@ --collect --quiet --unit=mindos-qa-session-peer timeout 180 {share}/build/keyboard-client mindos-qa-session-peer 0 224466')
    peer = wait(lambda: next((w for w in windows() if w['title'] == 'mindos-qa-session-peer'), None), 'Peer not mapped')
    request('toggle_fullscreen', window=peer['id'])
    time.sleep(.4)
    guest('chvt 2')
    time.sleep(.5)
    print(guest('getfacl -p /dev/dri/card1 /dev/dri/renderD128'), flush=True)
    guest('systemd-run --user -M qatest@ --collect --quiet --unit=mindos-qa-session-settings /usr/bin/mindshell --app settings --page shell')
    settings = wait(lambda: next((w for w in windows() if w['app_id'] == 'mindos-settings'), None), 'Inactive Settings did not map')
    pid = guest('systemctl --user -M qatest@ show mindos-qa-session-settings.service -p MainPID --value')
    time.sleep(.5)
    state = clients()
    (root / f'build/logs/qa-session-graphics-clients-{int(args.expect_blocked)}.log').write_text(state)
    is_master = any(len(fields := line.split()) >= 4 and fields[1] == pid and fields[3] == 'y' for line in state.splitlines())
    assert is_master == args.expect_blocked, (pid, state)
    guest('chvt 1')
    time.sleep(.7)
    request('focus', window=peer['id'])
    picture = screenshot('before' if args.expect_blocked else 'restored')
    pixel = picture.getpixel((picture.width//2, picture.height//2))
    assert (pixel == (34,68,102)) != args.expect_blocked, pixel
    if args.expect_blocked:
        print('CONFIRMED old video-group failure: inactive Settings becomes master and blocks desktop pixels', flush=True)
    else:
        request('focus', window=settings['id'])
        time.sleep(.4)
        screenshot('settings')
        print('PASS inactive Settings maps without acquiring DRM master; VT return restores pixels and Settings', flush=True)
    assert guest('pgrep -x mindwm') == wm_pid
finally:
    guest('systemctl --user -M qatest@ stop mindos-qa-session-settings.service mindos-qa-session-peer.service 2>/dev/null || true')
    guest('chvt 2'); time.sleep(.2); guest('chvt 1')
    assert guest('sha256sum /home/qatest/.local/state/mindos/mindwm.json') == prefs
    print('PASS original compositor and preferences retained; test windows removed', flush=True)
