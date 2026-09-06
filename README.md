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
driver ecosystem has to be redone. The whole boot, from GRUB through the
kernel console to the desktop, is white text on MindOS red (`#8c1010`).

```
┌─────────────────────────────────────────────────────────────────────┐
│  mindwm  ── MindOS compositor (Wayland + XWayland, DRM/KMS, libinput)│
│  ┌────────────┐ ┌──────────┐ ┌──────────┐ ┌────────────────────┐   │
│  │ Mind bar   │ │ Steam    │ │ Gamescope│ │ any Linux app      │   │
│  │ (LLM chat) │ │ (X11)    │ │ (nested) │ │ (Wayland / X11)    │   │
│  └─────┬──────┘ └──────────┘ └──────────┘ └────────────────────┘   │
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
| `mindwm` | `mindwm/`, `packages/mindwm/` | The compositor (Rust, Smithay). Game mode by default: every window opens maximized, `Super+F` fullscreens, `Super+Space` opens the **Mind bar** (launcher, shell and LLM chat in one field). Wayland and XWayland. See `docs/COMPOSITOR.md`. |
* [docs/DEV-VM.md](docs/DEV-VM.md) — persistent development VM on libvirt/virt-manager: shared source tree, snapshots, dev loops
| `mindd` / `mind` | `mindd/`, `packages/mindos-mind/` | The mind: a system daemon that runs `llama-server` on a local GGUF model, exposes typed tools (packages, updates, services, journal, files, commands, kernel parameters, game library) behind an observe/change/forbidden policy, logs everything to `/var/log/mindos/mind.jsonl`, and speaks newline-delimited JSON on `/run/mindos/mind.sock`. `mind` is the CLI. |
| `mindos-base` | `packages/mindos-base/` | Identity and tuning: `os-release`, kernel command line, sysctl (`vm.max_map_count`, BBR, split-lock mitigation off), zram, I/O scheduler and controller udev rules, NVIDIA modprobe defaults, mkinitcpio preset. |
| `mindos-session` | `packages/mindos-session/` | greetd config (`/etc/mindos/greetd.toml`), the `mindos-session` launcher, `session-startup`, the compositor defaults (`/etc/mindos/mindwm.toml`). |
| `mindos-theme` | `packages/mindos-theme/` | White-on-red GRUB, Plymouth theme, console theme service, wallpaper and icon. |
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

Developing the compositor does not need the container:

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
* `docs/THEME.md` — the white-on-red boot theme, stage by stage.
* `docs/ROADMAP.md` — what is done and what is next.
