#!/bin/bash
# mindos-vm.sh — a persistent MindOS development VM on libvirt/virt-manager.
#
#   scripts/vm/mindos-vm.sh create [iso]      define + start the VM booting the live ISO
#   scripts/vm/mindos-vm.sh install           run the unattended install inside the live VM
#   scripts/vm/mindos-vm.sh start|stop|console|shot out.png|state
#   scripts/vm/mindos-vm.sh gl on|off        virgl 3D for the guest GPU (breaks shot)
#   scripts/vm/mindos-vm.sh snapshot NAME [description]     libvirt snapshot (also in virt-manager)
#   scripts/vm/mindos-vm.sh revert NAME
#   scripts/vm/mindos-vm.sh snapshots
#   scripts/vm/mindos-vm.sh destroy           delete the VM and its disk (asks first)
#
# Environment: VM_NAME (mindos-dev) VM_MEM_MB (16384) VM_VCPUS (8) VM_DISK_GB (80)
#              VM_SHARE (repo root, exported to the guest as virtiofs tag "mindos")
#              VM_IMAGES (/var/lib/libvirt/images) VM_RENDERNODE (/dev/dri/renderD129)
#              VM_FIRMWARE (uefi|bios) VM_CPU (host-passthrough, or e.g. Nehalem)
# The guest side of the install lives in guest-install.sh; typing into the VM goes
# through vdrive.py (virsh send-key / screenshot), no guest agent needed.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
VM_NAME=${VM_NAME:-mindos-dev}
VM_MEM_MB=${VM_MEM_MB:-16384}
VM_VCPUS=${VM_VCPUS:-8}
VM_DISK_GB=${VM_DISK_GB:-80}
VM_SHARE=${VM_SHARE:-$repo}
VM_IMAGES=${VM_IMAGES:-/var/lib/libvirt/images}
VM_FIRMWARE=${VM_FIRMWARE:-uefi}
VM_CPU=${VM_CPU:-host-passthrough}
# The host GPU virglrenderer draws on. The iGPU is the safe default: QEMU runs as
# the "qemu" user, and libvirt's device ACL keeps it out of /dev/nvidia*, so EGL
# will not initialise on the NVIDIA render node.
VM_RENDERNODE=${VM_RENDERNODE:-/dev/dri/renderD129}
URI=qemu:///system
export VDOM=$VM_NAME
v() { virsh -c $URI "$@"; }
die() { echo "mindos-vm: $*" >&2; exit 1; }
need() { command -v "$1" >/dev/null || die "$1 missing (pacman -S $2)"; }

cmd=${1:-}; shift || true
case $cmd in
create)
  need virt-install virt-install; need virsh libvirt
  case $VM_FIRMWARE in
    uefi) firmware=(--boot uefi,firmware.feature0.name=secure-boot,firmware.feature0.enabled=no) ;;
    bios) firmware=(--boot cdrom,hd) ;;
    *) die "VM_FIRMWARE must be uefi or bios" ;;
  esac
  iso=${1:-$(ls -t "$repo"/build/out/mindos-*.iso 2>/dev/null | head -1)}
  [[ -f $iso ]] || die "no ISO given and none in build/out (make iso)"
  v dominfo "$VM_NAME" >/dev/null 2>&1 && die "$VM_NAME already exists (destroy first)"
  staged=$VM_IMAGES/$(basename "$iso")
  # the qemu user cannot read a home directory; stage the ISO next to the disk
  sudo mkdir -p "$VM_IMAGES"; sudo cp --reflink=auto "$iso" "$staged"
  sudo touch "$VM_IMAGES/$VM_NAME-serial.log"
  virt-install --connect $URI --name "$VM_NAME" --memory "$VM_MEM_MB" --vcpus "$VM_VCPUS" \
    --cpu "$VM_CPU" --osinfo archlinux \
    "${firmware[@]}" \
    --disk "path=$VM_IMAGES/$VM_NAME.qcow2,size=$VM_DISK_GB,format=qcow2,bus=virtio,discard=unmap" \
    --cdrom "$staged" --network network=default,model=virtio \
    --graphics spice,listen=none --video virtio \
    --channel spicevmc --channel unix,target.type=virtio,target.name=org.qemu.guest_agent.0 \
    --memorybacking source.type=memfd,access.mode=shared \
    --filesystem "source=$VM_SHARE,target=mindos,driver.type=virtiofs,binary.path=/usr/lib/virtiofsd" \
    --serial "file,path=$VM_IMAGES/$VM_NAME-serial.log" \
    --rng /dev/urandom --sound none --tpm none --noautoconsole
  echo "$VM_NAME is booting the live ISO. Next: scripts/vm/mindos-vm.sh install"
  ;;
