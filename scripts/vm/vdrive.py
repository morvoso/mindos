#!/usr/bin/env python3
"""Drive a libvirt VM: vdrive.py [--dom NAME] shot out.png | keys combo... | type "text" | exec "cmd" | state
                                  | move x y | click x y [left|right|middle] | rclick x y | dblclick x y | drag x1 y1 x2 y2
                                  | press [btn] | release [btn] | wheel up|down [n] | keydown name | keyup name
Key names follow qemu-drive.py (meta_l-spc, ctrl-alt-f2, ret, esc, spc, ...), mapped to linux keycodes.
Pointer coordinates are screen pixels (VDRIVE_SCREEN=WxH, default 1920x1080) sent as absolute USB-tablet events."""
import os, subprocess, sys, time, zlib, struct
DOM = os.environ.get("VDOM", "mindos-dev")
URI = "qemu:///system"
S = os.environ.get("VDRIVE_TMP", "/tmp/mindos-vdrive")
os.makedirs(S, exist_ok=True)

NAMES = {'ret': 'KEY_ENTER', 'spc': 'KEY_SPACE', 'esc': 'KEY_ESC', 'tab': 'KEY_TAB', 'backspace': 'KEY_BACKSPACE',
         'minus': 'KEY_MINUS', 'equal': 'KEY_EQUAL', 'dot': 'KEY_DOT', 'comma': 'KEY_COMMA', 'slash': 'KEY_SLASH',
         'backslash': 'KEY_BACKSLASH', 'semicolon': 'KEY_SEMICOLON', 'apostrophe': 'KEY_APOSTROPHE',
         'bracket_left': 'KEY_LEFTBRACE', 'bracket_right': 'KEY_RIGHTBRACE', 'grave_accent': 'KEY_GRAVE',
         'meta_l': 'KEY_LEFTMETA', 'ctrl': 'KEY_LEFTCTRL', 'alt': 'KEY_LEFTALT', 'shift': 'KEY_LEFTSHIFT',
         'up': 'KEY_UP', 'down': 'KEY_DOWN', 'left': 'KEY_LEFT', 'right': 'KEY_RIGHT', 'home': 'KEY_HOME', 'end': 'KEY_END',
         'pgup': 'KEY_PAGEUP', 'pgdn': 'KEY_PAGEDOWN', 'delete': 'KEY_DELETE'}
for i in range(1, 13): NAMES[f'f{i}'] = f'KEY_F{i}'
for c in 'abcdefghijklmnopqrstuvwxyz0123456789': NAMES[c] = 'KEY_' + c.upper()
CHARS = {' ': 'spc', '-': 'minus', '=': 'equal', '.': 'dot', ',': 'comma', '/': 'slash', '\\': 'backslash',
         ';': 'semicolon', "'": 'apostrophe', '[': 'bracket_left', ']': 'bracket_right', '\n': 'ret', '\t': 'tab',
         '?': 'shift-slash', '!': 'shift-1', '@': 'shift-2', '#': 'shift-3', '$': 'shift-4', '%': 'shift-5',
         '^': 'shift-6', '&': 'shift-7', '*': 'shift-8', '(': 'shift-9', ')': 'shift-0', '_': 'shift-minus',
         '+': 'shift-equal', ':': 'shift-semicolon', '"': 'shift-apostrophe', '<': 'shift-comma', '>': 'shift-dot',
         '~': 'shift-grave_accent', '`': 'grave_accent', '|': 'shift-backslash', '{': 'shift-bracket_left', '}': 'shift-bracket_right'}

def combo(c):
    return ' '.join(NAMES[p] for p in c.split('-'))

def char_combo(ch):
    if ch.isalpha(): return ('shift-' + ch.lower()) if ch.isupper() else ch
    if ch.isdigit(): return ch
    return CHARS[ch]

def virsh(cmds):
    return subprocess.run(['virsh', '-c', URI, '; '.join(cmds)], capture_output=True, text=True)

def send_keys(combos):
    cmds = [f'send-key {DOM} --codeset linux {combo(c)}' for c in combos]
    r = virsh(cmds)
    if r.returncode: print(r.stderr.strip()[:200])

def type_text(text):
    # small batches: the emulated PS/2 keyboard drops keys that arrive too fast
    combos = [char_combo(ch) for ch in text]
    for i in range(0, len(combos), 6):
        send_keys(combos[i:i + 6])
        time.sleep(0.08)

def ppm_to_png(ppm, png):
    data = open(ppm, 'rb').read()
    if data[:8] == b'\x89PNG\r\n\x1a\n':  # newer QEMU/libvirt already hand back PNG
        open(png, 'wb').write(data)
        return struct.unpack('>II', data[16:24])
    parts = data.split(maxsplit=4)
    w, h = int(parts[1]), int(parts[2]); px = parts[4]
    raw = b''.join(b'\x00' + px[y*w*3:(y+1)*w*3] for y in range(h))
    def chunk(t, d): return struct.pack('>I', len(d)) + t + d + struct.pack('>I', zlib.crc32(t + d) & 0xffffffff)
    open(png, 'wb').write(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 2, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(raw, 6)) + chunk(b'IEND', b''))
    return w, h

SCREEN = tuple(int(v) for v in os.environ.get("VDRIVE_SCREEN", "1920x1080").split("x"))

