#!/usr/bin/env python3
"""Real keyboard/window QA. Requires an explicitly selected disposable VM.

Uses the shared build/keyboard-client, changes focus, and closes its own probes.
Does not change desktop preferences or restart the session.
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
parser.add_argument('--user', default='qatest')
args = parser.parse_args()
root = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('vdrive', root / 'scripts/vm/vdrive.py')
v = importlib.util.module_from_spec(spec)
spec.loader.exec_module(v)
v.DOM = args.dom
share = f'/home/{args.user}/mindos'
prefix = 'mindos-keyboard-qa-'
held = set()
caps = False

def guest(command):
    rc, out, err = v.guest_exec(command)
    assert rc == 0, (command, out, err)
    return out

def request(kind, **kwargs):
    payload = json.dumps(dict(type=kind, **kwargs)) + '\n'
    program = ('import socket,json; s=socket.socket(socket.AF_UNIX); s.settimeout(5); '
               "s.connect('/run/user/1000/mindwm-wayland-1.sock'); "
               f's.sendall({payload.encode()!r}); print(s.makefile().readline())')
    result = json.loads(guest('python3 -c ' + shlex.quote(program)))
    assert result['ok'], result
    return result.get('result')

def windows(): return request('get_windows')['windows']

def wait(predicate, message):
    deadline = time.monotonic() + 8
    while time.monotonic() < deadline:
        result = predicate()
        if result: return result
        time.sleep(.1)
    raise AssertionError(message)

def find(label):
    return next((w for w in windows() if w['title'] == prefix + label), None)

def focus(label):
    window = find(label)
    assert window, label
    request('focus', window=window['id'])
    focused(label)

def focused(label):
    return wait(lambda: next((w for w in windows() if w['focused'] and w['title'] == prefix + label), None),
                'Expected focus on ' + label)

def hold(key, down):
    v.key_hold(key, down)
    if down: held.add(key)
    else: held.discard(key)
    time.sleep(.05)

def tap(key):
    hold(key, True)
    hold(key, False)
    time.sleep(.12)

def log(label):
    path = root / f'build/logs/qa-keyboard-{label}.log'
    return path.read_text() if path.exists() else ''

def start(label, color, inhibit=False, x11=False):
    executable = 'x11-keyboard-client' if x11 else 'keyboard-client'
    arguments = f'{prefix + label} {color}' if x11 else f'{prefix + label} {int(inhibit)} {color}'
    command = (f'timeout 180 {shlex.quote(share)}/build/{executable} '
               f'{arguments} > '
               f'{shlex.quote(share)}/build/logs/qa-keyboard-{label}.log 2>&1')
    guest(f'systemd-run --user -M {shlex.quote(args.user)}@ --collect --quiet '
          f'--unit={prefix + label} /bin/sh -c {shlex.quote(command)}')
    wait(lambda: find(label), 'Probe did not map: ' + label)
    focused(label)

def screenshot(name, expected):
    path = root / f'build/shots/qa-keyboard-{name}.png'
    subprocess.run(['python3', str(root / 'scripts/vm/vdrive.py'), '--dom', args.dom,
                    'shot', str(path)], check=True, capture_output=True)
    picture = Image.open(path).convert('RGB')
    center = picture.getpixel((picture.width // 2, picture.height // 2))
    assert center == expected, (name, center, expected)

assert args.dom.startswith('mindos-qa-'), 'Use a disposable MindOS QA VM'
assert not windows(), 'Start with an empty disposable QA desktop'
try:
    v.pointer(v.abs_xy(8, 8))
    start('a', '224466')
    start('b', '226644')
    start('c', '662244')
    hold('alt', True); tap('tab'); focused('b')
    tap('tab'); focused('a')
    hold('shift', True); tap('tab'); focused('b'); hold('shift', False)
    tap('esc'); focused('c'); hold('alt', False)
    hold('alt', True); tap('tab'); focused('b'); hold('alt', False)
    hold('alt', True); tap('tab'); focused('c'); hold('alt', False)
    print('PASS stable held cycling, reverse, Escape cancellation and quick recent-window return', flush=True)

    # Shift is released before Tab: applications must not receive an orphaned
    # Tab release with a different keysym from the intercepted press.
    hold('alt', True); hold('shift', True); hold('tab', True)
    hold('shift', False); hold('alt', False); hold('tab', False)
    for label in ('a', 'b', 'c'):
        assert 'KEY 15 ' not in log(label), (label, log(label))
    print('PASS consumed Tab press/release stays out of clients when Shift is released first', flush=True)

    focus('b'); focus('a')
    request('toggle_fullscreen', window=find('a')['id'])
    wait(lambda: 'FULLSCREEN 1' in log('a'), 'Fullscreen configure missing')
    time.sleep(.3); screenshot('fullscreen', (34, 68, 102))
    hold('alt', True); tap('tab'); focused('b'); hold('alt', False)
    time.sleep(.3); screenshot('away-from-game', (34, 102, 68))
    assert find('a')['fullscreen'], 'Switching away changed the game state'
    hold('alt', True); tap('tab'); focused('a'); hold('alt', False)
    time.sleep(.3); screenshot('return-to-game', (34, 68, 102))
    request('toggle_fullscreen', window=find('a')['id'])
    print('PASS fullscreen focus switch changes visible pixels and preserves game fullscreen state', flush=True)

    focus('c'); tap('caps_lock'); caps = True
    hold('meta_l', True); tap('m'); hold('meta_l', False)
    wait(lambda: find('c')['maximized'], 'Caps Lock disabled Super+M')
    hold('meta_l', True); tap('m'); hold('meta_l', False)
    tap('caps_lock'); caps = False
    hold('alt', True); tap('f4'); hold('alt', False)
    wait(lambda: find('c') is None, 'Alt+F4 did not close the window')
    assert 'CLOSED' in log('c')
    print('PASS Caps Lock shortcuts and real xdg_toplevel close', flush=True)

    start('x11', '664422', x11=True)
    assert find('x11')['x11']
    focus('b'); focus('x11')
    request('toggle_fullscreen', window=find('x11')['id'])
    wait(lambda: find('x11')['fullscreen'], 'XWayland did not enter fullscreen')
    time.sleep(.3); screenshot('x11-fullscreen', (102, 68, 34))
    hold('alt', True); tap('tab'); focused('b'); hold('alt', False)
    time.sleep(.3); screenshot('x11-away-from-game', (34, 102, 68))
    assert find('x11')['fullscreen']
    hold('alt', True); tap('tab'); focused('x11'); hold('alt', False)
    time.sleep(.3); screenshot('x11-return-to-game', (102, 68, 34))
    hold('alt', True); tap('f4'); hold('alt', False)
    wait(lambda: find('x11') is None, 'XWayland close failed')
    assert 'CLOSED' in log('x11')
    print('PASS XWayland fullscreen switching, visible pixels and WM_DELETE_WINDOW close', flush=True)

    start('inhibited', '665522', inhibit=True)
    wait(lambda: 'INHIBITED' in log('inhibited'), 'Shortcut inhibition not active')
    hold('alt', True); tap('f4'); tap('tab'); hold('alt', False)
    focused('inhibited')
    for key in (62, 15):
        assert f'KEY {key} 1' in log('inhibited') and f'KEY {key} 0' in log('inhibited'), log('inhibited')
    print('PASS inhibited client receives Alt+F4 and Alt+Tab without close/switch', flush=True)
finally:
    for key in list(held): hold(key, False)
    if caps: tap('caps_lock')
    for window in windows():
        if window['title'].startswith(prefix): request('close', window=window['id'])
    wait(lambda: not any(w['title'].startswith(prefix) for w in windows()), 'Probe cleanup failed')
