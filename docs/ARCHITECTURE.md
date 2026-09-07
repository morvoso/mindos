# MindOS architecture

## Goals

1. A complete, installable gaming distribution: boots on real hardware with
   NVIDIA, AMD and Intel GPUs, runs Steam/Proton, ordinary Wayland and X11
   applications, and updates itself.
2. A local LLM is the operator of the machine. Every administrative task
   (updating, installing drivers, changing settings, diagnosing problems) is
   performed by the model through audited tools, with the user confirming
   anything destructive.
3. Boot straight into the MindOS compositor. No display-manager screen. The
   compositor owns windows and input and carries the always-available Mind
   bar; `mindshell`, a lean web-rendered shell, adds panels, launcher, tray
   and widgets on top and can be rearranged live in an edit mode.
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
  kernel onwards is white text on a red background. Red is reserved for this
  boot stage; the loading screen, compositor and shell are dark.
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
* Window model: three layouts, switched with `Super+T`, the icon next to
  the clock or the Settings app and remembered across sessions. *Floating*
  (KDE-like: windows keep their size, open centred and cascade), *Tiles*
  (Hyprland-like dwindle: every window is a tile, a new one splits the
  focused tile) and *Columns* (Niri-like: full-height columns on a strip
  that slides to the focused column). Dialogs float and are centred over
  their parent; `Super+F` toggles fullscreen, `Super+M` maximize, `Super+Tab`
  cycles, `Super+Q` closes, `Super+Enter` opens a terminal, a tap on Super
  or `Super+Space` opens the Mind bar.
* Server-side decorations in the MindOS look: a 30 px title bar (title,
  minimise / maximise / close; close only on a tile) for windows that
  negotiate server decorations (Qt, foot, SDL, Chromium, ...) and for the
  shell's own app windows; GTK apps keep their client-side bars; games that
  draw nothing get nothing.
* The **Mind bar** is drawn by the compositor itself (text rasterised with
  fontdue into a memory buffer, uploaded as a texture): one field that is a
  launcher (type `steam`), a command line (`!pacman -Q | wc -l`) and a chat
  with the model (a question, or anything with `?`). Replies stream in; tool
  calls that need confirmation show as Y/N prompts; the model can launch apps
  and run commands in a terminal through client tools the compositor
  registers with mindd.
* Empty desktop: void black with the cyan-glowing wordmark and the key hints
  (only visible when the shell is not running).
* Talks to mindd over `/run/mindos/mind.sock`; reconnects when the daemon
  restarts.
* Exposes a small IPC socket (`MINDWM_SOCKET`, newline-delimited JSON) with
  the window list, focus/close/minimize requests, the layout mode, the
  preferences (`$XDG_STATE_HOME/mindos/mindwm.json`), the outputs and their
  modes (the Displays settings change resolution, refresh rate, scale,
  position, rotation, VRR and the primary output through it) and shortcuts,
  used by mindshell (`docs/SHELL.md`).

### mindshell (mindshell/)

The desktop environment. One Rust process opens wlr-layer-shell windows on
the compositor (desktop background, panels, popups) and renders each with
WebKitGTK 6 (GPU compositing, one shared web process); the interface is
HTML/CSS/TypeScript with no framework. Panels and widgets are data
(`layout.json`); the KDE-style edit mode adds panels on any edge, adds,
reorders and configures widgets, and places widgets on the desktop. The
default layout is a centred, transparent dock (pinned and running apps,
macOS-style) and a top bar (Mind status, system tray, audio, network,
battery, the window-layout switcher, clock). There is no launcher button:
a tap on Super opens the Mind bar, which is the launcher. The same binary
also opens an ordinary window (`mindshell --app settings`): the
**Settings** app (Mind: tool lines on/off, thinking, model choice and the
download catalog; Wallpaper; Displays with a basic and an advanced mode;
Desktop: layout mode, panels, shortcuts; About) and the login screen
(`--app greeter`). The host provides the system side: desktop entries and
icon themes, the StatusNotifier tray, power, audio, network, battery and
stats, the desktop folder and the compositor IPC. Details in `SHELL.md`.

### Standard applications (packages/mindos-apps/)

