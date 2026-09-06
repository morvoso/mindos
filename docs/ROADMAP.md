# Roadmap

## Milestone 1 — bootable MindOS image
- [x] Pivot from the from-scratch kernel to an Arch-based distribution with a custom kernel (`research/kernel-rs` parked)
- [x] Repository restructure, Docker build box, `make` targets, docs
- [x] `linux-mindos` package builds (7.2.3 + BORE + console theme, Clang ThinLTO, native CPU) and boots under KVM with the white-on-red console
- [x] `mindd` daemon + `mind` CLI: llama.cpp child, tool calling, observe/change/forbidden policy, audit log, Unix-socket protocol
- [x] `mindwm` compositor: DRM and nested backends, XWayland, game-mode window placement, Mind bar (launcher + shell + chat), wordmark, config file, keybindings
- [x] `mindos-base`, `mindos-session`, `mindos-theme`, `mindos-gaming`, `mindos-dev`, `mindos-install` packages
- [x] `[mindos]` repository and archiso profile produce an ISO with the bundled model
- [x] ISO boots in QEMU into mindwm with mindd answering (software rendering in the VM)
- [x] Screenshot proof: red/white boot, dark Plymouth splash, dark compositor, Mind bar answering a question (`docs/img/`)

## Milestone 2 — installable gaming system
- [x] `mindos-install` exercised end to end under UEFI in the libvirt dev VM (`docs/DEV-VM.md`); installed system boots into the session with mindd running
- [x] `mindos-pkg`: one install path across the MindOS/Arch repositories, Flathub and the AUR, driven by the Mind (`docs/PACKAGES.md`); "install octopi" from the Mind bar builds it from the AUR in the dev VM
- [x] Theme split: red only for GRUB/syslinux/kernel console; dark cyan HUD look for Plymouth, the compositor and the Mind bar
- [x] `mindshell`: web-rendered desktop shell (Rust host + WebKitGTK + TypeScript UI) with bottom launcher panel, top bar (tray, clock), desktop widgets and KDE-style edit mode; compositor IPC for window lists, focus and minimize
- [ ] `mindos-install` under BIOS firmware
- [ ] `linux-mindos-nvidia-open` prebuilt modules (no DKMS build on first boot), `mind doctor` GPU checks
- [ ] In-tree modules load with `module verification failed` (signature missing) on the current kernel build: keep module signatures intact or turn `MODULE_SIG` off
- [ ] Steam, gamescope, Proton, MangoHud, GameMode verified with a real game on the 4090
- [ ] `mindos-update.timer`: LLM-driven nightly update with report
- [ ] Larger default model (7B–14B class) when a GPU is present; CUDA backend by default on NVIDIA
- [ ] Pointer constraints and relative pointer verified with a first-person game under XWayland

## Milestone 3 — the mind grows up
- [ ] Screen understanding (screencopy → vision model) for "what is this error"
- [ ] Voice input, notifications, per-game profiles managed by the model
- [ ] Signed MindOS repo, hosted mirror, release channel
- [ ] Rust kernel (`research/kernel-rs`) revisited only as a research project
