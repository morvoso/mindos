#!/usr/bin/env python3
"""Run the Wine tray probe and Octopi in an explicitly selected disposable QA VM."""
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
    program = ('import socket,json; s=socket.socket(socket.AF_UNIX); s.settimeout(5); '
               "s.connect('/run/user/1000/mindwm-wayland-1.sock'); "
               f's.sendall({payload.encode()!r}); print(s.makefile().readline())')
    reply = json.loads(guest('python3 -c ' + shlex.quote(program)))
    assert reply['ok'], reply
    return reply.get('result')


def windows():
    return request('get_windows')['windows']


def wait_for(test):
    for _ in range(80):
        value = test()
        if value:
            return value
        time.sleep(.15)
    raise AssertionError('Timed out waiting for the desktop')


def tray():
    code = '''import json, socket
s=socket.socket(socket.AF_UNIX); s.settimeout(5); s.connect('/run/user/1000/mindwm-wayland-1.sock')
s.sendall(b'{"type":"subscribe"}\\n')
for line in s.makefile():
    value=json.loads(line)
    if value.get('event') == 'tray':
        print(json.dumps(value['items'])); break
'''
    return json.loads(guest('python3 -c ' + shlex.quote(code)))


def probe_windows():
    return [w for w in windows() if 'MindOS Windows tray probe' in w['title']]


user = 'runuser -u qatest -- env XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus '
entry = '/home/qatest/.local/share/applications/mindos-win-mindos-tray-probe.desktop'
try:
    existing_icons = {item['id'] for item in tray()}
    code = "import runpy,pathlib; m=runpy.run_path('/usr/bin/mindos-win-open',run_name='winopen'); m['shortcut']('MindOS tray probe','mindos-tray-probe',pathlib.Path('/home/qatest/mindos/build/windows-tray-probe.exe'),pathlib.Path('/home/qatest/.local/share'))"
    guest(user + 'python3 -c ' + shlex.quote(code))
    # Allow the background desktop-entry index to notice the new app.
    time.sleep(4)
    request('mindbar', action='close')
    request('mindbar', action='open')
    v.type_text('MindOS tray probe')
    v.send_keys(['ret'])
    window = wait_for(probe_windows)[0]
    assert window['wine'], window
    icon = wait_for(lambda: next((item for item in tray() if item['id'] not in existing_icons), None))
    assert icon['pixels'], 'Tray icon must have visible pixels'
    print('PASS: new desktop entry launched from Mind; Windows type and tray icon published', flush=True)
    request('close', window=window['id'])
    wait_for(lambda: not probe_windows())
    # Wine's explorer tray and app use separate X connections. Allow the
    # app to consume WM_STATE withdrawal before requesting a restore.
    time.sleep(1)
    v.pointer(v.abs_xy(960, 540))
    time.sleep(.2)
    request('tray_click', icon=icon['id'], button=1)
    wait_for(probe_windows)
    print('PASS: close to tray and click to restore', flush=True)
    request('blank')
    assert request('get_idle')['stage'] == 'blank'
    request('wake')
    wait_for(lambda: request('get_idle')['stage'] == 'active')
    time.sleep(2)
    assert request('get_idle')['locked'], 'Wake must retain authentication'
    v.type_text('mindos')
    v.send_keys(['ret'])
    wait_for(lambda: not request('get_idle')['locked'])
    wait_for(probe_windows)
    print('PASS: display blank/wake, authenticated unlock and Windows app survival', flush=True)
    request('launch', exec='gio launch /usr/share/applications/octopi.desktop')
    wait_for(lambda: any('octopi' in w['app_id'].lower() for w in windows()))
    print('PASS: Octopi opens in the native desktop', flush=True)
finally:
    request('mindbar', action='close')
    guest(user + 'env WINEPREFIX=/home/qatest/.local/share/mindos/win/mindos-tray-probe wineserver -k')
    guest('rm -f -- ' + shlex.quote(entry))