MindOS ships existing applications wherever one can be themed instead of
writing its own; only what has to talk to the Mind or the compositor
(Settings, the shell, the login screen) is custom. `mindos-apps` pulls in
**Files** (Nautilus), **Image Viewer** (Loupe), **Archive Manager** (File
Roller) and **Text Editor** (GNOME Text Editor), all GTK 4 + libadwaita, and
the terminal is foot. It ships the default handlers (`mimeapps.list`), the
MindOS colours for libadwaita and GTK 3 (`/usr/share/mindos/gtk/`, linked
into each user's `gtk.css` by an autostart entry, see `THEME.md`), a
nautilus-python extension that adds "Open in Terminal" to the Files context
menus, and `mindos-wallpaper PATH`, a command that sets the shell's wallpaper.
"Set as Background" in these apps needs no extension: the shell host
implements the Wallpaper portal backend (`SHELL.md`).

### mindd and mind (mindd/)

The mind of the OS: a system daemon that owns the model and the tools.

* Runs `llama-server` (llama.cpp, from the Arch `llama-cpp` package with the
  Vulkan or CUDA backend) as a child with the configured model, picking the
  largest GGUF in `/var/lib/mindos/models` that fits the GPU when
  `path = "auto"`. The OpenAI-compatible API is bound to localhost and used
  only by mindd. An external server can be configured instead.
* The default model is **Qwen3.5 4B (Q4_K_M, 2.7 GB, Apache-2.0)**: small
  enough to sit beside a running game on any 8 GB GPU and reliable at tool
  calling; `make model` downloads it with its licence for the ISO. The
  choice is a symlink (`models_dir/default.gguf`) so any GGUF a user drops in
  works, and the Settings app switches models at runtime: a catalog
  (`/etc/mindos/model-catalog.json`: Qwen3.5 0.8B, 2B, 4B, 4B high quality,
  9B, 27B, all Apache-2.0) is downloaded with `curl` into the models folder
  with its licence, or the user points at a file of their own. "Think before
  answering" (Qwen3.5 reasoning) is off by default for quick answers; both
  settings live in `/var/lib/mindos/mind-prefs.json` and a change of either
  restarts `llama-server`.
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
  `status`, `history`, `models`, `set_model`, `set_thinking`,
  `download_model`, `cancel_download`. Events: `welcome`, `delta`,
  `tool_call`, `tool_result`, `client_tool`, `done`, `status`, `history`,
  `models`, `download`, `error`.
* Configuration: `/etc/mindos/mind.toml` (model, daemon, policy sections).

### The Mind managing the system

mindd is not only a chat backend. Three background loops make it the
system's minder (`docs/UPDATES.md`):

* `updates::watch` — checks for updates on a timer, tags and risk-rates
  them (rules first, then the model's short JSON assessment), reads the
  Arch news, and posts an `updates:available` notice with actions. With
  auto-apply on it installs low-risk updates itself, never while a game
  runs, and verifies the system after.
* `health` — the checks (`failed-units`, `kernel-stale`, `nvidia-*`,
  `disk-*`, `pacnew`, `kernel-errors`, `mindd-socket`, `no-snapshots`)
  run on a timer and a minute after every pacman transaction; the pacman
  hook `96-mindos-update-mark.hook` writes the transaction record they
  verify against, including the pre-update snapper snapshot.
* `notices` — the store and broadcaster. Any client that sends `subscribe`
  gets every notice, update status and sleep change as it happens; the shell
  keeps one such connection (`mindshell/src/mindwatch.rs`) and shows the
  events as toasts, in the notification centre and on the Settings pages.

The status the model sees is refreshed before every turn (a second system
message: performance mode, game running, pending updates, last update,
notices), so "what should I know?" is answered from the same facts the
notices came from. The tools grew accordingly: `update_status`,
`health_check`, `notices`, `list_snapshots`, `rollback`,
`performance_mode`, `dlss`, `mind_sleep`.

The Mind can **sleep**: `set_sleep` (and GameMode through `mindos-perf`)
stops `llama-server` so a game has the whole GPU; a chat request wakes it
and waits for the model to load. The supervisor in `mindd.rs` idles while
sleeping instead of restarting the server.

### Performance modes, GameMode and the shell as the notification server

`mindos-perf` (mindos-base) is a bash tool over the kernel's knobs:
governor / EPP / boost / platform profile, sched_ext through
`systemd-run --unit=mindos-scx.service scx_lavd`, THP and compaction,
swappiness, NVIDIA persistence and power limit (`docs/PERFORMANCE.md`).
`gamemode.ini` calls its `game-start` / `game-end` hooks; a run counter in
`/run/mindos/perf/` makes several games at once safe. The shell drives it
through `shell.run` (an allow-list of read-mostly helpers) and the
`sudoers` rule for the `mindos` group.

mindshell owns `org.freedesktop.Notifications` on the session bus
(`mindshell/src/notify.rs`, zbus on its own tokio thread). Notifications
reach the UI as `notify` events with resolved icon URLs; a `toast` layer
window on the primary output shows the new ones, the bell widget's popup
is the centre, and `ActionInvoked` / `NotificationClosed` go back to the
application. Mind notices travel the same way, so an update warning and a
Steam download both land in the same place.

It is also the session's polkit authentication agent
(`mindshell/src/polkit.rs`): it registers for the logind session, shows the
password dialog for anything that asks polkit — `pkexec`, systemd unit
management, the system pages of the standard apps — and hands the answer to
polkit's helper. GameMode's helpers skip the prompt through a rules file in
`mindos-gaming`, and `mindos-perf` stays on sudoers because its hooks run
with nobody watching (docs/SHELL.md, *The authentication dialog*).

### mindos-session (packages/mindos-session)

greetd on VT 1 shows the MindOS login screen, `mindos-greeter`: the compositor
in kiosk mode (`/etc/mindos/greeter/mindwm.toml`) running
`mindshell --app greeter` as the unprivileged `greeter` user, which relays the
login to greetd over its socket (PAM stays in greetd; docs/SHELL.md, *The
login screen*). The installer can add an `initial_session` for automatic
login instead. Either way greetd logs the user into `mindos-session`, a
script that exports the Wayland environment (Qt, GTK, SDL, Firefox, Java hints) and execs
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
  the pacman hook that re-applies branding after updates. Also the boot menu
  and the way back from a bad update: Limine, snapper with snap-pac, and
  `mindos-boot`, which writes `/boot/limine.conf`, keeps a kernel copy on the
  ESP for every snapshot, lists the snapshots in the menu and restores one
  (`docs/ROLLBACK.md`). Depends on `linux-mindos`, `linux-mindos-headers`,
  the NVIDIA and Mesa stacks.
* **theme**: the boot menu colours (Limine) and the console theme service (red boot stage), the
  dark animated Plymouth `mindos` theme, the MindOS fonts, wallpaper, icon.
* **gaming**: Steam, gamescope, GameMode (with `gamemode.ini` and the polkit
  rules its helpers need), MangoHud, Lutris, Wine and the lib32 runtime.
* **dev**: base-devel, git, Rust, Clang/LLVM, CMake, Node, Python, Docker,
  editors and shell tools.

### mindos-install (packages/mindos-install)

A guided installer run as root from the live ISO. GPT with a BIOS boot
partition, a 1 GiB EFI system partition on `/boot` and btrfs with `@`,
`@home`, `@log`, `@pkg` and `@snapshots` subvolumes; installs from the
bundled `[mindos]` repository plus the Arch mirrors; asks for disk, hostname,
user, password, timezone and whether to add the gaming and development
stacks. Fully non-interactive with `MINDOS_AUTO=1` and `MINDOS_*` variables.
Puts Limine on the EFI partition (UEFI entry "MindOS", plus the removable
path) and into the BIOS boot partition, activates the snapper `root`
configuration, takes a first snapshot ("MindOS installed") and writes the
boot menu.

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
`qemu-bios` (boot the ISO with KVM and virtio-gpu), `screenshot`, `qemu-stop`,
`model` (download the default Qwen3.5 4B GGUF and its licence into `models/`
for the ISO).

## Boot sequence

```
firmware → Limine (white on red; installed system) · GRUB/syslinux on the ISO
  → linux-mindos (white-on-red VT) → plymouth "mindos" (dark, cyan)
  → systemd → mindd (llama-server loads the model) · greetd on VT 1
  → mindos-greeter (mindwm kiosk + mindshell --app greeter: the login screen)
  → mindos-session → mindwm (DRM/KMS) → session-startup → mindos-shell.service
  → dock, top bar, tray · Mind bar (Super tap or Super+Space): "What should we do?"
```
