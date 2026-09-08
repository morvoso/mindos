#!/usr/bin/env python3
"""QA VM: system pages stay embedded; native tile/column windows keep owners.
Run as the desktop user. Creates and closes only its own test GTK windows.
"""
import json, os, pathlib, socket, time, sys, shlex
if len(sys.argv) > 1 and sys.argv[1] == "--window":
    import gi
    gi.require_version("Gtk", "4.0")
    from gi.repository import Gtk, Gio
    app = Gtk.Application(application_id="org.mindos.WorkspaceTest", flags=Gio.ApplicationFlags.NON_UNIQUE)
    def activate(app):
        w=Gtk.ApplicationWindow(application=app, title="Workspace test"); w.set_default_size(480, 320)
        w.set_child(Gtk.Label(label="Workspace layout verification")); w.present()
    app.connect("activate", activate); app.run([]); sys.exit(0)
path = pathlib.Path(os.environ['XDG_RUNTIME_DIR']) / f"mindwm-{os.environ['WAYLAND_DISPLAY']}.sock"
def call(kind, **data):
    with socket.socket(socket.AF_UNIX) as s:
        s.settimeout(5); s.connect(str(path)); s.sendall((json.dumps(dict(type=kind, **data))+'\n').encode())
        reply=json.loads(s.makefile().readline())
        assert reply['ok'], reply
        return reply.get('result', {})
def windows(): return call('get_windows')['windows']
original = call('get_layout_mode')['mode']
try:
    for i in range(3): call('launch', exec=f'python3 {shlex.quote(str(pathlib.Path(__file__).resolve()))} --window')
    for _ in range(100):
        test = [w for w in windows() if w['app_id']=='org.mindos.WorkspaceTest']
        if len(test)==3: break
        time.sleep(.1)
    assert len(test)==3, windows()
    ids=[w['id'] for w in test]
    owners={w['id']:w['output'] for w in test}
    assert all(owners.values()), owners
    for mode in ('dwindle','columns'):
        call('set_layout_mode', mode=mode)
        for ident in ids + ids[::-1]:
            call('focus', window=ident); time.sleep(.35)
            assert {w['id']:w['output'] for w in windows() if w['id'] in ids} == owners
    before={w['id'] for w in windows()}
    call('launch', exec='mindshell --app settings --page shell'); time.sleep(1)
    assert {w['id'] for w in windows()} == before, 'Settings opened a separate window'
    call('launch', exec='mindshell --app gaming'); time.sleep(1)
    assert {w['id'] for w in windows()} == before, 'Gaming Center opened a separate window'
    print('PASS: tile/column focus preserves window ownership; system app launches stay embedded')
finally:
    for w in windows():
        if w['app_id']=='org.mindos.WorkspaceTest': call('close', window=w['id'])
    call('set_layout_mode', mode=original)
    call('launch', exec='mindshell --app library')
