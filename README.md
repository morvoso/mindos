# MindOS

MindOS is a gaming-first Linux distribution with a local language model as the
mind of the system. It boots straight into its own Wayland compositor, runs
ordinary Linux (Wayland and X11) applications, Steam and Proton games, loads
the NVIDIA and Mesa drivers like any other distribution, and hands system
administration to an on-device LLM: updates, driver installs, diagnostics and
configuration are conversations, not man pages.

MindOS is built the way Arch, CachyOS and SteamOS are built: on top of the
Linux kernel and the Arch package ecosystem, with its own kernel package, its
own packages, its own repository and its own image. Nothing in the Linux
driver ecosystem has to be redone. The boot loader and the kernel console are
white text on MindOS red (`#8c1010`). The desktop uses warm graphite or light
stone surfaces, orange accents, square panels, a floating bottom shelf and a
unified installed-game library. Its live circuit background can be switched
off and pauses during games. Inter carries the interface; Kitty uses
JetBrains Mono. See the [gaming desktop](docs/GAMING-DESKTOP.md) for the
launcher integrations, appearance controls and current boundaries.

```
┌─────────────────────────────────────────────────────────────────────┐
│  mindwm  ── MindOS compositor (Wayland + XWayland, DRM/KMS, libinput)│
│  ┌────────────┐ ┌──────────┐ ┌──────────┐ ┌────────────────────┐   │
│  │ Mind bar   │ │ Steam    │ │ Gamescope│ │ any Linux app      │   │
│  │ (LLM chat) │ │ (X11)    │ │ (nested) │ │ (Wayland / X11)    │   │
│  └─────┬──────┘ └──────────┘ └──────────┘ └────────────────────┘   │
│  mindshell ── dock, top bar, Settings, login: WebKit + TypeScript,   │
│               KDE-style edit mode, driven over the compositor IPC    │
├────────┼────────────────────────────────────────────────────────────┤
│  mindd ── LLM daemon: llama.cpp, tools, policy, audit log            │
│  mind  ── CLI: `mind update`, `mind "install steam"`, `mind doctor`  │
├─────────────────────────────────────────────────────────────────────┤
│  systemd · greetd + MindOS login screen · pacman + [mindos] repo     │
├─────────────────────────────────────────────────────────────────────┤
│  linux-mindos ── Linux 7.2 + BORE, 1000 Hz, full preempt, ntsync,   │
│                  Clang ThinLTO, tuned for Zen 5, white-on-red console│
└─────────────────────────────────────────────────────────────────────┘
```

Graphics setup and compatibility: [graphics drivers](docs/GRAPHICS.md).

## What is in the box

