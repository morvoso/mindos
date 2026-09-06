#!/usr/bin/env bash
# Build a bootable MBR disk image: partition 1 = FAT32 (boot: limine + kernel + initrd),
# partition 2 = ext2 (system root, populated from build/rootfs).
set -euo pipefail
DISK="$1"; KERNEL="$2"; INITRD="$3"
BUILD="$(dirname "$DISK")"
mkdir -p "$BUILD"
BOOT_MB=${BOOT_MB:-64}
INITRD_MB=$(( ( $(stat -c %s "$INITRD") + 1048575 ) / 1048576 ))
BOOT_MB=$(( BOOT_MB + INITRD_MB ))
ROOT_MB=${ROOT_MB:-256}
TOTAL_MB=$(( BOOT_MB + ROOT_MB + 2 ))

rm -f "$DISK"
truncate -s "${TOTAL_MB}M" "$DISK"
# MBR partition table: p1 FAT32 (type 0c, bootable), p2 Linux (83)
sfdisk -q "$DISK" <<SF
label: dos
unit: sectors
start=2048, size=$(( BOOT_MB * 2048 )), type=0c, bootable
start=$(( 2048 + BOOT_MB * 2048 )), size=$(( ROOT_MB * 2048 )), type=83
SF

P1_OFF=$(( 2048 * 512 ))
P2_OFF=$(( (2048 + BOOT_MB * 2048) * 512 ))

# --- partition 1: FAT32 with limine + kernel + initrd
mformat -i "$DISK@@$P1_OFF" -F -v MINDBOOT ::
mmd -i "$DISK@@$P1_OFF" ::/boot ::/boot/limine
mcopy -i "$DISK@@$P1_OFF" boot/limine.conf ::/boot/limine/limine.conf
mcopy -i "$DISK@@$P1_OFF" /usr/share/limine/limine-bios.sys ::/boot/limine/limine-bios.sys
mcopy -i "$DISK@@$P1_OFF" "$KERNEL" ::/boot/mindos-kernel
mcopy -i "$DISK@@$P1_OFF" "$INITRD" ::/boot/initrd.img
# UEFI fallback path (for machines booting the image via UEFI)
mmd -i "$DISK@@$P1_OFF" ::/EFI ::/EFI/BOOT
mcopy -i "$DISK@@$P1_OFF" /usr/share/limine/BOOTX64.EFI ::/EFI/BOOT/BOOTX64.EFI
mcopy -i "$DISK@@$P1_OFF" boot/limine.conf ::/EFI/BOOT/limine.conf

# --- partition 2: ext2 system root
ROOTFS="$BUILD/rootfs"
mkdir -p "$ROOTFS"
ROOTIMG="$BUILD/root.ext2"
rm -f "$ROOTIMG"
mke2fs -q -t ext2 -L mindos-root -d "$ROOTFS" "$ROOTIMG" "${ROOT_MB}M"
dd if="$ROOTIMG" of="$DISK" bs=1M seek=$(( P2_OFF / 1048576 )) conv=notrunc status=none

# --- BIOS boot code
limine bios-install "$DISK" >/dev/null 2>&1

# --- a scratch data disk (persistent state / update staging), created once
if [ ! -f "$BUILD/data.img" ]; then
    truncate -s 256M "$BUILD/data.img"
    mke2fs -q -t ext2 -L mindos-data "$BUILD/data.img"
fi
echo "disk image: $DISK ($TOTAL_MB MiB; boot=${BOOT_MB}M root=${ROOT_MB}M)"
