#!/usr/bin/env python3
"""Drive a libvirt VM: vdrive.py [--dom NAME] shot out.png | keys combo... | type "text" | exec "cmd" | state
Key names follow qemu-drive.py (meta_l-spc, ctrl-alt-f2, ret, esc, spc, ...), mapped to linux keycodes."""
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
    elif a[0] == 'exec':
        rc, out, err = guest_exec(a[1]); sys.stdout.write(out); sys.stderr.write(err); sys.exit(rc)
    else: print(__doc__)
