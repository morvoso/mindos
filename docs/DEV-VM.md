# The MindOS development VM

A persistent MindOS install on libvirt, managed from virt-manager, for
developing the system itself without losing anything between reboots or
experiments.

* The **source tree stays on the host** (this repository, under git). The VM
  sees it read-write at `~/mindos` through virtiofs, so nothing you edit lives
  only inside the VM.
* The **VM disk persists**: packages you install and files you change survive
  reboots, unlike the live ISO.
* **Snapshots** (virt-manager: View > Snapshots, or `make vm-snapshot`) freeze
  the whole VM before a risky change; reverting only touches the VM disk,
  never the shared source tree.
* pacman inside the VM prefers packages you build on the host
  (`build/repo` on the share), so `make packages && make repo` on the host
  followed by `sudo pacman -Syu` in the VM is the update path.
* The QEMU guest agent is installed, so the host can run commands in the
  guest without typing into the console (`scripts/vm/vdrive.py exec`).

## Host setup (CachyOS / Arch, once)

```sh
sudo pacman -S --needed qemu-desktop libvirt virt-manager virt-install virt-viewer \
                        edk2-ovmf virtiofsd swtpm dnsmasq
sudo systemctl enable --now libvirtd
sudo usermod -aG libvirt "$USER"
sudo virsh net-autostart default && sudo virsh net-start default
```

If Docker or ufw is on the host, libvirt's default nftables firewall backend
cannot get guest traffic past their `DROP` policies (no DHCP, no DNS, no
internet in the VM). Switch libvirt to its iptables backend, whose rules are
inserted ahead of theirs:

```sh
sudo sed -i 's/^#firewall_backend = "nftables"/firewall_backend = "iptables"/' /etc/libvirt/network.conf
sudo virsh net-destroy default; sudo systemctl restart libvirtd; sudo virsh net-start default
```

## Create and install

```sh
make iso                       # or use an existing build/out/*.iso
make vm                        # define + start "mindos-dev" booting the newest ISO
make vm-install                # unattended install, follows the serial log
scripts/vm/mindos-vm.sh stop && scripts/vm/mindos-vm.sh start   # first boot from disk
make vm-snapshot NAME=fresh-install
make vm-console                # virt-manager window on the VM
```

`make vm` runs `scripts/vm/mindos-vm.sh create`: UEFI (OVMF, no Secure
Boot), 8 vCPUs, 16 GiB, an 80 GiB qcow2 in `/var/lib/libvirt/images`, virtio
disk/net/video, SPICE graphics, the repository shared as virtiofs tag
`mindos`, a serial console logged to `/var/lib/libvirt/images/mindos-dev-serial.log`.
`VM_NAME`, `VM_MEM_MB`, `VM_VCPUS`, `VM_DISK_GB`, `VM_SHARE` override the
defaults.

For independent release checks, use a separate domain and disk:

```sh
VM_NAME=mindos-qa-bios VM_FIRMWARE=bios VM_CPU=Nehalem VM_MEM_MB=8192 \
  scripts/vm/mindos-vm.sh create build/out/<new-image>.iso
```

`VM_FIRMWARE` defaults to `uefi`; `VM_CPU` defaults to `host-passthrough`.
Nehalem provides an x86-64 CPU without AVX2 for testing portable builds.
The script refuses to replace an existing domain. Use the same `VM_NAME`
for subsequent commands; `vdrive.py --dom mindos-qa-bios …` targets it directly.

`make vm-install` types one command into the live ISO's root console
(Ctrl+Alt+F2): mount the share and run `scripts/vm/guest-install.sh`. That
script runs `mindos-install` unattended and then adds the dev-VM extras:

| Setting | Default | Override |
|---|---|---|
| target disk | `/dev/vda` | `MINDOS_DISK` |
| hostname | `mindos-dev` | `MINDOS_HOSTNAME` |
| user / password | `morvoso` / `mindos` | `MINDOS_USER`, `MINDOS_PASSWORD` |
| timezone | `America/New_York` | `MINDOS_TZ` |
| graphics policy | automatic | `MINDOS_GPU_DRIVER=mesa` for explicit Nouveau compatibility |
| gaming stack | off (no GPU in the VM) | `MINDOS_INSTALL_GAMING=1` |
| dev stack (mindos-dev) | on | `MINDOS_INSTALL_DEV=0` |

