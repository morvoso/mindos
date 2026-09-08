#!/usr/bin/env python3
"""Native media-key checks in an explicitly selected disposable QA guest."""
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
args = parser.parse_args()
assert args.dom.startswith('mindos-qa-'), 'Disposable QA guests only'
root = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('vdrive', root / 'scripts/vm/vdrive.py')
v = importlib.util.module_from_spec(spec)
spec.loader.exec_module(v)
v.DOM = args.dom
share = '/home/qatest/mindos'
source = None
held = set()
locked = False

def guest(command):
    rc, out, err = v.guest_exec(command)
    assert rc == 0, (command, out, err)
    return out.strip()

def user(command):
    return guest('runuser -u qatest -- env XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus ' + command)

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
    deadline = time.monotonic() + 8
    while time.monotonic() < deadline:
        result = predicate()
        if result: return result
        time.sleep(.1)
    raise AssertionError(message)

def key(code, down):
    program = ("import socket; s=socket.socket(socket.AF_UNIX); s.settimeout(2); "
               "s.connect('/run/mindos-qa-media-keys.sock'); "
               f's.sendall({f"{code} {int(down)}".encode()!r}); assert s.recv(2)==b"OK"')
    guest('python3 -c ' + shlex.quote(program))
    (held.add if down else held.discard)(code)

def tap(code):
    key(code, True); time.sleep(.03); key(code, False); time.sleep(.18)

def volume(target='SINK'):
    result = user(f'wpctl get-volume @DEFAULT_AUDIO_{target}@')
    return float(result.split()[1]), '[MUTED]' in result

def screenshot(name):
    path = root / f'build/shots/qa-media-{name}.png'
    subprocess.run(['python3', str(root / 'scripts/vm/vdrive.py'), '--dom', args.dom, 'shot', str(path)], check=True, capture_output=True)
    return Image.open(path).convert('RGB')

def start(label, inhibit=False):
    command = f'timeout 150 {share}/build/keyboard-client mindos-media-qa-{label} {int(inhibit)} 224466 > {share}/build/logs/qa-media-{label}.log 2>&1'
    guest(f'systemd-run --user -M qatest@ --collect --quiet --unit=mindos-media-qa-{label} /bin/sh -c ' + shlex.quote(command))
    return wait(lambda: next((w for w in windows() if w['title'] == 'mindos-media-qa-' + label), None), 'Probe did not map')

