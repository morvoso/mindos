#!/usr/bin/env python3
"""Drive pointer_client on an explicitly selected disposable VM (1920x1080).
Requires the compiled probe in its shared MindOS tree. Mutates pointer/focus.
"""
import argparse
import importlib.util
import pathlib
import shlex
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--dom', required=True)
parser.add_argument('--user', default='qatest')
args = parser.parse_args()
root = pathlib.Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('vdrive', root / 'scripts/vm/vdrive.py')
v = importlib.util.module_from_spec(spec)
spec.loader.exec_module(v)
v.DOM = args.dom
share = f'/home/{args.user}/mindos'
log = root / 'build/logs/qa-pointer-probe.log'

def move(x, y):
    v.pointer(v.abs_xy(x, y))
    time.sleep(0.15)

def read():
    return log.read_text() if log.exists() else ''

def wait_for(token):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if token in read():
            return read()
        time.sleep(0.1)
    raise AssertionError(f'Timed out waiting for {token}: {read()}')

def start(mode):
    move(800, 700)
    command = f'timeout 60 {shlex.quote(share)}/build/pointer-client {mode} > {shlex.quote(share)}/build/logs/qa-pointer-probe.log 2>&1 < /dev/null &'
    rc, out, err = v.guest_exec(f'runuser -u {shlex.quote(args.user)} -- env XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-1 sh -c {shlex.quote(command)}')
    assert rc == 0, (out, err)
    wait_for('MAPPED')
    wait_for('FULLSCREEN 1')
    move(810, 710)
    wait_for('CREATED')

def release():
    v.send_keys(['spc'])
    wait_for('RELEASED')
    move(800, 700)
    assert 'MOTION 800.' in read().split('RELEASED')[-1], read()
    v.send_keys(['esc'])
    time.sleep(0.25)

start('lock')
assert 'LOCKED' not in read(), read()  # creation outside region must stay inactive
move(200, 200)
wait_for('LOCKED')
move(210, 205)
move(220, 210)
locked = read().split('LOCKED\n')[-1]
assert 'MOTION' not in locked, locked  # includes commit of the advisory hint
relative = [line.split() for line in locked.splitlines() if line.startswith('RELATIVE ')]
assert len(relative) == 2, relative
for fields in relative:
    assert abs(float(fields[1]) - 10) < 0.15 and abs(float(fields[2]) - 5) < 0.15, fields
# QEMU also exposes a relative PS/2 mouse. Verify its unaccelerated delta
# survives the compositor path while the same client holds the lock.
v.pointer([{'type': 'rel', 'data': {'axis': 'x', 'value': 12}},
           {'type': 'rel', 'data': {'axis': 'y', 'value': 4}}])
time.sleep(0.2)
relative = [line for line in read().splitlines() if line.startswith('RELATIVE ')][-1]
assert 'raw=12.0000,4.0000' in relative, relative
assert 'MOTION' not in read().split('LOCKED\n')[-1], read()
release()
(root / 'build/logs/qa-pointer-lock-passed.log').write_text(read())
print('PASS absolute pointer lock, incremental relative deltas, region activation, advisory hint and release')

start('confine')
assert 'CONFINED' not in read(), read()
move(200, 200)
wait_for('CONFINED')
move(500, 200)  # endpoint valid, but crosses the excluded vertical strip
motion = [line.split() for line in read().splitlines() if line.startswith('MOTION ')][-1]
assert 299.9 < float(motion[1]) < 300, motion
move(550, 500)  # slide along the boundary, retaining vertical movement
motion = [line.split() for line in read().splitlines() if line.startswith('MOTION ')][-1]
assert 299.9 < float(motion[1]) < 300 and 499 < float(motion[2]) < 502, motion
release()
(root / 'build/logs/qa-pointer-confine-passed.log').write_text(read())
print('PASS absolute pointer confinement, excluded hole, edge sliding and release')

def ipc(kind):
    import json
    rc, out, err = v.guest_exec(f'python3 {shlex.quote(share)}/build/vm-query.py {shlex.quote(json.dumps({"type": kind}))}')
    assert rc == 0 and json.loads(out)['ok'], (out, err)

start('lock')
move(200, 200)
wait_for('LOCKED')
try:
    ipc('lock')
    wait_for('LEAVE')
    after_lock = read()
    move(230, 210)
    v.pointer([{'type': 'rel', 'data': {'axis': 'x', 'value': 12}}])
    time.sleep(0.2)
    assert read() == after_lock, read()
    (root / 'build/logs/qa-pointer-session-lock-passed.log').write_text(read())
    print('PASS session lock releases game mouse focus and stops both motion paths')
finally:
    ipc('unlock')
    v.guest_exec(f'pkill -TERM -u {shlex.quote(args.user)} -x pointer-client')
