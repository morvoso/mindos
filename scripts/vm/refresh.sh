#!/bin/bash
# Update the dev VM from build/repo, restart the graphical session and take a screenshot.
#   scripts/vm/refresh.sh [out.png] [--no-update] [--wait N]
# Prints the compositor and shell journals of the new session at the end.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
out="${1:-/tmp/mindos-vdrive/refresh.png}"; shift || true
update=1; wait=12
while [[ $# -gt 0 ]]; do case "$1" in --no-update) update=0 ;; --wait) wait="$2"; shift ;; esac; shift; done
v() { python3 "$here/vdrive.py" "$@"; }
if [[ $update == 1 ]]; then
  echo "== pacman -Syu (from the shared build/repo)"
  v exec 'pacman -Syu --noconfirm 2>&1 | tail -15'
fi
echo "== restarting the session"
v exec 'rm -f /run/greetd.run; systemctl restart greetd; sleep 1; systemctl is-active greetd'
sleep "$wait"
v shot "$out"
echo "== mindwm journal (last 40 lines)"
v exec 'journalctl -t mindwm -b --no-pager -n 40 -o cat 2>/dev/null | tail -40'
echo "== mindos-shell journal (last 60 lines)"
v exec 'journalctl -b --no-pager -n 60 -o cat _SYSTEMD_USER_UNIT=mindos-shell.service 2>/dev/null; su - morvoso -c "XDG_RUNTIME_DIR=/run/user/1000 systemctl --user status mindos-shell.service --no-pager 2>&1 | head -12"'
