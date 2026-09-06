#!/bin/bash
# Boot a MindOS ISO in QEMU/KVM, headless (the host QEMU has no GUI backends).
# Look at the screen with scripts/qemu-screenshot.sh; the HMP monitor is on a
# Unix socket, the serial console is logged to build/qemu/serial.log.
#
#   scripts/qemu-iso.sh <iso> [--bios] [--fg] [--disk-boot]
#   scripts/qemu-iso.sh --stop
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
qemu=${QEMU:-$(command -v qemu-system-x86_64)}
mon=${QEMU_MON:-/run/user/$(id -u)/mindos-mon.sock}
qdir="$here/build/qemu"
mkdir -p "$qdir"

if [[ ${1:-} == --stop ]]; then
  if [[ -S $mon ]]; then
    python3 - "$mon" <<'PY' || true
import socket,sys,time
s=socket.socket(socket.AF_UNIX); s.settimeout(3); s.connect(sys.argv[1]); time.sleep(0.2)
try: s.recv(65536)
except Exception: pass
s.sendall(b"quit\n"); time.sleep(0.3); s.close()
PY
  fi
  [[ -f $qdir/qemu.pid ]] && kill "$(cat "$qdir/qemu.pid")" 2>/dev/null || true
  rm -f "$qdir/qemu.pid" "$mon"
  exit 0
fi

iso=${1:?usage: qemu-iso.sh <iso> [--bios] [--fg] [--disk-boot] | --stop}; shift
[[ -f $iso ]] || { echo "no such ISO: $iso" >&2; exit 1; }
uefi=1 fg=0 boot=d
for a in "$@"; do
  case $a in
    --bios) uefi=0 ;;
    --fg) fg=1 ;;
    --disk-boot) boot=c ;;
    *) echo "unknown option $a" >&2; exit 2 ;;
  esac
done

disk="$qdir/disk.qcow2"
[[ -f $disk ]] || qemu-img create -f qcow2 "$disk" 64G >/dev/null
rm -f "$mon"
: > "$qdir/serial.log"

args=(-machine q35 -m "${QEMU_MEM:-8G}" -smp "${QEMU_CPUS:-8}"
      -device virtio-vga -display none
      -device virtio-keyboard-pci -device virtio-tablet-pci
      -drive "file=$disk,if=virtio,format=qcow2"
      -cdrom "$iso" -boot "$boot"
      -nic none
      -monitor "unix:$mon,server,nowait"
      -serial "file:$qdir/serial.log")
if [[ -e /dev/kvm ]]; then args+=(-enable-kvm -cpu host); fi
if (( uefi )); then
  fw=""
  for c in "$(dirname "$qemu")/../share/qemu/edk2-x86_64-code.fd" \
           /usr/share/edk2/x64/OVMF_CODE.4m.fd /usr/share/edk2-ovmf/x64/OVMF_CODE.4m.fd \
           /usr/share/OVMF/x64/OVMF_CODE.4m.fd /usr/share/OVMF/OVMF_CODE.fd /usr/share/qemu/edk2-x86_64-code.fd; do
    [[ -f $c ]] && { fw=$(realpath "$c"); break; }
  done
  if [[ -n $fw ]]; then
    vars="$qdir/efivars.fd"
    if [[ ! -f $vars ]]; then
      src=""
      for c in "$(dirname "$fw")/edk2-i386-vars.fd" "$(dirname "$fw")/OVMF_VARS.4m.fd" "$(dirname "$fw")/OVMF_VARS.fd"; do
        [[ -f $c ]] && { src=$c; break; }
      done
      [[ -n $src ]] && cp "$src" "$vars" || truncate -s 540672 "$vars"
    fi
    args+=(-drive "if=pflash,format=raw,readonly=on,file=$fw" -drive "if=pflash,format=raw,file=$vars")
  else
    echo "no UEFI firmware found, booting with SeaBIOS" >&2
  fi
fi

# Extra raw QEMU arguments, e.g. QEMU_EXTRA="-drive file=build/qemu/xfer.img,if=virtio,format=raw"
if [[ -n ${QEMU_EXTRA:-} ]]; then read -r -a extra <<<"$QEMU_EXTRA"; args+=("${extra[@]}"); fi
if (( fg )); then exec "$qemu" "${args[@]}"; fi
"$qemu" "${args[@]}" > "$qdir/qemu.log" 2>&1 &
echo $! > "$qdir/qemu.pid"
sleep 1
if ! kill -0 "$(cat "$qdir/qemu.pid")" 2>/dev/null; then
  echo "qemu exited:"; cat "$qdir/qemu.log"; exit 1
fi
echo "qemu pid $(cat "$qdir/qemu.pid")  monitor $mon  serial $qdir/serial.log  firmware ${fw:-seabios}"