| Component | Where | What it does |
| --- | --- | --- |
| `linux-mindos` | `packages/linux-mindos/` | Custom kernel: kernel.org 7.2.y + BORE scheduler + MindOS console theme, built with Clang ThinLTO for generic x86-64 (native CPU optional), 1000 Hz, full preemption, `amd-pstate`, ntsync. Signed in-tree modules and a headers package for DKMS. |
| `mindwm` | `mindwm/`, `packages/mindwm/` | The compositor (Rust, Smithay). Three window layouts (floating like KDE, tiles like Hyprland, columns like Niri), title bars in the MindOS look, `Super+F` fullscreens, `Super+Space` opens the **Mind bar** (launcher, shell and LLM chat in one field), Super held with the mouse wheel steps through the windows. Wayland and XWayland. See `docs/COMPOSITOR.md`. |
| `mindshell` | `mindshell/`, `packages/mindshell/` | The desktop shell: a lean Rust host that opens layer-shell windows and renders them with WebKitGTK; the UI is HTML/CSS/TypeScript. A centred dock (pins, running apps), top bar (Mind status, performance mode, notifications, tray, audio, network, battery, layout switcher, clock), the notification centre and toasts (the shell is the freedesktop notification server), desktop widgets, a KDE-like **edit mode**, the **Settings** app (Mind, updates, performance, games, software, wallpaper, displays, screen, desktop), the screensavers and lock screen, and the login screen. See `docs/SHELL.md`. |
| `mindos-apps` | `packages/mindos-apps/` | The standard apps, existing ones in the MindOS look: Firefox, Files (Nautilus), Image Viewer (Loupe), Archive Manager (File Roller), Text Editor, Document Viewer (Papers), Celluloid and Calculator; libadwaita colours, default handlers, "Open in Terminal" in Files; their "Set as Background" works through the shell's Wallpaper portal. |
| `mindd` / `mind` | `mindd/`, `packages/mindos-mind/` | The mind: a system daemon that runs `llama-server` on a local GGUF model (Qwen3.5 2B by default; any model from the catalog or your own file), exposes typed tools (packages, updates, services, journal, files, commands, kernel parameters, game library, and the web: search, page reading, the Arch Wiki, Wikipedia, ProtonDB, downloads) behind an observe/change/forbidden policy, logs everything to `/var/log/mindos/mind.jsonl`, and speaks newline-delimited JSON on `/run/mindos/mind.sock`. It also watches for updates (rules + the model assess the risk, optional auto-apply of the low-risk ones, never while a game runs), verifies the system after every pacman run and points at the snapshot to go back to, runs health checks, and pushes all of that to the desktop as **notices**. `mind` is the CLI. See `docs/UPDATES.md` and `docs/WEB.md`. |
| `mindos-base` | `packages/mindos-base/` | Identity and tuning: `os-release`, kernel command line, sysctl (`vm.max_map_count`, BBR, split-lock mitigation off), zram, I/O scheduler and controller udev rules, NVIDIA modprobe defaults, mkinitcpio preset, and the **performance modes** (`mindos-perf`: balanced / performance / quiet — governor, EPP, boost, sched_ext `scx_lavd`, huge pages, NVIDIA persistence and power limit; GameMode switches to performance and puts the Mind to sleep while a game runs, see `docs/PERFORMANCE.md`). The boot menu and the way back: Limine, snapper + snap-pac (a snapshot before and after every pacman run) and `mindos-boot`, which lists the snapshots in the boot menu and restores one. See `docs/ROLLBACK.md`. |
| `mindos-session` | `packages/mindos-session/` | greetd config (`/etc/mindos/greetd.toml`), the MindOS login screen (`mindos-greeter`: mindwm in kiosk mode + `mindshell --app greeter`), the `mindos-session` launcher, `session-startup`, the compositor defaults (`/etc/mindos/mindwm.toml`). |
| `mindos-theme` | `packages/mindos-theme/` | White-on-red boot menu (Limine) and console theme service (the boot stage), the dark animated Plymouth theme, the display fonts (Orbitron, Share Tech Mono), the system font rendering defaults for fontconfig, wallpaper and icon. |
| `mindos-gaming` | `packages/mindos-gaming/` | Steam, gamescope, GameMode, MangoHud, Lutris, Wine and the 32-bit runtime, gaming sysctl and `gamemode.ini` (hooked to `mindos-perf`), and **`mindos-dlss`**, the DLSS / FSR / XeSS swapper (Settings › Games): every game's upscaler DLLs, a library of versions from the vendors' manifests, swap and restore. See `docs/GAMES.md`. |
| `mindos-install` | `packages/mindos-install/` | Guided installer run from the live ISO (GPT, btrfs subvolumes, user, timezone, automatic GPU selection and gaming tools). |
| ISO | `iso/` | archiso profile: live system that boots into the compositor with the `mind` user logged in, the `[mindos]` repo and the bundled model on the image, root shell on tty2. |
| Rust kernel | `research/kernel-rs/` | The original from-scratch kernel experiment. Parked; not on the product path. |

## Building

Everything builds inside a Docker "build box" (an Arch container), so the
host needs only Docker, QEMU and Python:

```sh
scripts/buildbox.sh --build        # one-time: build the container image
make kernel                        # packages/linux-mindos → build/packages/ (~20 min on 8 cores)
make nvidia                        # prebuilt modules, signed with that kernel's build key
make packages                      # every other MindOS package
make repo                          # build/repo: the [mindos] pacman repository
make iso                           # build/out/mindos-<date>-x86_64.iso
make qemu-bios                     # boot the ISO in QEMU (KVM, virtio-gpu, BIOS)
make screenshot                    # build/qemu/screen.png via the QEMU monitor
```

`make iso` refreshes the local repository before staging the image. The repository
contains the newest version of each package for x86-64 (including architecture-
independent packages), using pacman's version comparison. Older build archives
remain in `build/packages` for development; they are not copied into the ISO.
A failed repository build leaves the previously published directory intact.