install)
  # drive the live ISO's root console (tty2): mount the share, run guest-install.sh
  log=$VM_IMAGES/$VM_NAME-serial.log
  [[ $(v domstate "$VM_NAME") == running ]] || die "$VM_NAME is not running"
  "$here/vdrive.py" keys ctrl-alt-f2; sleep 3
  "$here/vdrive.py" type 'clear; mkdir -p /run/share && mount -t virtiofs mindos /run/share && bash /run/share/scripts/vm/guest-install.sh > /dev/ttyS0 2>&1; echo GUEST-EXIT=$? > /dev/ttyS0'
  "$here/vdrive.py" keys ret
  echo "installing; following $log (Ctrl+C stops following, not the install)"
  sudo tail -n +1 -f "$log" | tr '\r' '\n' | sed -u -n '/== network/,$p' | while IFS= read -r line; do
    printf '%s\n' "$line"
    [[ $line == GUEST-EXIT=* ]] && pkill -P $$ tail && break
  done || true
  echo "when it printed GUEST-INSTALL-OK: scripts/vm/mindos-vm.sh stop && scripts/vm/mindos-vm.sh start"
  ;;
start)   v start "$VM_NAME" ;;
stop)    v shutdown "$VM_NAME" ;;
kill)    v destroy "$VM_NAME" ;;
state)   v domstate "$VM_NAME" ;;
console) virt-manager --connect $URI --show-domain-console "$VM_NAME" & ;;
shot)    "$here/vdrive.py" shot "${1:?out.png}" ;;
gl)
  # virtio-vga-gl plus an egl-headless display: the guest renders through virgl on
  # the host GPU instead of llvmpipe, and QEMU reads the result back so the SPICE
  # console keeps working. The cost is that QMP screendump then has no surface, so
  # `shot` (vdrive.py, refresh.sh, bootshots.sh) needs GL off.
  want=${1:?on|off}
  [[ $want == on || $want == off ]] || die "gl takes on or off"
  running=no
  [[ $(v domstate "$VM_NAME") == running ]] && running=yes
  if [[ $running == yes ]]; then
    v shutdown "$VM_NAME"
    for _ in $(seq 60); do [[ $(v domstate "$VM_NAME") == "shut off" ]] && break; sleep 2; done
  fi
  [[ $(v domstate "$VM_NAME") == "shut off" ]] || die "$VM_NAME did not shut down"
  tmp=$(mktemp)
  v dumpxml "$VM_NAME" --inactive > "$tmp"
  WANT=$want NODE=$VM_RENDERNODE python3 "$here/vm-gl.py" "$tmp"
  v define "$tmp" >/dev/null
  rm -f "$tmp"
  echo "3D acceleration $want ($VM_RENDERNODE)"
  if [[ $running == yes ]]; then v start "$VM_NAME"; fi
  ;;
snapshot)
  name=${1:?snapshot name}; shift
  v snapshot-create-as "$VM_NAME" "$name" ${1:+--description "$*"}
  ;;
revert)   v snapshot-revert "$VM_NAME" "${1:?snapshot name}" --running ;;
snapshots) v snapshot-list "$VM_NAME" --tree ;;
destroy)
  read -rp "delete VM $VM_NAME and $VM_IMAGES/$VM_NAME.qcow2? [y/N] " ok
  [[ $ok =~ ^[Yy] ]] || exit 1
  v destroy "$VM_NAME" 2>/dev/null || true
  v undefine "$VM_NAME" --nvram --snapshots-metadata --remove-all-storage
  ;;
*) sed -n '2,17p' "$0"; exit 1 ;;
esac
