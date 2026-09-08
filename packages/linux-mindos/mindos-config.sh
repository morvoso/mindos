#!/bin/bash
# Turn the base config (a full Arch/CachyOS-style desktop config) into the
# MindOS gaming kernel config. Run from the kernel source tree after copying
# the base config to .config; `make olddefconfig` follows.
#
#   MINDOS_CPU=generic  portable x86-64 build (default)
#   MINDOS_CPU=native  -march=native for a build used only on this CPU
#   MINDOS_LTO=thin|none  Clang ThinLTO (default thin)
set -e
cfg=scripts/config
: "${MINDOS_CPU:=generic}"
: "${MINDOS_LTO:=thin}"

# --- identity ------------------------------------------------------------
$cfg --set-str LOCALVERSION "" -d LOCALVERSION_AUTO
$cfg --set-str DEFAULT_HOSTNAME "mindos"
$cfg -e IKCONFIG -e IKCONFIG_PROC

# --- CPU, scheduler, latency ------------------------------------------------
case "$MINDOS_CPU" in
  native)  $cfg -e X86_NATIVE_CPU ;;
  generic) $cfg -d X86_NATIVE_CPU ;;
  *) echo "MINDOS_CPU must be generic or native" >&2; exit 1 ;;
esac
$cfg -e SCHED_BORE                      # Burst-Oriented Response Enhancer
$cfg -e SCHED_CLASS_EXT                 # sched_ext: scx_lavd / scx_bpfland for games
$cfg -d HZ_250 -d HZ_300 -e HZ_1000 --set-val HZ 1000
$cfg -e PREEMPT -d PREEMPT_LAZY -d PREEMPT_VOLUNTARY -d PREEMPT_NONE -e PREEMPT_DYNAMIC
$cfg -d HZ_PERIODIC -d NO_HZ_FULL -e NO_HZ_IDLE -e NO_HZ -e NO_HZ_COMMON
$cfg -e RCU_LAZY -e RCU_NOCB_CPU
$cfg -d CPU_FREQ_DEFAULT_GOV_SCHEDUTIL -d CPU_FREQ_DEFAULT_GOV_POWERSAVE -e CPU_FREQ_DEFAULT_GOV_PERFORMANCE
$cfg -e X86_AMD_PSTATE -e X86_INTEL_PSTATE
$cfg -e NTSYNC                          # Wine/Proton NT synchronisation primitives
$cfg -e FUTEX -e FUTEX_PI

# --- memory -----------------------------------------------------------------
$cfg -d TRANSPARENT_HUGEPAGE_MADVISE -e TRANSPARENT_HUGEPAGE_ALWAYS
$cfg -e ZRAM -e ZRAM_DEF_COMP_ZSTD -d ZSWAP_DEFAULT_ON
$cfg -e LRU_GEN -e LRU_GEN_ENABLED
$cfg -e USERFAULTFD -e ANON_VMA_NAME

# --- network ----------------------------------------------------------------
$cfg -e TCP_CONG_BBR -d DEFAULT_CUBIC -e DEFAULT_BBR --set-str DEFAULT_TCP_CONG bbr
$cfg -e NET_SCH_FQ -d DEFAULT_FQ_CODEL -e DEFAULT_FQ --set-str DEFAULT_NET_SCH fq

# --- toolchain --------------------------------------------------------------
case "$MINDOS_LTO" in
  thin) $cfg -d LTO_NONE -d LTO_CLANG_FULL -e LTO_CLANG_THIN ;;
  none) $cfg -e LTO_NONE -d LTO_CLANG_FULL -d LTO_CLANG_THIN ;;
  *) echo "MINDOS_LTO must be thin or none" >&2; exit 1 ;;
esac
# Lockdown selects signature verification. Sign in-tree modules with the
# build's embedded key; modules_install strips before signing/compressing.
# External DKMS modules remain loadable without enrolled Secure Boot keys.
$cfg -e MODULE_SIG -e MODULE_SIG_ALL -d MODULE_SIG_FORCE
$cfg -e DEBUG_INFO -e DEBUG_INFO_DWARF5 -e DEBUG_INFO_BTF -d DEBUG_INFO_REDUCED -d DEBUG_INFO_COMPRESSED_NONE
$cfg -e MODULE_COMPRESS_ZSTD
$cfg -d RUST

