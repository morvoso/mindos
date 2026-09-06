# MindOS architecture

## Goals

1. A complete, installable gaming distribution: boots on real hardware with
   NVIDIA, AMD and Intel GPUs, runs Steam/Proton, ordinary Wayland and X11
   applications, and updates itself.
2. A local LLM is the operator of the machine. Every administrative task
   (updating, installing drivers, changing settings, diagnosing problems) is
   performed by the model through audited tools, with the user confirming
   anything destructive.
3. Boot straight into the MindOS compositor. No display-manager screen, no
   desktop environment: the compositor *is* the desktop, and its built-in
   Mind bar is the launcher, the terminal and the chat.
4. Do not redo the ecosystem. Reuse the Linux kernel, systemd, pacman and the
   Arch repositories; ship the MindOS-specific pieces as packages in a MindOS
   repository.
5. Maximum performance on the development machine (Ryzen 7 9800X3D, RTX 4090)
   without breaking other x86-64 hardware.

## Why a distribution and not a kernel

The first MindOS prototype was a from-scratch kernel speaking the Linux syscall
ABI (see `research/kernel-rs/`). It could run static Linux binaries, but no
kernel other than Linux can load the NVIDIA driver, Mesa's DRM drivers or any
other Linux kernel module; graphics, sound, Wi-Fi and anti-cheat would all
have to be rewritten. A gaming OS needs those today, so MindOS is built the way
Arch, CachyOS and SteamOS are built: a custom Linux kernel package plus a
custom userland on top of an existing package ecosystem.

## Components

### linux-mindos (packages/linux-mindos)

Vanilla kernel.org stable (7.2.y) plus a small, reviewable patch set:

* **BORE** (Burst-Oriented Response Enhancer) scheduler patch for desktop and
  game responsiveness (`kernel.sched_bore=1`).
* **MindOS console theme**: the VT default attribute is white on red and the
  palette's red is MindOS red (`#8c1010`), so every boot message from the
  kernel onwards is white text on a red background.
* A DKMS/Clang compatibility patch so out-of-tree modules build with the same
  toolchain as the kernel.
