# Graphics drivers

MindOS installs Mesa's AMD and Intel graphics stack by default, including
32-bit Vulkan support for games. The installer detects display devices by
numeric PCI class; a GPU's audio or USB controller is not a second GPU.

For NVIDIA, automatic selection checks the supported-products table shipped
with the live image's NVIDIA driver. Current supported cards get Arch's
`nvidia-open`, `nvidia-utils`, the 32-bit libraries and `nvidia-settings`.
Integrated AMD/Intel graphics are retained on hybrid systems. The installer
does not change the running live session's GPU driver.

Nothing here is pinned, and that is the whole point. Arch builds `nvidia-open`
against its own `linux` package and releases the two together, so `pacman -Syu`
moves the kernel and its modules in one transaction and a driver upgrade can
never strand the module. MindOS used to ship prebuilt modules signed for a
kernel of its own, pinned to an exact `nvidia-utils` version; the next Arch
driver release then refused to upgrade on every installed machine. There is no
prebuilt-versus-DKMS decision left to make, and no `MINDOS_NVIDIA_MODULES`
switch, because there is no longer anything to choose between.

Before formatting, the installer refreshes repositories and resolves the full
package selection in a temporary package database it then deletes, so packages
already present in the live session cannot hide a missing target dependency.
Unresolved dependencies abort before disk changes; downloads still require a
working connection during installation.

On a machine running `linux-lts`, `linux-zen` or a hand-built kernel, install
`nvidia-open-dkms` instead — it compiles against whatever kernel is installed:

```sh
sudo pacman -Syu nvidia-open-dkms linux-lts-headers
```

Check that DKMS and initramfs hooks succeed before rebooting. This does not
establish Secure Boot support for the distribution.

For image builders there is no kernel or driver step: `make packages`, then
`make repo`, then `make iso`. The ISO takes `linux` and `nvidia-open` from
Arch's own repositories at build time.

Read the proposed graphics plan without starting an installation:

```sh
python3 /usr/lib/mindos/install-gpu.py
python3 /usr/lib/mindos/install-gpu.py --json
```

The current NVIDIA open modules support Turing and newer hardware. NVIDIA's
590 series dropped Pascal and older GPUs from the main Arch driver packages;
installing those packages on unsupported cards can break the graphical
session. [NVIDIA's supported GPU list](https://github.com/NVIDIA/open-gpu-kernel-modules#compatible-gpus),
[Arch's driver transition notice](https://archlinux.org/news/nvidia-590-driver-drops-pascal-support-main-packages-switch-to-open-kernel-modules/).

The installer stops before asking for passwords or changing disks when any
NVIDIA display device is legacy or absent from its support table. This also
covers mixed old/new NVIDIA systems. It reports the device and legacy branch,
when known, instead of silently leaving one display without a usable driver.
A missing or outdated support table also stops automatic NVIDIA selection;
use a current live image. Hardware is checked again after the install review.

An explicit compatibility installation can use Mesa/Nouveau instead:

```sh
sudo env MINDOS_GPU_DRIVER=mesa mindos-install
```

This selects Nouveau early KMS and its 64-bit/32-bit Vulkan packages, omitting
NVIDIA's driver packages. Support and gaming performance depend on
the particular GPU; choosing this option is not a promise of full-speed
legacy NVIDIA gaming. In particular, a maintained proprietary legacy branch
may be necessary. MindOS does not yet ship or validate those legacy branches
against its kernel. Installing an untested legacy DKMS package is not part
of the automatic installer.

On AMD and Intel, the installer retains the detected kernel driver for early
KMS (including `radeon` on older AMD hardware and `i915`/`xe` on Intel).
The initramfs KMS hook handles the remaining detected display modules.

### How the drivers load, and why not from `MODULES`

The installer writes the detected drivers to `MINDOS_GPU_MODULES` in
`/etc/mkinitcpio.conf.d/mindos.conf`, and `MODULES` is left empty. The two are
not interchangeable. Both put a driver in the initramfs, but `MODULES` also
writes it into `/etc/modules-load.d/MODULES.conf` inside the image, where
`systemd-modules-load` loads the list one module at a time — and `initrd.target`
is ordered after that service, so the boot cannot switch root until the slowest
GPU probe has returned.

That cost is not small. On a two-display NVIDIA machine the root filesystem was
mounted and fsck'd 5.71s into the boot, and then nothing happened at all until
`nvidia_drm`'s probe returned at 9.95s — `systemd-modules-load` reported 7.171s
of CPU over 7.106s of wall clock, one process on one core. Early KMS was buying
nothing for that: the driver became ready 0.26s before the initramfs ended.

The `mindos-gpu` hook (`/usr/lib/initcpio/install/mindos-gpu`) adds the same
modules with `add_module` and writes no `modules-load.d` entry. Every display
driver carries a PCI modalias, so udev's `80-drivers.rules` loads it during
coldplug from the udev worker pool — in parallel, with no unit ordered behind
it. `nvidia_uvm` is not in the list at all: it is the CUDA unified-memory
driver, nothing in early boot opens it, and nvidia-utils' `60-nvidia.rules`
runs `nvidia-modprobe -c0 -u` to load it for the first CUDA context.

The compositor is the one part of userspace that cannot start before a driver is
up, so `greetd` alone waits for one, through `ExecStartPre=/usr/lib/mindos/wait-drm`.

## Session access to graphics devices

New installed and live users rely on logind's active-session access to primary
DRM devices; they are not added to `video`. Rendering nodes remain available
for graphics work. This prevents an app started while the desktop is inactive
from becoming DRM master and blocking the display on return. The compositor
opens its display devices through libseat/logind.

Earlier installations may still have `video` membership. To adopt the default,
run `sudo gpasswd -d "$USER" video`, then reboot so the user service manager
and every application receive the new groups. Package updates do not alter
existing users' group choices automatically. Keep deliberate device-access
customizations only when your workflow requires them.

The shipped brightnessctl uses logind's brightness API, so backlight control
does not require `video` membership. Physical backlight adjustment still needs
hardware validation. See [logind's device access contract](https://www.freedesktop.org/software/systemd/man/latest/org.freedesktop.login1.html)
and [brightnessctl's permission methods](https://github.com/Hummer12007/brightnessctl#permissions).

## Gaming defaults and waking the displays

CUDA is optional and is not selected by GPU detection. The desktop keeps the
Vulkan backend for Mind; installing a graphics driver does not install a CUDA
development toolkit. Interactive installation offers Mesa/Nouveau compatibility
or cancellation if automatic NVIDIA selection fails, before any disk changes.
Unattended installation still fails unless compatibility was explicitly selected.

The current open NVIDIA driver uses `NVreg_UseKernelSuspendNotifiers=1` and
`NVreg_TemporaryFilePath=/var/tmp`. The older forced
`NVreg_PreserveVideoMemoryAllocations=1` setting is removed: its procfs sleep
mechanism is not the open driver's kernel notifier mechanism. See
[NVIDIA 610 power management](https://download.nvidia.com/XFree86/Linux-x86_64/610.57.04/README/powermanagement.html).

Mindwm resets display state and buffer ages after session activation, and
invalidates its overlay textures after display wake. Mindshell rebuilds its
panels and desktop content after wake while retaining the lock state and app
windows. These changes require physical NVIDIA/AMD/Intel wake testing; a
software-rendered test cannot establish that a GPU preserves application VRAM.