# --- console theme: white on MindOS red is patched into vt.c ----------------
$cfg -e VT -e VT_CONSOLE -e FRAMEBUFFER_CONSOLE -e FRAMEBUFFER_CONSOLE_DEFERRED_TAKEOVER
$cfg -e DRM_SIMPLEDRM -e SYSFB_SIMPLEFB -e DRM_FBDEV_EMULATION
$cfg --set-str DRM_PANIC_SCREEN kmsg

# --- virtualisation: KVM host (dev) and QEMU guest (test harness) -----------
$cfg -m KVM -m KVM_AMD -m KVM_INTEL -e KVM_GUEST -e PARAVIRT
$cfg -e VIRTIO_PCI -e VIRTIO_BLK -m VIRTIO_NET -m DRM_VIRTIO_GPU -m VIRTIO_INPUT -m VIRTIO_CONSOLE -e VIRTIO_MENU -m VIRTIO_BALLOON -m HW_RANDOM_VIRTIO -m VIRTIO_FS -m 9P_FS -m NET_9P -m NET_9P_VIRTIO -m VSOCKETS -m VIRTIO_VSOCKETS
$cfg -m VIRTIO_VDPA -m VHOST_NET -m VHOST_VSOCK

# --- omit unrelated server/legacy subsystems --------------------------------
$cfg -d XEN -d HYPERV -d VMWARE_VMCI -d VMWARE_BALLOON -d VBOXGUEST
$cfg -d INFINIBAND -d STAGING -d MTD -d COMEDI -d GREYBUS
$cfg -d MEDIA_DIGITAL_TV_SUPPORT -d MEDIA_ANALOG_TV_SUPPORT -d MEDIA_RADIO_SUPPORT -d MEDIA_SDR_SUPPORT -d MEDIA_PLATFORM_SUPPORT -d MEDIA_TEST_SUPPORT -d DVB_CORE
$cfg -d SLIMBUS
$cfg -d CAN -d NFC -d ATM -d FDDI -d HIPPI -d WAN
$cfg -d PARPORT -d PCMCIA -d FIREWIRE -d MEMSTICK -d W1 -d FPGA -d SIOX -d MOST -d RAPIDIO -d ISDN
$cfg -d USB_GADGET -d USB_OTG
$cfg -d DRM_PANEL_BRIDGE -d DRM_LOONGSON -d DRM_ETNAVIV -d DRM_HISI_HIBMC -d DRM_ARCPGU

# Preserve the base config's laptop/handheld audio, touch, gyro, regulator,
# connector and Wi-Fi drivers, along with PCI capture and accessibility.
# These device modules load on demand; deleting them does not improve FPS
# on a desktop but prevents the same image working on other gaming hardware.

# --- things a gaming desktop does load ---------------------------------------
$cfg -m DRM_AMDGPU -m DRM_I915 -m DRM_XE -m DRM_NOUVEAU -e DRM_AMDGPU_USERPTR -e DRM_AMD_DC
$cfg -e HID -m HID_STEAM -m HID_NINTENDO -m HID_PLAYSTATION -m HID_SONY -m HID_LOGITECH_DJ -m HID_LOGITECH_HIDPP -m HID_XPADNEO -m JOYSTICK_XPAD -m HID_WIIMOTE -m HID_CORSAIR -m HID_RAZER -m HID_ROCCAT -m HID_STEELSERIES
$cfg -m INPUT_JOYDEV -m INPUT_EVDEV -e INPUT_FF_MEMLESS -m INPUT_UINPUT
$cfg -m SND_HDA_INTEL -m SND_USB_AUDIO -e SND_HDA_PREALLOC_SIZE
$cfg -m BT -m BT_HCIBTUSB -e BT_HCIBTUSB_AUTOSUSPEND -m BT_HIDP
$cfg -e BLK_DEV_NVME -e NVME_MULTIPATH -e SATA_AHCI
$cfg -e EXT4_FS -m BTRFS_FS -m XFS_FS -m F2FS_FS -m NTFS3_FS -m EXFAT_FS -m VFAT_FS -e FUSE_FS -m OVERLAY_FS -m SQUASHFS -e SQUASHFS_ZSTD -m EROFS_FS
$cfg -e CGROUPS -e BPF_SYSCALL -e USER_NS -e BINFMT_MISC
$cfg -e IO_URING
