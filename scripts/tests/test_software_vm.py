#!/usr/bin/env python3
"""Exercise Octopi transactions and dialog stacking in a disposable 1920x1080 QA VM."""
import argparse
import importlib.util
import json
from pathlib import Path
import shlex
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--dom', required=True)
args = parser.parse_args()
assert args.dom.startswith('mindos-qa-'), 'Disposable QA domains only'
root = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('vdrive', root / 'scripts/vm/vdrive.py')
v = importlib.util.module_from_spec(spec)
spec.loader.exec_module(v)
v.DOM = args.dom


def guest(command):
    rc, out, err = v.guest_exec(command)
    assert rc == 0, (out, err)
    return out.strip()


def request(kind, **kwargs):
    payload = json.dumps(dict(type=kind, **kwargs)) + '\n'
    code = ("import socket; s=socket.socket(socket.AF_UNIX); s.settimeout(5); "
            "s.connect('/run/user/1000/mindwm-wayland-1.sock'); "
            f"s.sendall({payload.encode()!r}); print(s.makefile().readline())")
    reply = json.loads(guest('python3 -c ' + shlex.quote(code)))
    assert reply['ok'], reply
    return reply.get('result')


def windows():
    return request('get_windows')['windows']


def wait_for(test):
    for _ in range(100):
        value = test()
        if value:
            return value
        time.sleep(.2)
    raise AssertionError('Timed out waiting for the desktop')


def installed():
    return guest('if pacman -Q figlet >/dev/null 2>&1; then echo yes; else echo no; fi') == 'yes'


assert not installed(), 'figlet must be absent before this disposable test'
assert not windows(), 'Start with an empty QA desktop'
request('mindbar', action='open')
v.type_text('octopi')
time.sleep(.4)
v.send_keys(['ret'])
manager = wait_for(lambda: next((w for w in windows() if w['title'] == 'Octopi'), None))
command = guest('ps -C octopi -o args=')
assert '-stylesheet /usr/share/mindos/octopi.qss' in command, command
print('PASS: Mind search opens the styled software manager', flush=True)
time.sleep(2)
v.send_keys(['ctrl-l'])
time.sleep(.3)
v.type_text('figlet')
time.sleep(1.5)

for removing in (False, True):
    v.click(666, 380, 'right')
    time.sleep(.5)
    v.click(710, 486 if removing else 438)
    time.sleep(1)
    v.send_keys(['ctrl-y'])
    dialog = wait_for(lambda: next((w for w in windows() if w['title'] == 'Confirmation'), None))
    time.sleep(.5)
    # A click on exposed parent content must keep its dialog on top/focused.
    v.click(620, 450)
    time.sleep(.4)
    assert request('get_windows')['focused'] == dialog['id'], windows()
    request('focus', window=manager['id'])
    assert request('get_windows')['focused'] == dialog['id'], windows()
    print('PASS: parent click/activation preserves the confirmation dialog', flush=True)
    v.send_keys(['alt-y'])
    wait_for(lambda: any('qt-sudo' in w['app_id'] for w in windows()))
    time.sleep(.5)
    v.type_text('mindos')  # Disposable fixture credential, never a personal desktop.
    time.sleep(.3)
    v.send_keys(['ret'])
    wait_for(lambda: installed() != removing)
    wait_for(lambda: not any('qt-sudo' in w['app_id'] for w in windows()))
    time.sleep(2)
    print('PASS: graphical ' + ('removal' if removing else 'installation') + ' with password authentication', flush=True)

request('close', window=manager['id'])
wait_for(lambda: not windows())
print('PASS: software manager closes after both transactions', flush=True)
