#!/bin/bash
# Screenshot the running QEMU guest (HMP monitor screendump -> PNG).
#   scripts/qemu-screenshot.sh out.png [key ...]   e.g. ctrl-alt-f2, ret, "s u d o spc ..."
set -euo pipefail
out=${1:?usage: qemu-screenshot.sh out.png [sendkey ...]}; shift || true
mon=${QEMU_MON:-/run/user/$(id -u)/mindos-mon.sock}
out=$(realpath -m "$out")
python3 - "$mon" "$out" "$@" <<'PY'
import os, socket, sys, time
mon, out, keys = sys.argv[1], sys.argv[2], sys.argv[3:]
def cmd(c, wait=0.6):
    s = socket.socket(socket.AF_UNIX); s.settimeout(5); s.connect(mon); time.sleep(0.2)
    try: s.recv(65536)
    except Exception: pass
    s.sendall((c + "\n").encode()); time.sleep(wait)
    try: r = s.recv(65536).decode(errors="replace")
    except Exception: r = ""
    s.close(); return r
for k in keys:
    cmd("sendkey " + k, 0.15)
if keys: time.sleep(1.5)
ppm = os.path.splitext(out)[0] + ".ppm"
cmd(f"screendump {ppm}", 1.0)
for _ in range(20):
    if os.path.exists(ppm) and os.path.getsize(ppm) > 100: break
    time.sleep(0.2)
from PIL import Image
im = Image.open(ppm); im.save(out); os.unlink(ppm)
print(f"{out} {im.size[0]}x{im.size[1]}")
PY