Extras: `qemu-guest-agent`, the fstab entry `mindos /home/<user>/mindos
virtiofs`, `[mindos]` in `/etc/pacman.conf` pointing at
`~/mindos/build/repo` first and the ISO's copy second, and
`CARGO_TARGET_DIR=~/build/cargo` in `/etc/profile.d` so guest builds never
collide with host builds in the shared tree.

The install takes a few minutes (pacstrap pulls the Arch packages from a
mirror; the MindOS packages come from the ISO). The installed system boots
straight into the compositor as the configured user, with mindd running.

## Everyday commands

```sh
scripts/vm/mindos-vm.sh start | stop | kill | state | console
scripts/vm/mindos-vm.sh snapshot NAME [description] | revert NAME | snapshots
scripts/vm/mindos-vm.sh shot out.png            # screenshot of the VM display
scripts/vm/mindos-vm.sh gl on | off             # virgl 3D for the guest GPU
scripts/vm/vdrive.py exec 'systemctl status mindd'   # run in the guest (guest agent)
scripts/vm/vdrive.py keys meta_l-ret            # send a key combo
scripts/vm/vdrive.py type 'echo hello'          # type text (throttled for the PS/2 keyboard)
```

virt-manager does the same from its GUI: the console tab is the VM's display,
View > Snapshots takes and reverts snapshots, and the toolbar sends
Ctrl+Alt+F2 for the root console. Internal snapshots work with the UEFI
firmware on libvirt 12.7.

## Display and refresh rate

Three things cap how smooth the VM looks, and they are independent:

1. **The guest's mode.** QEMU's virtio-gpu writes an EDID that advertises a
   single mode, 1920x1080@75, and the compositor paces itself to it. The
   installer adds `video=Virtual-1:1920x1080@120` to the guest's kernel command
   line, which makes the kernel synthesise a 120 Hz mode, and seeds
   `~/.local/state/mindos/mindwm.json` with `"1920x1080@120040"` so mindwm picks
   it. Settings > Displays lists both; `journalctl -t mindwm -b | grep vrefresh`
   says which one is live.
2. **How the guest renders.** Without 3D acceleration mindwm falls back to
   llvmpipe and every pixel is composited on the guest's CPUs.
   `scripts/vm/mindos-vm.sh gl on` switches the video device to `virtio-vga-gl`
   and adds an `egl-headless` display, so the guest gets a real GL renderer
   through virgl on the host GPU (`GL Renderer: "virgl (...radeonsi...)"` in the
   mindwm log). `VM_RENDERNODE` picks the host GPU; it defaults to the iGPU
   because QEMU runs as the `qemu` user and libvirt's device ACL keeps it out of
   `/dev/nvidia*`, where EGL then fails to initialise.
3. **The viewer.** virt-manager's SPICE console redraws from a read-back of the
   guest's framebuffer, so it is the slowest link in the chain and never shows
   the full guest frame rate.

**Screenshots with GL enabled.** QEMU's `screendump` has no CPU surface for
a GL scanout, so the host `shot`/`bootshots.sh` commands can fail with
`screendump: no surface`. Use `grim` inside the unlocked guest instead; the
compositor's screencopy protocol works with virgl and saves to the shared tree:

```sh
python3 scripts/vm/vdrive.py --dom mindos-qa-bios exec 'runuser -u qatest -- env XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-1 grim /home/qatest/mindos/build/shots/qa-desktop.png'
```

Use the guest's actual user, runtime directory and display socket. For early
boot and login-console screenshots, GL-off remains necessary. Switching GL
restarts the VM. One accelerated QA run triggered a host AMD GPU reset and
QEMU abort; see `docs/VALIDATION.md`. Virgl results are not bare-metal gaming
benchmarks.

## Development loops

**Compositor, daemon, CLI (Rust).** Edit on the host or in the VM, they are
the same files. Build in the VM:

```sh
cd ~/mindos/mindwm && cargo build --release          # lands in ~/build/cargo/release
sudo install ~/build/cargo/release/mindwm /usr/bin/mindwm.new \
  && sudo mv -f /usr/bin/mindwm.new /usr/bin/mindwm
sudo rm -f /run/greetd.run && sudo systemctl restart greetd   # new session with the new binary
sudo systemctl restart greetd                                 # ... or the login screen (mindos-greeter)
```

The dev VM installs with autologin (`MINDOS_AUTOLOGIN=1` in
`guest-install.sh`), so the login screen only appears after a logout or a
`systemctl restart greetd` without removing `/run/greetd.run`. Its compositor
logs as `journalctl -t mindos-greeter`, the page as `journalctl -t mindshell`
(both under the `greeter` user).

