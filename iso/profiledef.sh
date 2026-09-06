#!/usr/bin/env bash
# shellcheck disable=SC2034
# MindOS live/install ISO (archiso profile). Build with `make iso`.

iso_name="mindos"
iso_label="MINDOS_$(date --date="@${SOURCE_DATE_EPOCH:-$(date +%s)}" +%Y%m)"
iso_publisher="MindOS <https://mindos.local>"
iso_application="MindOS Live"
iso_version="$(date --date="@${SOURCE_DATE_EPOCH:-$(date +%s)}" +%Y.%m.%d)"
install_dir="mindos"
buildmodes=('iso')
bootmodes=('bios.syslinux'
           'uefi.grub')
arch="x86_64"
pacman_conf="pacman.conf"
airootfs_image_type="squashfs"
airootfs_image_tool_options=('-comp' 'zstd' '-Xcompression-level' '15' '-b' '1M')
bootstrap_tarball_compression=('zstd' '-c' '-T0' '--auto-threads=logical' '--long' '-19')
file_permissions=(
  ["/etc/shadow"]="0:0:400"
  ["/etc/sudoers.d"]="0:0:750"
  ["/etc/sudoers.d/10-live"]="0:0:440"
  ["/root"]="0:0:750"
  ["/home/mind"]="1000:1000:750"
  ["/etc/xdg/mindos/autostart/10-welcome"]="0:0:755"
)
