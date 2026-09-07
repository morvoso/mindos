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
- [x] Screenshot proof of the shell: dock and top bar, columns and tiles layouts with title bars, Settings › Mind, a Qwen3.5 answer without tool lines (`docs/img/shell-*.png`, `settings-mind.png`, `mind-answer.png`)

## Milestone 2 — installable gaming system
- [x] `mindos-install` exercised end to end under UEFI in the libvirt dev VM (`docs/DEV-VM.md`); installed system boots into the session with mindd running
- [x] `mindos-pkg`: one install path across the MindOS/Arch repositories, Flathub and the AUR, driven by the Mind (`docs/PACKAGES.md`); "install octopi" from the Mind bar builds it from the AUR in the dev VM
- [x] Theme split: red only for the boot menu/kernel console; dark cyan HUD look for Plymouth, the compositor and the Mind bar
- [x] A way back from a bad update: Limine + snapper + snap-pac, every snapshot bootable from the menu with a RAM overlay, `mindos-boot restore` (`docs/ROLLBACK.md`)
- [x] `mindshell`: web-rendered desktop shell (Rust host + WebKitGTK + TypeScript UI) with a centred dock, top bar (tray, layout switcher, clock), desktop widgets and KDE-style edit mode; compositor IPC for window lists, focus and minimize
- [x] Three window layouts in `mindwm` (floating like KDE, tiles like Hyprland, columns like Niri) switched from the top bar, `Super+T` or Settings and remembered across sessions; server-side title bars in the MindOS look
- [x] Settings app (Mind: tool lines, thinking, model catalog; Wallpaper; Displays with basic/advanced modes over the compositor's `set_output`; Desktop; About) as a `mindshell --app` window
- [x] Standard apps instead of home-grown ones (`mindos-apps`): Nautilus, Loupe, File Roller and Text Editor in the MindOS colours (libadwaita named colours), default handlers, "Open in Terminal" in Files; the shell implements the Wallpaper portal so "Set as Background" in Files and Image Viewer works
- [x] A login screen: greetd with a MindOS greeter (the compositor in kiosk mode rendering `mindshell --app greeter`), themed like the desktop; autologin is an installer option
- [x] Qwen3.5 2B (Apache-2.0) as the default model, switchable at runtime from Settings or `mind model`, with catalog downloads and user-supplied GGUFs
- [ ] `mindos-install` under BIOS firmware
- [ ] `linux-mindos-nvidia-open` prebuilt modules (no DKMS build on first boot)
- [x] Health checks (`mind health`, every 30 min and after every update): failed units, kernel/driver mismatch, disk, pacnew, kernel errors, snapshots — as notices on the desktop
- [x] Performance modes (`mindos-perf`: balanced / performance / quiet, sched_ext `scx_lavd`, NVIDIA power limit) with GameMode hooks; the Mind sleeps while a game runs (`docs/PERFORMANCE.md`)
- [x] DLSS / FSR / XeSS swapper (`mindos-dlss`, Settings › Games) (`docs/GAMES.md`)
- [x] Notifications: the shell is the freedesktop notification server (toasts, the centre, do-not-disturb)
- [x] The shell is the session's polkit authentication agent: one themed password dialog for `pkexec`, systemd and the standard apps; GameMode's helpers allowed without a prompt (`docs/SHELL.md`)
- [ ] In-tree modules load with `module verification failed` (signature missing) on the current kernel build: keep module signatures intact or turn `MODULE_SIG` off
- [ ] Steam, gamescope, Proton, MangoHud, GameMode verified with a real game on the 4090
- [x] The Mind can look things up: web search, page reading, the Arch Wiki, Wikipedia and ProtonDB, downloads, all through curl behind a guard that refuses this machine and the local network, with fetched text treated as data and never as instructions (`docs/WEB.md`)
- [x] The Mind watches for updates: rules + model risk assessment, Arch news, notices with actions, optional auto-apply of low-risk updates (never during a game), post-update verification against the pre-update snapshot, one-click rollback (`docs/UPDATES.md`)
- [ ] Per-package release-note reading for the assessment (changelogs, upstream tags) rather than names and the news feed alone
- [ ] Pick a bigger catalog model (Qwen3.5 9B / 27B) automatically when a GPU with enough memory is present; CUDA backend by default on NVIDIA
- [ ] Pointer constraints and relative pointer verified with a first-person game under XWayland

## Milestone 3 — the mind grows up
- [ ] Screen understanding (screencopy → vision model) for "what is this error"
- [ ] Voice input, per-game profiles managed by the model (DLSS version, performance mode, MangoHud per game)
- [ ] Signed MindOS repo, hosted mirror, release channel
- [ ] Rust kernel (`research/kernel-rs`) revisited only as a research project
