#!/bin/bash
# Type text into the running QEMU guest through the HMP monitor (sendkey).
#   scripts/qemu-type.sh 'journalctl -b > /dev/ttyS0'    (a newline is sent at the end)
#   scripts/qemu-type.sh --keys ctrl-alt-f2              (raw sendkey names)
#   scripts/qemu-type.sh --hmp 'mouse_move 16000 16000' 'mouse_button 1' 'mouse_button 0'   (raw HMP)
set -euo pipefail
mon=${QEMU_MON:-/run/user/$(id -u)/mindos-mon.sock}
python3 - "$mon" "$@" <<'PY'
import socket, sys, time
mon = sys.argv[1]; args = sys.argv[2:]
def cmd(c, wait=0.05):
    s = socket.socket(socket.AF_UNIX); s.settimeout(5); s.connect(mon); time.sleep(0.05)
    try: s.recv(65536)
    except Exception: pass
    s.sendall((c + "\n").encode()); time.sleep(wait); s.close()
special = {' ': 'spc', '-': 'minus', '=': 'equal', '[': 'bracket_left', ']': 'bracket_right',
           ';': 'semicolon', "'": 'apostrophe', '`': 'grave_accent', '\\': 'backslash',
           ',': 'comma', '.': 'dot', '/': 'slash', '\n': 'ret', '\t': 'tab'}
shifted = {'!': '1', '@': '2', '#': '3', '$': '4', '%': '5', '^': '6', '&': '7', '*': '8', '(': '9', ')': '0',
           '_': 'minus', '+': 'equal', '{': 'bracket_left', '}': 'bracket_right', ':': 'semicolon',
           '"': 'apostrophe', '~': 'grave_accent', '|': 'backslash', '<': 'comma', '>': 'dot', '?': 'slash'}
if args and args[0] == '--hmp':          # raw monitor commands, e.g. --hmp 'mouse_move 16000 16000' 'mouse_button 1'
    for c in args[1:]:
        cmd(c, 0.2)
    sys.exit(0)
if args and args[0] == '--keys':
    for k in args[1:]:
        cmd('sendkey ' + k, 0.15)
    sys.exit(0)
text = ' '.join(args) + '\n'
for ch in text:
    if ch in special: k = special[ch]
    elif ch in shifted: k = 'shift-' + shifted[ch]
    elif ch.isupper(): k = 'shift-' + ch.lower()
    elif ch.isalnum(): k = ch
    else: continue
    cmd('sendkey ' + k, 0.06)
PY
