#!/bin/bash
# guest-install.sh — run as root on the MindOS live ISO *inside the dev VM*.
#
# Unattended MindOS install onto the VM disk, then the dev-VM extras:
#   * qemu-guest-agent (clean shutdown, host-side command execution)
#   * the host's source tree (virtiofs tag "mindos") mounted at ~/mindos
#   * pacman prefers packages built on the host (build/repo on the share)
#   * CARGO_TARGET_DIR on the VM disk so host and guest builds never collide
#
# Typical use from the VM's root console (Ctrl+Alt+F2 on the live ISO):
#   mkdir -p /run/share && mount -t virtiofs mindos /run/share \
#     && bash /run/share/scripts/vm/guest-install.sh
# Override any MINDOS_* variable below through the environment.
set -euo pipefail
: "${MINDOS_DISK:=/dev/vda}" "${MINDOS_HOSTNAME:=mindos-dev}" "${MINDOS_USER:=morvoso}"
: "${MINDOS_PASSWORD:=mindos}" "${MINDOS_TZ:=America/New_York}"
: "${MINDOS_INSTALL_GAMING:=0}" "${MINDOS_INSTALL_DEV:=1}" "${MINDOS_SHARE_TAG:=mindos}"
export MINDOS_AUTO=1 MINDOS_DISK MINDOS_HOSTNAME MINDOS_USER MINDOS_PASSWORD MINDOS_TZ \
       MINDOS_INSTALL_GAMING MINDOS_INSTALL_DEV
T=/mnt

echo "== network"
for _ in 1 2 3 4 5 6 7 8 9 10; do getent hosts archlinux.org >/dev/null 2>&1 && break; sleep 5; done
getent hosts archlinux.org >/dev/null || { echo "guest-install: no network/DNS in the VM"; exit 1; }
ip -4 -brief address | grep -v '^lo'

echo "== mindos-install ($MINDOS_DISK, user $MINDOS_USER, gaming=$MINDOS_INSTALL_GAMING dev=$MINDOS_INSTALL_DEV)"
mindos-install

echo "== dev VM extras"
arch-chroot "$T" pacman -S --noconfirm --needed qemu-guest-agent
arch-chroot "$T" systemctl enable qemu-guest-agent >/dev/null 2>&1 || true

uid=$(awk -F: -v u="$MINDOS_USER" '$1==u {print $3}' "$T/etc/passwd")
gid=$(awk -F: -v u="$MINDOS_USER" '$1==u {print $4}' "$T/etc/passwd")
home=/home/$MINDOS_USER
install -d -o "$uid" -g "$gid" "$T$home/mindos" "$T$home/build"
grep -q " $home/mindos virtiofs" "$T/etc/fstab" || \
  echo "$MINDOS_SHARE_TAG $home/mindos virtiofs defaults,nofail 0 0" >> "$T/etc/fstab"

# [mindos] repo: packages built on the host (make packages && make repo) win
# over the copy that came with the ISO; falls back to the ISO copy when the
# share is not mounted.
if ! grep -q "$home/mindos/build/repo" "$T/etc/pacman.conf"; then
  sed -i "/^\[mindos\]/,/^\[/{s#^Server = file:///var/lib/mindos/repo#Server = file://$home/mindos/build/repo\n&#}" \
    "$T/etc/pacman.conf"
fi

cat > "$T/etc/profile.d/mindos-dev-vm.sh" <<'PROFILE'
# MindOS dev VM: the host's source tree is shared at ~/mindos. Build
# artifacts stay on the VM disk so host and guest toolchains never collide.
export CARGO_TARGET_DIR="$HOME/build/cargo"
PROFILE

echo "== summary"
echo "  disk      $MINDOS_DISK      hostname $MINDOS_HOSTNAME      user $MINDOS_USER / $MINDOS_PASSWORD"
echo "  share     $MINDOS_SHARE_TAG -> $home/mindos (virtiofs)"
echo "  pacman    [mindos] = $home/mindos/build/repo, then /var/lib/mindos/repo"
echo "  next      poweroff, then start the VM again to boot from disk"
echo GUEST-INSTALL-OK