* Config: `-mindos` local version, Clang + LLD with ThinLTO, `X86_NATIVE_CPU`
  (the build machine's CPU, Zen 5 here), 1000 Hz, full preemption, `NO_HZ`,
  `amd-pstate`, ntsync, futex2, zstd modules, BBR + fq, THP madvise, KVM and
  virtio guest support (for the QEMU test harness), amdgpu/i915/xe/nouveau
  and everything a desktop needs. Subsystems a gaming desktop never uses
  (media capture beyond UVC, InfiniBand, staging, ISDN, exotic buses) are off.
* `linux-mindos-headers` is shipped so DKMS modules (NVIDIA open modules,
  VirtualBox, ZFS) build normally; `mindos-base` pulls `nvidia-open-dkms`.

Verified: the package builds in the build box, boots under KVM
(`Linux version 7.2.3-1-mindos ... clang 22.1.8, LLD 22.1.8 ... SMP
PREEMPT_DYNAMIC`, `BORE CPU Scheduler modification`) and the console is white
on red from the first line.

### mindwm (mindwm/)

The MindOS compositor, written in Rust on Smithay 0.7 (a fork of the anvil
reference compositor with MindOS-specific pieces on top). Details, keybindings
and the config schema are in `COMPOSITOR.md`.

* Backends: DRM/KMS + libinput + libseat for real boots (`--tty-udev`), winit
  for nested development (`--winit`). Multi-GPU capable; when the primary
  device has no hardware EGL (virtual machines, GPUs without a driver) it
  falls back to Mesa's software rasterizer instead of refusing to start.
* Protocols: wl_compositor/shm, linux-dmabuf, xdg-shell, xdg-decoration,
  wlr-layer-shell, presentation-time, viewporter, relative-pointer,
  pointer-constraints, keyboard-shortcuts-inhibit, tearing-control,
  xdg-activation, foreign-toplevel, screencopy, data-device and primary
  selection, XWayland (Steam and most games are X11).
* Window model: game mode by default. New toplevels open maximized to the
  output's usable area; dialogs and transient windows are centred over their
  parent; `Super+F` toggles fullscreen, `Super+M` maximize, `Super+Tab`
  cycles, `Super+Q` closes, `Super+Enter` opens a terminal, `Super+Space`
  opens the Mind bar. Server-side decorations are off (games draw their own
  or none).
* The **Mind bar** is drawn by the compositor itself (text rasterised with
  fontdue into a memory buffer, uploaded as a texture): one field that is a
  launcher (type `steam`), a command line (`!pacman -Q | wc -l`) and a chat
  with the model (a question, or anything with `?`). Replies stream in; tool
  calls that need confirmation show as Y/N prompts; the model can launch apps
  and run commands in a terminal through client tools the compositor
  registers with mindd.
* Empty desktop: MindOS red with the wordmark and the three key hints.
* Talks to mindd over `/run/mindos/mind.sock`; reconnects when the daemon
  restarts.

### mindd and mind (mindd/)

The mind of the OS: a system daemon that owns the model and the tools.

* Runs `llama-server` (llama.cpp, from the Arch `llama-cpp` package with the
  Vulkan or CUDA backend) as a child with the configured model, picking the
  largest GGUF in `/var/lib/mindos/models` that fits the GPU when
  `path = "auto"`. The OpenAI-compatible API is bound to localhost and used
  only by mindd. An external server can be configured instead.
* An **agent loop** with tool calling. Tools are typed Rust functions with a
  JSON schema: `system_info`, `gpu_info`, `list_packages`, `search_packages`,
  `check_updates`, `apply_updates`, `install_packages`, `remove_packages`,
  `service_status`, `service_control`, `journal`, `read_file`,
  `write_config`, `run_command`, `set_kernel_parameter`, `game_library`,
  `launch`. Clients can register additional tools (the compositor registers
  `launch_app`, `open_terminal` and `run_in_terminal`).
* The package tools go through `mindos-pkg` (shipped by `mindos-mind`), which
  resolves a plain name against the MindOS/Arch repositories, then Flathub,
  then the AUR (built by the unprivileged `mindos-build` user) and reports
  which source it used. See `docs/PACKAGES.md`.
* A **policy layer** classifies every tool call as *observe* (runs
  immediately), *change* (the requesting client must confirm unless autopilot
  is on for that category) or *forbidden* (never: wiping disks, disabling
  the audit log, ...).
* Every request, model reply and tool call is appended to
  `/var/log/mindos/mind.jsonl`; `mind history` shows it.
* Protocol: newline-delimited JSON over a Unix socket owned by `root:mindos`
  (mode 660). Requests: `hello`, `chat`, `confirm`, `tool_result`, `cancel`,
  `status`, `history`. Events: `welcome`, `delta`, `tool_call`,
  `tool_result`, `client_tool`, `done`, `status`, `history`, `error`.
* Configuration: `/etc/mindos/mind.toml` (model, daemon, policy sections).

### mindos-session (packages/mindos-session)

greetd on VT 1 logs the user into `mindos-session`, a script that exports the
Wayland environment (Qt, GTK, SDL, Firefox, Java hints) and execs
`mindwm --tty-udev` with its output in the journal (`journalctl -t mindwm`).
Once the Wayland socket is up the compositor runs
`/usr/lib/mindos/session-startup`, which publishes the display to
`systemd --user` and D-Bus, starts the portal, and launches every executable
in `/etc/xdg/mindos/autostart` and `~/.config/mindos/autostart`. A crash of
the compositor drops back to greetd, which restarts the session; a root shell
stays available on tty2 on the live ISO.

### mindos-base, mindos-theme, mindos-gaming, mindos-dev (packages/)

* **base**: `/etc/os-release`, the kernel command line (`amd_pstate=active
  preempt=full nvidia_drm.modeset=1 nvidia_drm.fbdev=1 quiet splash`), sysctl
  tuning (`vm.max_map_count` for Proton, BBR, dirty-page bounds, split-lock
  mitigation off, `kernel.sched_bore`), zram swap, I/O scheduler and
  game-controller udev rules, NVIDIA modprobe defaults, mkinitcpio preset and
  the pacman hook that re-applies branding after updates. Depends on
  `linux-mindos`, `linux-mindos-headers`, the NVIDIA and Mesa stacks.
* **theme**: GRUB colours, Plymouth `mindos` theme, console theme service,
  wallpaper, icon.
* **gaming**: Steam, gamescope, GameMode (with `gamemode.ini`), MangoHud,
  Lutris, Wine and the lib32 runtime.
* **dev**: base-devel, git, Rust, Clang/LLVM, CMake, Node, Python, Docker,
  editors and shell tools.

### mindos-install (packages/mindos-install)

A guided installer run as root from the live ISO. GPT with a BIOS boot
partition, a 1 GiB EFI system partition on `/boot` and btrfs with `@`,
`@home`, `@log`, `@pkg` and `@snapshots` subvolumes; installs from the
bundled `[mindos]` repository plus the Arch mirrors; asks for disk, hostname,
user, password, timezone and whether to add the gaming and development
stacks. Fully non-interactive with `MINDOS_AUTO=1` and `MINDOS_*` variables.

### The image (iso/)

An archiso profile with BIOS (syslinux) and UEFI (GRUB) boot modes, both
white on red. The live system boots into the compositor with the `mind` user
logged in, opens a terminal with the welcome text, and has mindd running on
the bundled model. The `[mindos]` repository and the model live on the image
(`/var/lib/mindos/repo`, `/var/lib/mindos/models`), so installation works
offline.

## Build system

All builds run in `scripts/buildbox.sh`, a Docker container based on
`archlinux:base-devel` with archiso, the kernel toolchain and Rust
(`--root` for the privileged steps). `make` targets: `kernel`, `packages`,
`repo` (`repo-add` into `build/repo`), `iso-stage` (profile + repo + model
into `build/iso-profile`), `iso` (`mkarchiso` into `build/out`), `qemu` /
`qemu-bios` (boot the ISO with KVM and virtio-gpu), `screenshot`, `qemu-stop`.

## Boot sequence

```
firmware → GRUB or syslinux (white on red)
  → linux-mindos (white-on-red VT, plymouth "mindos")
  → systemd → mindd (llama-server loads the model) · greetd on VT 1
  → mindos-session → mindwm (DRM/KMS) → session-startup → autostart
  → Mind bar (Super+Space): "What should we do?"
```
