# Graphics drivers

MindOS installs Mesa's AMD and Intel graphics stack by default, including
32-bit Vulkan support for games. The installer detects display devices by
numeric PCI class; a GPU's audio or USB controller is not a second GPU.

For NVIDIA, automatic selection checks the supported-products table shipped
with the live image's NVIDIA driver. Current supported cards get
`linux-mindos-nvidia-open`, matching userspace and 32-bit libraries. These
modules are built and signed for the released MindOS kernel; installing them
requires no local module compilation. Integrated AMD/Intel graphics are retained on hybrid systems.
The installer does not change the running live session's GPU driver.

Before formatting, the installer refreshes repositories and resolves the full
package selection using an empty temporary package database. Installed live
packages cannot hide missing target dependencies. If Arch has advanced beyond
the prebuilt NVIDIA package's exact userspace dependency, the installer tries
`nvidia-open-dkms` with matching MindOS headers and Clang/LLVM/LLD instead. The
review shows this choice. Unresolved dependencies abort before disk changes;
downloads still require a working connection during installation.

`MINDOS_NVIDIA_MODULES=prebuilt` requires the prebuilt path;
`MINDOS_NVIDIA_MODULES=dkms` explicitly chooses local compilation. The default
is `auto`. These choices apply only when the graphics plan selects NVIDIA.

Prebuilt packages pin both kernel and NVIDIA userspace versions so an upgrade
cannot silently install incompatible halves. Until matching MindOS packages
are available, pacman may refuse a full upgrade. To switch an installed system
to the rolling DKMS path, run a full transaction and accept the provider conflict:

```sh
sudo pacman -Syu nvidia-open-dkms linux-mindos-headers clang llvm lld
```

Check that DKMS and initramfs hooks succeed before rebooting. DKMS uses its own
signing key; the prebuilt modules use the kernel's embedded build certificate.
This does not establish Secure Boot support for the distribution.

For image builders, run `make kernel`, then `make nvidia`, then `make packages`
and `make iso`. The NVIDIA recipe pins the source checksum, kernel package
version, kernel release and driver version. Update them together for a release.
The matching kernel's private signing key must still be available in its build
tree; `make clean` removes that tree. An archived matching key/certificate can
be supplied using `MINDOS_MODULE_SIGN_KEY` and `MINDOS_MODULE_SIGN_CERT` (paths
inside the build container). Neither is copied into the module package. Verify
the built package against the released kernel using
`scripts/tests/check_nvidia_package.py` before publishing it.

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
