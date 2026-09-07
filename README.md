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
white text on MindOS red (`#8c1010`); from the loading screen onwards MindOS is
dark glass: a navy void with a cyan and violet aurora behind everything,
frosted translucent panels with soft corners, one electric-cyan accent.

```
┌─────────────────────────────────────────────────────────────────────┐
│  mindwm  ── MindOS compositor (Wayland + XWayland, DRM/KMS, libinput)│
│  ┌────────────┐ ┌──────────┐ ┌──────────┐ ┌────────────────────┐   │
│  │ Mind bar   │ │ Steam    │ │ Gamescope│ │ any Linux app      │   │
│  │ (LLM chat) │ │ (X11)    │ │ (nested) │ │ (Wayland / X11)    │   │
│  └─────┬──────┘ └──────────┘ └──────────┘ └────────────────────┘   │
│  mindshell ── dock, top bar, Settings, Files: WebKit + TypeScript,   │
│               KDE-style edit mode, driven over the compositor IPC    │
├────────┼────────────────────────────────────────────────────────────┤
│  mindd ── LLM daemon: llama.cpp, tools, policy, audit log            │
│  mind  ── CLI: `mind update`, `mind "install steam"`, `mind doctor`  │
├─────────────────────────────────────────────────────────────────────┤
│  systemd · greetd autologin · pacman + [mindos] repo · Arch ecosystem│
├─────────────────────────────────────────────────────────────────────┤
│  linux-mindos ── Linux 7.2 + BORE, 1000 Hz, full preempt, ntsync,   │
│                  Clang ThinLTO, tuned for Zen 5, white-on-red console│
└─────────────────────────────────────────────────────────────────────┘
```

## What is in the box

| Component | Where | What it does |
| --- | --- | --- |
| `linux-mindos` | `packages/linux-mindos/` | Custom kernel: kernel.org 7.2.y + BORE scheduler + MindOS console theme, built with Clang ThinLTO for the local CPU (`X86_NATIVE_CPU`), 1000 Hz, full preemption, `amd-pstate`, ntsync. Headers package for DKMS (NVIDIA). |
| `mindwm` | `mindwm/`, `packages/mindwm/` | The compositor (Rust, Smithay). Three window layouts (floating like KDE, tiles like Hyprland, columns like Niri), title bars in the MindOS look, `Super+F` fullscreens, a tap on Super opens the **Mind bar** (launcher, shell and LLM chat in one field). Wayland and XWayland. See `docs/COMPOSITOR.md`. |
| `mindshell` | `mindshell/`, `packages/mindshell/` | The desktop shell: a lean Rust host that opens layer-shell windows and renders them with WebKitGTK; the UI is HTML/CSS/TypeScript. A centred dock (pins, running apps), top bar (Mind status, tray, audio, network, battery, layout switcher, clock), desktop widgets, a KDE-like **edit mode**, and the **Settings** (Mind, wallpaper, displays, desktop) and **Files** apps. See `docs/SHELL.md`. |
| `mindd` / `mind` | `mindd/`, `packages/mindos-mind/` | The mind: a system daemon that runs `llama-server` on a local GGUF model (Qwen3.5 4B by default; any model from the catalog or your own file), exposes typed tools (packages, updates, services, journal, files, commands, kernel parameters, game library) behind an observe/change/forbidden policy, logs everything to `/var/log/mindos/mind.jsonl`, and speaks newline-delimited JSON on `/run/mindos/mind.sock`. `mind` is the CLI. |
| `mindos-base` | `packages/mindos-base/` | Identity and tuning: `os-release`, kernel command line, sysctl (`vm.max_map_count`, BBR, split-lock mitigation off), zram, I/O scheduler and controller udev rules, NVIDIA modprobe defaults, mkinitcpio preset. |
| `mindos-session` | `packages/mindos-session/` | greetd config (`/etc/mindos/greetd.toml`), the `mindos-session` launcher, `session-startup`, the compositor defaults (`/etc/mindos/mindwm.toml`). |
| `mindos-theme` | `packages/mindos-theme/` | White-on-red GRUB and console theme service (the boot stage), the dark animated Plymouth theme, the display fonts (Orbitron, Share Tech Mono), the system font rendering defaults for fontconfig, wallpaper and icon. |
| `mindos-gaming` | `packages/mindos-gaming/` | Steam, gamescope, GameMode, MangoHud, Lutris, Wine and the 32-bit runtime, plus gaming sysctl and `gamemode.ini`. |
| `mindos-dev` | `packages/mindos-dev/` | Compilers, Rust, Node, Python, Docker, editors, CLI tools. |
| `mindos-install` | `packages/mindos-install/` | Guided installer run from the live ISO (GPT, btrfs subvolumes, user, timezone, optional gaming/dev stacks). |
| ISO | `iso/` | archiso profile: live system that boots into the compositor with the `mind` user logged in, the `[mindos]` repo and the bundled model on the image, root shell on tty2. |
| Rust kernel | `research/kernel-rs/` | The original from-scratch kernel experiment. Parked; not on the product path. |

## Building

Everything builds inside a Docker "build box" (an Arch container), so the
host needs only Docker, QEMU and Python:

```sh
scripts/buildbox.sh --build        # one-time: build the container image
make kernel                        # packages/linux-mindos → build/packages/ (~20 min on 8 cores)
make packages                      # every other MindOS package
make repo                          # build/repo: the [mindos] pacman repository
make iso                           # build/out/mindos-<date>-x86_64.iso
make qemu-bios                     # boot the ISO in QEMU (KVM, virtio-gpu, BIOS)
make screenshot                    # build/qemu/screen.png via the QEMU monitor
```

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
and NVIDIA RTX 4090. The kernel is compiled for the local CPU, `amd-pstate`
runs in active mode, the NVIDIA open kernel modules are built by DKMS against
`linux-mindos-headers`, and `mindd` prefers the CUDA build of llama.cpp when
`ggml-cuda` is installed. It still boots and runs on other x86-64 machines
(and in QEMU with Mesa software rendering).

## Documentation

* `docs/ARCHITECTURE.md` — how the pieces fit together.
* `docs/COMPOSITOR.md` — mindwm features, keybindings, the Mind bar, configuration.
* `docs/SHELL.md` — mindshell: the web-rendered desktop shell, widgets, edit mode, the bridge and the compositor IPC.
* `docs/THEME.md` — the theme, stage by stage: red boot loader and console, dark cyan loading screen, compositor and shell.
* `docs/PACKAGES.md` — where packages come from and how `mindos-pkg` and the Mind install them.
* `docs/ROADMAP.md` — what is done and what is next.
* `docs/DEV-VM.md` — persistent development VM on libvirt/virt-manager: shared source tree, snapshots, dev loops.
