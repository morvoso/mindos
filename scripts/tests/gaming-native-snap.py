#!/usr/bin/env python3
"""QA VM only: snap/release and fullscreen restoration through compositor IPC."""
import json,pathlib,socket,time
path=next(pathlib.Path('/run/user/1000').glob('mindwm-*.sock'))
def call(p):
    with socket.socket(socket.AF_UNIX) as s:
        s.settimeout(5)
        s.connect(str(path));s.sendall((json.dumps(p)+'\n').encode());return json.loads(s.makefile().readline())
def windows():return call(dict(type='get_windows'))['result']['windows']
window=next(w for w in windows() if w['app_id']=='mindos-gaming')['id']
def state():return next(w for w in windows() if w['id']==window)
def wait_fullscreen(value):
    for _ in range(60):
        if state()['fullscreen']==value:return
        time.sleep(.1)
    raise AssertionError(state())
call(dict(type='snap',window=window,zone='release'))
if state()['fullscreen']:call(dict(type='toggle_fullscreen',window=window));wait_fullscreen(False)
assert call(dict(type='toggle_fullscreen',window=window))['ok'];wait_fullscreen(True)
assert call(dict(type='snap',window=window,zone='left-two-thirds'))['ok'];wait_fullscreen(False)
assert call(dict(type='snap',window=window,zone='release'))['ok'];wait_fullscreen(True)
call(dict(type='toggle_fullscreen',window=window));wait_fullscreen(False)
assert not call(dict(type='snap',window=window,zone='invalid'))['ok']
assert not call(dict(type='snap',window=window,zone='right-third',output='not-connected'))['ok']
print('PASS native snapping: fullscreen restored, invalid zones and disconnected displays rejected')