(`rustup default stable` once in a fresh VM; the mindos-dev package ships
rustup, clang, lld and the rest of the toolchain.)

**Packages.** Build on the host in the build box, publish to the shared repo,
update the VM:

```sh
make packages && make repo          # host
sudo pacman -Syu                    # VM: picks up build/repo from the share
```

**Kernel.** `make kernel && make repo` on the host, then in the VM
`sudo pacman -Syu linux-mindos` and reboot. snap-pac takes a snapshot of the
system before and after, and the boot menu lists it under "Snapshots": pick
the "before" one if the new kernel does not boot, then `sudo mindos-boot
restore` (`docs/ROLLBACK.md`). A libvirt snapshot (`make vm-snapshot
NAME=before-kernel`, `scripts/vm/mindos-vm.sh revert before-kernel`) is the
belt to those braces.

**Boot menu.** `guest-install.sh` puts `video=Virtual-1:1920x1080@120` into
`CMDLINE_EXTRA` in `/etc/mindos/boot.conf`; after editing that file run
`sudo mindos-boot config`.

**Installer and ISO.** `make iso`, then either `scripts/vm/mindos-vm.sh
destroy` and `make vm` again for a from-scratch install, or attach the new
ISO in virt-manager and boot from it.

## What the VM cannot do

* No real GPU: with `gl off` the compositor uses Mesa's software renderer, and
  with `gl on` it uses virgl on the host GPU. Either way it is fine for window
  management, the Mind bar and Wayland/XWayland clients, not for measuring
  frame rates or NVIDIA driver work.
* The gaming stack is skipped by default for the same reason.
* Keys typed by scripts go through an emulated PS/2 keyboard that drops
  keys sent too fast; `vdrive.py type` throttles accordingly.

## Suspend testing in a disposable VM

The normal development VM keeps its virtiofs workspace and is not a suspend
fixture. For visual sleep testing, use an installed `mindos-qa-*` guest with
its guest agent, the `qatest` account (test password `mindos`) and an unlocked
desktop. Save its inactive XML, then shut it down before applying a test XML:

```sh
virsh -c qemu:///system dumpxml --inactive mindos-qa-bios > build/qa-original.xml
python3 scripts/tests/prepare_suspend_vm.py build/qa-original.xml build/qa-sleep.xml
# Once the disposable guest is shut down:
virsh -c qemu:///system define build/qa-sleep.xml
virsh -c qemu:///system start mindos-qa-bios
python3 scripts/tests/test_suspend_vm.py --dom mindos-qa-bios --mode s2idle
python3 scripts/tests/test_suspend_vm.py --dom mindos-qa-bios --mode deep
# Shut down the guest, then restore its original devices and boot settings:
virsh -c qemu:///system define build/qa-original.xml
```

The generator only writes XML. It removes virtiofs (which refuses suspend),
places the virtio GPU behind an unused PCIe root port, enables
`x-pcie-pm-no-soft-reset`, and exposes S3 for diagnostic testing. It rejects ordinary domain names and
never overwrites the original XML. No installed disk or desktop preference
is changed. The generated fixture was exercised on the BIOS/Nehalem QA guest.

PCIe placement matters: QEMU exposes PCI power management for this device only
behind a root port. Setting the no-reset property on the root-bus VGA device
has no effect on the kernel's decision. The working guest reports a Power
Management capability and `setpci -s 01:00.0 CAP_PM+4.w` returns `0008` (substitute
the GPU address shown by `lspci` for other XML layouts). The kernel's virtio PCI
suspend path then preserves the device rather than resetting its resources.
See [QEMU's virtio PCI implementation](https://github.com/qemu/qemu/blob/master/hw/virtio/virtio-pci.c)
and [Linux's virtio PCI power-management path](https://github.com/torvalds/linux/blob/master/drivers/virtio/virtio_pci_common.c).

The test verifies the kernel's suspend entry/exit, unchanged boot/compositor/app
identities, a visible lock screen, real password unlock, and a surviving usable
terminal. It saves screenshots and journal output, restores the selected sleep
mode, and removes its terminal. S3 checks also watch for delayed resets; the current Q35/SeaBIOS guest fails that extended check despite restoring its display and accepting the password. Suspend-to-idle passes. It
does not reset or reboot a failing guest. The virtio guest's RTC alarm did not
wake suspend-to-idle reliably; the test uses a virtual power-button event, and
QEMU's wakeup operation for S3. These checks cover virtual hardware, not a
physical gaming GPU or laptop's sleep behavior.