assert not windows(), 'Start with an empty QA desktop'
original = volume()
prefs = guest('sha256sum /home/qatest/.local/state/mindos/mindwm.json')
try:
    guest('modprobe uinput')
    guest('systemctl stop mindos-qa-media-input.service 2>/dev/null || true')
    guest(f'systemd-run --collect --quiet --unit=mindos-qa-media-input python3 {share}/scripts/tests/media_input.py')
    time.sleep(1)
    v.key_hold('esc', True); v.key_hold('esc', False); time.sleep(.3)
    user('wpctl set-volume @DEFAULT_AUDIO_SINK@ 0.4')
    user('wpctl set-mute @DEFAULT_AUDIO_SINK@ 0')
    probe = start('fullscreen')
    request('toggle_fullscreen', window=probe['id'])
    time.sleep(.3)
    tap(115)
    wait(lambda: volume() == (.45, False), 'Volume up failed')
    picture = screenshot('volume-fullscreen')
    # Fullscreen peer remains focused and visible behind the native OSD.
    assert next(w for w in windows() if w['focused'])['id'] == probe['id']
    assert picture.getpixel((picture.width//2, picture.height//2)) == (34,68,102)
    point = (picture.width//2, picture.height-88-46)
    assert picture.getpixel(point) != (34,68,102), 'OSD not visible over fullscreen'
    time.sleep(2)
    assert screenshot('expired').getpixel(point) == (34,68,102), 'OSD did not expire'
    key(115, True); time.sleep(.95); key(115, False); time.sleep(.2)
    after = volume()
    assert after[0] >= .65, after
    time.sleep(.5)
    assert volume() == after, 'Volume kept repeating after release'
    user('wpctl set-volume @DEFAULT_AUDIO_SINK@ 0.98')
    tap(115); assert volume() == (1.0, False)
    tap(113); assert volume() == (1.0, True)
    tap(114); assert volume() == (.95, False)
    print('PASS volume step, held repeat/release, 100% cap, mute/unmute and cached fullscreen OSD expiry', flush=True)

    user('pw-cli create-node adapter ' + shlex.quote('{ factory.name=support.null-audio-sink node.name=mindos_qa_source node.description="QA microphone" media.class=Audio/Source/Virtual object.linger=true audio.position=[ MONO ] }'))
    time.sleep(.5)
    nodes = json.loads(user('pw-dump'))
    source = next(n['id'] for n in nodes if n.get('info',{}).get('props',{}).get('node.name') == 'mindos_qa_source')
    tap(248); assert volume('SOURCE')[1]
    tap(248); assert not volume('SOURCE')[1]
    assert volume() == (.95, False), 'Mic key changed output volume'
    print('PASS independent microphone mute/unmute using a real PipeWire virtual source', flush=True)
    assert not guest('ls -A /sys/class/backlight'), 'This test expects no guest backlight'
    tap(225); screenshot('brightness-unavailable')
    print('PASS absent-backlight path remains responsive (visual feedback captured)', flush=True)
    request('close', window=probe['id'])
    wait(lambda: not windows(), 'Probe close failed')

    inhibited = start('inhibited', True)
    path = root / 'build/logs/qa-media-inhibited.log'
    wait(lambda: path.exists() and 'INHIBITED' in path.read_text(), 'Inhibition missing')
    before = volume(); tap(115)
    assert volume() == before
    text = path.read_text()
    assert 'KEY 115 1' in text and 'KEY 115 0' in text, text
    request('close', window=inhibited['id'])
    wait(lambda: not windows(), 'Inhibited probe close failed')
    print('PASS shortcut-inhibited client receives media key press/release without system volume changes', flush=True)

    program = "import wave; w=wave.open('/tmp/mindos-qa-media.wav','wb'); w.setparams((1,2,8000,0,'NONE','not compressed')); w.writeframes(bytes(8000*2*120)); w.close()"
    guest('python3 -c ' + shlex.quote(program))
    guest('systemd-run --user -M qatest@ --collect --quiet --unit=mindos-media-qa-player /usr/bin/celluloid /tmp/mindos-qa-media.wav')
    wait(lambda: user('/bin/sh -c ' + shlex.quote('playerctl status 2>/dev/null || true')) == 'Playing', 'Player not playing')
    tap(164); wait(lambda: user('/bin/sh -c ' + shlex.quote('playerctl status 2>/dev/null || true')) == 'Paused', 'Play/pause did not pause')
    tap(164); wait(lambda: user('/bin/sh -c ' + shlex.quote('playerctl status 2>/dev/null || true')) == 'Playing', 'Play/pause did not resume')
    tap(166); wait(lambda: user('/bin/sh -c ' + shlex.quote('playerctl status 2>/dev/null || true')) == 'Stopped', 'Stop failed')
    print('PASS hardware play/pause/resume/stop against Celluloid MPRIS', flush=True)
    user('wpctl set-volume @DEFAULT_AUDIO_SINK@ 0.4')
    key(115, True); time.sleep(.15)
    request('lock'); locked = True
    time.sleep(.4); before = volume(); time.sleep(.6)
    assert volume() == before, 'Lock did not cancel held repetition'
    key(115, False); tap(113)
    assert volume() == before, 'Locked media key changed volume'
    screenshot('locked')
    request('unlock'); locked = False
    print('PASS locking cancels held repeat and blocks media controls', flush=True)
    key(115, True); time.sleep(.15)
    guest('systemctl stop mindos-qa-media-input.service'); held.clear()
    time.sleep(.4); before = volume(); time.sleep(.6)
    assert volume() == before, 'Removed keyboard kept repeating'
    print('PASS keyboard unplug cancels held repetition', flush=True)
finally:
    if locked: request('unlock')
    for code in list(held): key(code, False)
    for window in windows():
        if window['title'].startswith('mindos-media-qa-'): request('close', window=window['id'])
    guest('systemctl --user -M qatest@ stop mindos-media-qa-player.service 2>/dev/null || true')
    if source is not None: user(f'pw-cli destroy {source}')
    user(f'wpctl set-volume @DEFAULT_AUDIO_SINK@ {original[0]}')
    user(f'wpctl set-mute @DEFAULT_AUDIO_SINK@ {int(original[1])}')
    guest('systemctl stop mindos-qa-media-input.service; rm -f /tmp/mindos-qa-media.wav')
    assert volume() == original
    assert guest('sha256sum /home/qatest/.local/state/mindos/mindwm.json') == prefs
    print('PASS audio and preferences restored; native test devices and player removed', flush=True)