Kernel and Rust packages default to portable x86-64 code. For a build used
only on the build machine, `MINDOS_CPU=native make kernel nvidia packages` enables
CPU-specific optimization. `MINDOS_JOBS=8` limits compiler parallelism and
`MINDOS_LTO=none` disables kernel ThinLTO for quicker development builds.
These options are forwarded into the build container; the kernel target
uses a clean source tree so repeated patch application cannot corrupt a build.

Developing the compositor does not need the container; the shell UI only needs
Node (`cd mindshell/ui && npm install && npm run build && npm run shot` renders
preview screenshots with headless Chromium), and the shell host builds in the
container (`scripts/buildbox.sh bash -c 'cd mindshell && cargo build --release'`):

```sh
cd mindwm && cargo build --release
./target/release/mindwm --winit             # nested window on your desktop
./target/release/mindwm --tty-udev          # real DRM/KMS session (from a TTY)
```

## Target hardware

MindOS is tuned first for the machine it is developed on: AMD Ryzen 7 9800X3D
and NVIDIA RTX 4090. Native CPU compilation is an opt-in build choice; `amd-pstate`
runs in active mode, the NVIDIA open kernel modules are prebuilt and signed for
the MindOS kernel (with an installer DKMS fallback when versions differ; see
[graphics drivers](docs/GRAPHICS.md)), and `mindd` prefers the CUDA build of llama.cpp when
`ggml-cuda` is installed. It still boots and runs on other x86-64 machines
(and in QEMU with Mesa software rendering).

## Documentation

* `docs/ARCHITECTURE.md` — how the pieces fit together.
* `docs/COMPOSITOR.md` — mindwm features, keybindings, the Mind bar, configuration.
* `docs/SHELL.md` — mindshell: the web-rendered desktop shell, widgets, edit mode, the bridge and the compositor IPC.
* `docs/THEME.md` — the theme, stage by stage: red boot loader and console, dark cyan loading screen, compositor and shell.
* `docs/PACKAGES.md` — where packages come from and how `mindos-pkg` and the Mind install them.
* `docs/ROLLBACK.md` — updates and the way back: Limine, snapper snapshots around every pacman run, booting a snapshot, `mindos-boot restore`.
* `docs/UPDATES.md` — the Mind as the system's minder: update watch and risk assessment, notices, health checks, post-update verification, auto-apply, rollback.
* `docs/WEB.md` — the Mind on the web: search, page reading, the wikis, ProtonDB, downloads, and the guard that keeps a page from reaching this machine.
* `docs/VALIDATION.md` — tested performance/UI changes, VM evidence and remaining release checks.
* `docs/PERFORMANCE.md` — the performance modes (`mindos-perf`), sched_ext, GameMode hooks, the Mind sleeping during games.
* `docs/GAMES.md` — the DLSS / FSR / XeSS swapper (`mindos-dlss`, Settings › Games).
* `docs/ROADMAP.md` — what is done and what is next.
* `docs/DEV-VM.md` — persistent development VM on libvirt/virt-manager: shared source tree, snapshots, dev loops.

## Licence

MindOS is copyright © 2026 Black Arrow Software, LLC. See `LICENSE`.

You may install it and run it on as many machines as you like, for anything you
like, and pass an unchanged copy on to anyone. You may not sell it, and you may
not publish a changed version of it — no forks, no re-spins, no rebranded
images. Modify your own copy on your own machines all you want.

That licence covers the parts Black Arrow wrote: `mindwm`, `mindshell`, `mindd`
and `mind`, the MindOS packages, the installer, the ISO profile, the theme and
the artwork. It covers nothing else on the image. The Linux kernel stays
GPL-2.0, the Arch packages keep their own licences, the fonts stay under the
SIL OFL, and Qwen3.5 stays Apache-2.0 — `THIRD-PARTY.md` lists all of it,
including where to get the source for the GPL parts.

MindOS and Black Arrow Software are names and marks of Black Arrow Software,
LLC, and are not licensed for use on anything else.

## Issues

Bugs, feature requests and anything that should work differently go here:

**<https://github.com/morvoso/mindos/issues>**

That is the way to get MindOS changed. Black Arrow Software, LLC maintains it
and makes the updates; the issue tracker is where the work comes from.