def qmp(obj):
    import json
    r = subprocess.run(['virsh', '-c', URI, 'qemu-monitor-command', DOM, json.dumps(obj)], capture_output=True, text=True)
    if r.returncode: raise SystemExit('qmp: ' + r.stderr.strip())
    return r.stdout

def pointer(events):
    qmp({'execute': 'input-send-event', 'arguments': {'events': events}})

def abs_xy(x, y):
    return [{'type': 'abs', 'data': {'axis': 'x', 'value': int(int(x) * 32767 / max(SCREEN[0] - 1, 1))}},
            {'type': 'abs', 'data': {'axis': 'y', 'value': int(int(y) * 32767 / max(SCREEN[1] - 1, 1))}}]

def button(name, down):
    return {'type': 'btn', 'data': {'down': down, 'button': name}}

def wheel(direction, times=1):
    """Mouse wheel notches: QEMU sends them as button presses."""
    name = 'wheel-up' if direction in ('up', 'u') else 'wheel-down'
    for _ in range(int(times)):
        pointer([button(name, True)]); time.sleep(0.05); pointer([button(name, False)]); time.sleep(0.12)

# QMP key events, for a key that has to stay down while something else happens
# (Super + the mouse wheel); `keys` only ever taps.
QCODE = {'meta_l': 'meta_l', 'meta': 'meta_l', 'super': 'meta_l', 'ctrl': 'ctrl', 'alt': 'alt', 'shift': 'shift'}

def key_hold(name, down):
    qmp({'execute': 'input-send-event', 'arguments': {'events': [
        {'type': 'key', 'data': {'down': down, 'key': {'type': 'qcode', 'data': QCODE.get(name, name)}}}]}})

def click(x, y, btn='left', times=1):
    pointer(abs_xy(x, y)); time.sleep(0.08)
    for _ in range(times):
        pointer([button(btn, True)]); time.sleep(0.05); pointer([button(btn, False)]); time.sleep(0.05)

def drag(x1, y1, x2, y2, steps=12):
    # The coordinates arrive from argv as strings.
    x1, y1, x2, y2 = int(x1), int(y1), int(x2), int(y2)
    pointer(abs_xy(x1, y1)); time.sleep(0.08); pointer([button('left', True)]); time.sleep(0.12)
    for i in range(1, steps + 1):
        pointer(abs_xy(x1 + (x2 - x1) * i / steps, y1 + (y2 - y1) * i / steps)); time.sleep(0.03)
    time.sleep(0.12); pointer([button('left', False)])

def guest_exec(cmd, timeout=120):
    """Run a shell command in the guest through the QEMU guest agent; returns (rc, stdout, stderr)."""
    import json, base64
    def agent(obj):
        r = subprocess.run(['virsh', '-c', URI, 'qemu-agent-command', DOM, json.dumps(obj)], capture_output=True, text=True)
        if r.returncode: raise SystemExit('agent: ' + r.stderr.strip())
        return json.loads(r.stdout)['return']
    pid = agent({'execute': 'guest-exec', 'arguments': {'path': '/bin/bash', 'arg': ['-lc', cmd], 'capture-output': True}})['pid']
    for _ in range(timeout * 4):
        st = agent({'execute': 'guest-exec-status', 'arguments': {'pid': pid}})
        if st.get('exited'): break
        time.sleep(0.25)
    dec = lambda k: base64.b64decode(st.get(k, '')).decode(errors='replace')
    return st.get('exitcode', -1), dec('out-data'), dec('err-data')

if __name__ == '__main__':
    a = sys.argv[1:]
    if a and a[0] == '--dom': DOM = a[1]; a = a[2:]
    if a[0] == 'shot':
        ppm = f'{S}/vshot.ppm'
        r = subprocess.run(['virsh', '-c', URI, 'screenshot', DOM, ppm], capture_output=True, text=True)
        if r.returncode: print(r.stderr.strip()); sys.exit(1)
        print('shot', a[1], *ppm_to_png(ppm, a[1]))
    elif a[0] == 'keys': send_keys(a[1:])
    elif a[0] == 'type': type_text(a[1])
    elif a[0] == 'state': print(virsh([f'domstate {DOM}']).stdout.strip())
    elif a[0] == 'move': pointer(abs_xy(a[1], a[2]))
    elif a[0] == 'click': click(a[1], a[2], a[3] if len(a) > 3 else 'left')
    elif a[0] == 'rclick': click(a[1], a[2], 'right')
    elif a[0] == 'dblclick': click(a[1], a[2], 'left', 2)
    elif a[0] == 'drag': drag(a[1], a[2], a[3], a[4])
    elif a[0] == 'wheel': wheel(a[1], a[2] if len(a) > 2 else 1)
    elif a[0] == 'keydown': key_hold(a[1], True)
    elif a[0] == 'keyup': key_hold(a[1], False)
    elif a[0] == 'press': pointer([button(a[1] if len(a) > 1 else 'left', True)])
    elif a[0] == 'release': pointer([button(a[1] if len(a) > 1 else 'left', False)])
    elif a[0] == 'exec':
        rc, out, err = guest_exec(a[1]); sys.stdout.write(out); sys.stderr.write(err); sys.exit(rc)
    else: print(__doc__)
