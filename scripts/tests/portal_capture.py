#!/usr/bin/env python3
"""Interactive portal/PipeWire probe; run as a user in a disposable desktop.

Requires python-gobject, gstreamer, gst-plugin-pipewire and gst-plugins-good.
Select Share display to save one PNG, or use --cancel and press Cancel.
"""
import argparse
import os
from pathlib import Path
import subprocess
import uuid
from gi.repository import Gio, GLib

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--cancel', action='store_true')
parser.add_argument('--output', type=Path, default=Path('/tmp/mindos-portal-frame.png'))
args = parser.parse_args()
os.umask(0o077)
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
portal = 'org.freedesktop.portal.Desktop'
path = '/org/freedesktop/portal/desktop'
interface = 'org.freedesktop.portal.ScreenCast'
cancelled = False


def request(method, signature, values):
    global cancelled
    loop, responses = GLib.MainLoop(), []

    def received(_bus, _sender, _path, _interface, _signal, params):
        responses.append(params.unpack())
        loop.quit()

    subscription = bus.signal_subscribe(portal, 'org.freedesktop.portal.Request',
        'Response', None, None, Gio.DBusSignalFlags.NONE, received)
    try:
        bus.call_sync(portal, path, interface, method, GLib.Variant(signature, values),
                      None, Gio.DBusCallFlags.NONE, 10000, None)
        timeout = GLib.timeout_add_seconds(45, lambda: (loop.quit(), False)[1])
        loop.run()
        if responses:
            GLib.source_remove(timeout)
        print(method, responses, flush=True)
        assert responses, 'Portal response timed out'
        code, result = responses[0]
        if args.cancel and code == 1:
            cancelled = True
            print('Cancellation confirmed: no PipeWire stream opened', flush=True)
            raise SystemExit(0)
        assert code == 0, f'Portal failed: {code}'
        return result
    finally:
        bus.signal_unsubscribe(subscription)


session = request('CreateSession', '(a{sv})', ({
    'session_handle_token': GLib.Variant('s', 'qa' + uuid.uuid4().hex),
},))['session_handle']
try:
    request('SelectSources', '(oa{sv})', (session, {
        'types': GLib.Variant('u', 1),
        'cursor_mode': GLib.Variant('u', 2),
        'persist_mode': GLib.Variant('u', 0),
    }))
    result = request('Start', '(osa{sv})', (session, '', {}))
    assert not args.cancel, 'Sharing was approved when cancellation was expected'
    handle, fds = bus.call_with_unix_fd_list_sync(portal, path, interface,
        'OpenPipeWireRemote', GLib.Variant('(oa{sv})', (session, {})),
        GLib.VariantType.new('(h)'), Gio.DBusCallFlags.NONE, 10000, None, None)
    fd = fds.get(handle.unpack()[0])
    node, _properties = result['streams'][0]
    try:
        subprocess.run(['gst-launch-1.0', '-e', 'pipewiresrc', f'fd={fd}',
            f'path={node}', 'num-buffers=1', '!', 'videoconvert', '!', 'pngenc',
            'snapshot=true', '!', 'filesink', f'location={args.output.resolve()}'],
            pass_fds=(fd,), timeout=25, check=True)
    finally:
        os.close(fd)
finally:
    try:
        bus.call_sync(portal, session, 'org.freedesktop.portal.Session', 'Close',
                      None, None, Gio.DBusCallFlags.NONE, 10000, None)
    except GLib.Error:
        if not cancelled:
            raise
