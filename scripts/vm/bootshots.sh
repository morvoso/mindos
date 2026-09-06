#!/bin/bash
# Reboot the dev VM and capture the boot sequence: one screenshot every INTERVAL seconds for DURATION seconds.
#   scripts/vm/bootshots.sh OUTDIR [DURATION=40] [INTERVAL=0.5]
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
out="${1:?output directory}"; dur="${2:-40}"; iv="${3:-0.5}"
mkdir -p "$out"
v() { python3 "$here/vdrive.py" "$@"; }
v exec 'systemctl reboot' >/dev/null 2>&1 || virsh -c qemu:///system reboot mindos-dev
start=$(date +%s.%N); i=0
while :; do
  now=$(date +%s.%N); t=$(python3 -c "print(round($now-$start,1))")
  (( $(python3 -c "print(int($t >= $dur))") )) && break
  v shot "$out/$(printf '%03d' "$i")-t${t}.png" >/dev/null 2>&1
  i=$((i+1)); sleep "$iv"
done
echo "captured $i frames in $out"
