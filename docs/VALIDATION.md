# Performance and desktop validation — 7–8 September 2026

This pass targets gaming overhead, correct mode transitions and usable desktop
controls. It preserves the existing dark glass theme and user layout changes.
The source tree has no project `CLAUDE.md` or `AGENTS.md`; the documentation
under `docs/`, package recipes and runtime code supply the project contract.
Cached third-party build dependencies are not project policy.

## Current image and readiness

The latest tested image is **r27**, built on 8 September 2026:
`build/out/mindos-2026.09.08-r27-x86_64.iso` (4,180,836,352 bytes).
SHA-256: `dff155a89cbe0bcf79cdad8f8ae747f9de6e84977c0b0fc77789354c9273adbe`.
It contains kernel 7.2.3-2.1, compositor 30, shell 47, base 26, session 22
and installer 11. The sections below are a chronological evidence log;
earlier test counts and image revisions describe those earlier runs.

| Requirement | Current evidence and practical limit |
| --- | --- |
| Install, boot and recover | Fresh BIOS gaming and UEFI minimal installations, Btrfs snapshot restore and cleanup exercised. r27's fresh UEFI live session boots with healthy services. See [installation](#portable-kernel-fresh-installation-and-idle-rendering) and [preflight](#prebuilt-nvidia-modules-and-installation-preflight). |
| Modern, adjustable desktop | Dark glass Settings, searchable navigation, staged Input changes, compact layouts and dialog recovery exercised in browser/native checks. See [interface](#game-tuning-reliability-and-interface) and [input preferences](#keyboard-and-mouse-preferences). |
| Everyday applications and developer flexibility | Browser, files, media, PDF and text apps are packaged; networking/audio controls and optional Games/Developer setup exercised. Developer tools remain an optional bundle, with Docker activation opt-in. |
| Gaming behavior | GameMode transitions, background-work suppression, DLL swap/restore, pointer constraints and Wayland/XWayland fullscreen switching exercised. Steam reaches sign-in and Xonotic movement/aim/fire passes in virgl; real Steam/Proton gameplay and FPS remain unverified. See [gaming input](#gaming-input-and-accelerated-vm-follow-up). |
| Keyboard, mouse and media controls | Latest compositor unit run: 58 passed. Native keyboard, pointer and media suites passed again with the new session-access defaults. See [media](#hardware-media-keys-and-fullscreen-feedback) and [graphics access](#active-session-graphics-access). |
| NVIDIA installation | Five prebuilt modules pass cryptographic/ABI checks and signature-enforced kernel loading reaches device detection. The QA guest has no NVIDIA GPU; rendering and legacy-driver compatibility remain unverified. |
| Screenshots and sharing | Screenshot and portal/PipeWire monitor capture exercised, including scale/rotation, cancellation and lock rejection. Hardware GPU-buffer capture and real OBS/Discord game recording remain open. |
| Suspend | Visible suspend-to-idle recovery, password unlock and application survival pass with the documented PCIe virtio fixture. Q35 S3 fails after a delayed watchdog reset; physical suspend is unverified. See [latest suspend evidence](#virtio-suspend-diagnosis-and-visual-recovery). |

Measured improvements so far concern desktop/CPU overhead and model prompt
processing, not game FPS. The development VM has received updated packages
without restarting its existing compositor, shell or model; its running
desktop therefore does not demonstrate every newly installed feature.
Disposable QA guests provide the native checks for those changes.

Completion of gaming-performance validation needs a dedicated physical test
machine and a usable game/scene. Record its CPU, GPU, display/VRR connection,
driver/kernel versions, power profile, game/Proton version and identical
resolution/settings for baseline and candidate runs. Capture repeat runs of
average FPS, 1% lows and frame times; check input, gamescope/VRR, capture, and
lock/password/application survival after hardware suspend. Save raw results
alongside the test conditions before claiming a performance improvement.
MindOS is now installed on the user's dedicated Samsung 980 PRO, as recorded
under [physical SSD installation](#physical-ssd-installation). Its first
physical boot and gaming measurements are still pending. The host's active
GPU must remain attached to CachyOS until the user chooses to reboot.

Public repository signing/hosting, voice/vision and automatic larger-model
selection remain roadmap work; they are separate from this desktop and
gaming-performance validation. This image is a tested development artifact,
not a declaration that every roadmap item or hardware release check is done.

## Completed and exercised

* Compositor IPC snapshot work is limited to one refresh per 16 ms even with
  high-rate input. A simulated 8 kHz input stream produces 63 snapshots in a
  second. Frame retry deadlines retain millihertz precision and avoid
  immediate catch-up loops after delays.
* Performance commands serialize state and device writes; concurrent game
  hooks, saved preferences and service restarts preserve the gaming interval.
  Active pstate governors restore the intended EPP, THP compaction is deferred
  in performance mode, and NVIDIA limits are resolved separately per card.
* Scheduled maintenance, app scans and GPU sampling yield to GameMode. The
  model stays unloaded during a game unless explicitly requested.
* Settings has an overview, grouped searchable navigation, clear mode state,
  advanced tuning, labelled controls, focus-contained dialogs and compact
  layouts. Network and audio widgets open usable connection/device controls.
* Default apps include Firefox, Papers, Celluloid and Calculator alongside
  the existing file, image, archive and text tools. Intel Vulkan drivers and
  SOF firmware are included. NetworkManager's applet supplies Wi-Fi selection
  and a secret agent. New installs leave Docker opt-in.
* GameMode's nice limit now covers MindOS's actual desktop group (`mindos`),
  matching its existing permission rules.

## Checks and artifacts

| Check | Result |
| --- | --- |
| mindwm Rust library tests | 47 passed |
| mindd Rust tests | 16 passed |
| mindshell Rust tests in the Arch build box | 29 passed |
| `python3 scripts/tests/test_perf.py` | 12 passed; temporary fake devices only |
| `npm run check` and `npm run build` in `mindshell/ui` | passed |
| `node scripts/tests/ui-smoke.mjs` | navigation/search, mode feedback, write failure recovery, keyboard dialogs, compact layout passed |
| Changed package builds | mindwm, mindshell, mindos-mind, base, gaming, session, apps and installer built |
| Local package repository | refreshed with built versions |
| Native development VM | six installed packages upgraded; WebKit overview, keyword navigation, real Quiet mode switch, Network Connections and Volume Control launch checked |
| Native GameMode integration | entered from Quiet, slept Mind, saved Balanced during gaming, survived `apply`, restored Balanced and woke Mind on exit; original Performance preference restored afterward |
| GameMode self-test in VM | client/reaper/supervisor, scripts, renice and I/O priority passed; overall test does **not** pass because CPU governors are absent and GameMode GPU overclocking is deliberately disabled |

Logs are under `build/logs/perf-*`, `vm-performance-install.log`,
`vm-gamemode-test.log` and `vm-perf-integration.log`. Browser screenshots
are under `build/shots/ui-smoke/`; native screenshots include
`docs/img/settings-overview.png` and `docs/img/settings-performance-modern.png`.
Original VM runtime files were saved in
`build/vm-backup/performance-ux/originals.tar.gz` before deployment. The VM
has no `mindos-gaming` metapackage: its existing GameMode configuration and
new nice limits were installed directly for the integration checks.

## Remaining release checks

These are open work, not claims established by a software-rendered VM:

* Benchmark the same real game/scene on the 9800X3D and RTX 4090: average FPS,
  1% lows, frame-time variance, idle power and input behavior. Verify Proton,
  relative pointer/constraints, gamescope, VRR and direct scanout on hardware.
* Verify hardware capture performance and real OBS/Discord sessions. Basic
  screenshots and portal/PipeWire monitor capture now pass in the VM; GPU
  buffers and modern ext-image-copy-capture support remain open.
* Verify laptop/handheld hardware using the rebuilt kernel; SoC/SOF audio,
  SoundWire, IIO and touchscreen drivers are enabled in the resolved config.
* Validate legacy NVIDIA gaming drivers on hardware. Automatic installation
  now rejects unsupported NVIDIA devices before disk changes; explicit Mesa/
  Nouveau compatibility is available, but proprietary legacy branches remain
  unshipped and untested. Portable kernel/userspace boot without AVX2 in the BIOS VM.

The existing Roadmap still tracks release signing, hardware gaming checks
and other unfinished features. Passing unit and UI tests is not evidence of
an FPS improvement or complete hardware compatibility.

## Rollback and installer follow-up

The pre-existing cleanup failure is resolved in `mindos-base` 0.1.0-10.
Ten integration tests passed on a disposable Btrfs filesystem, covering empty
children, nested data, writable/default/mounted roots, bind-mounted children,
symlinks and failure recovery. After installing the hook, the VM's existing
Snapper retention policy completed successfully, including snapshot 4;
`systemctl --failed` returned no units. Logs: `vm-snapshot-prune-test.log`,
`vm-snapshot-fix-install.log` and the guest's snapper-cleanup journal.

The installer now shares a read-only preflight between interactive and
unattended paths. It validates names, timezone and boolean settings before
building shell configuration; checks the entire target disk tree for mounts
and swap; and refuses an occupied `/mnt`. It rechecks storage after the
review prompt and no longer unmounts unrelated filesystems or disables all
swap. Eleven fixture tests passed; the real VM's active disk was correctly
rejected without mutation. `parted` is now an explicit installer dependency
for `partprobe`.

Portable userspace packages built and were installed in the VM. The new
kernel's resolved configuration passed 41 assertions for gaming settings,
audio/input hardware, portable CPU targeting and module signing. Full kernel
packaging and boot checks subsequently passed, as detailed below.

The three existing VM `.pacnew` files were reviewed and merged: only comments
changed, and parsed settings (including autologin) were preserved. Originals
are saved under `build/vm-backup/config-merge/`. The subsequent `mind health`
check reported all good.

## Portable kernel, fresh installation and idle rendering

Kernel 7.2.3-2 finished packaging. All 5,727 packaged modules retain their
signature footer; no private signing key is packaged. The development VM
booted the new kernel and signed virtio driver, with no failed services and
`mind health` reporting all good. NVIDIA's separately built DKMS module
still reports an untrusted signature; that is distinct from the fixed
in-tree signing failure. One initial boot attempt displayed emergency mode;
a subsequent normal boot succeeded. No persistent log of the failed attempt
was recovered, so its cause remains unconfirmed.

The BIOS/Nehalem QA VM booted the live desktop without AVX2. Its fresh
installation completed with gaming enabled and the developer bundle off,
using base 11, session 19 and mind 15 from the shared local repository.
The resulting root has named Btrfs mounts without numeric subvolume IDs.
Four regression checks cover normalization, comments, custom ID-only mounts,
permissions and symlinks. The installed system booted with zero kernel taint,
no failed services and the bundled model ready. A real rollback test restored
a fixture containing an old root subvolume ID; after reboot the old marker
was restored, the stale ID was gone, and health checks passed again.

NVIDIA driver/DKMS packages are selected for supported NVIDIA hardware;
they remain included on live media. The new Mesa-only installation has no
NVIDIA DKMS package. RealtimeKit is an explicit desktop dependency. Health
checks no longer report immutable live media as full disks or recommend
creating system snapshots in the live session.

Screensavers now render at up to 24 fps with a 1920×1080 backing-pixel budget,
pause when hidden and honor reduced motion. Deterministic loop tests passed
at simulated 60/75/120/144/240 Hz, including simulation timing and cleanup.
The normal desktop and games retain their output refresh rate. TypeScript
checks and the browser UI smoke suite passed after the change.

The revision 6 ISO also completed a fresh UEFI installation with both optional
bundles disabled. The installed desktop booted on the Nehalem CPU model with
zero kernel taint, no failed services, and the bundled model ready. PipeWire
detected the emulated HDA device, played a silent test stream successfully,
and its data loop ran with realtime round-robin priority 20 through RealtimeKit.
Logs: `qa-uefi-install.log`, `qa-uefi-firstboot.log`, `qa-uefi-audio.log` and
`qa-uefi-audio-playback.log`. BIOS logs use the `qa-bios-` prefix.

Steam installed its client update and reached the sign-in screen under
XWayland on the BIOS gaming install. No account was used and no real-game
performance result is claimed. The installer chroot emits Snapper's
`fatal library error, lookup self` warning during package hooks; installed
snapshots and the subsequent restore test work, but the bootstrap warning
still needs cleanup.

With the other QA guests shut down, paired 10-second Serpent screensaver
samples at 1920×1440 measured 406.7% versus 317.7% combined CPU for mindwm,
mindshell and WebKit (100% means one CPU core): about 22% less CPU time in
this software-rendered VM. The final loop uses a 24 fps ceiling, sleeps
between requested frames, bounds backing pixels and moves the clock once
per minute instead of continuously. Earlier 30 fps experiments did not
reduce CPU usage and are not counted as improvements. Evidence:
`build/logs/screensaver-ab-isolated.log`; original UI files and idle
preferences were restored after each comparison. This is a screensaver
measurement, not a game benchmark or hardware power measurement.

Minimal installations now offer an explicit “Install developer tools” action
in Settings and disable unavailable setup controls. Individual languages
remain installable separately. The browser test covers the missing-helper
state, approved package arguments and the successful transition to setup.

The development VM was upgraded to mindshell 39 and rebooted again with
kernel 7.2.3-2. The desktop returned normally, `systemctl --failed` was empty
and `mind health` reported all good (`vm-final-reboot.log`). Its existing
idle settings, output mode, autologin and performance preference remain
preserved. The two separate QA installations are retained for future tests.

The minimal UEFI installation also displayed the new Developer onboarding
correctly in native WebKit. That check found a pre-existing broken guide
link: `/usr/share/doc/mindos/DEV.md` was never packaged. Base 12 now includes
the local user guides and illustrations; the button is labelled “Developer
guide”. These files are read on demand and add no background work.

Previous image: `build/out/mindos-2026.09.07-r8-x86_64.iso` (4,530,958,336 bytes),
with a sibling SHA-256 file. Revision 8 includes base 12 and mindshell 40;
the runtime and guide changes were additionally exercised by upgrading the
installed VMs. The full fresh-install tests described above used revision 6
(UEFI) and revision 4 plus the updated local repository (BIOS). The main
`mindos-dev` VM is running with the updated Settings window; both separate
QA VMs are shut down. Source changes remain uncommitted.


## Screenshot and sharing integration

Mindwm 26, session 20 and mindshell 41 add screenshots and portal monitor
sharing. Native grim full/region captures were correct at 100% and 150%
scale and with all eight rotations/reflections at 150%; region images exactly matched
crops of the corresponding full captures. The initial vertical-orientation
bug was fixed before deployment. The PNG clipboard and Super+Shift+S area
shortcut were exercised; six workflow tests cover cancellation, failed
capture cleanup, private file modes, clipboard failure and filename collisions.

The live protocol client confirmed static damage waiting, wrong-format
rejection and one-shot frame enforcement. Locking with a queued damage
request failed it with zero extra frames; new captures while locked also
failed. All 42 compositor unit tests passed. Geometry placement now runs
after the committed buffer and window geometry refresh, removing the stale
size used by dialog centering.

The UEFI QA guest's real desktop portal returned a PipeWire stream after
Share display, and GStreamer received a PNG frame including the cursor.
Escape returned cancellation without opening a stream. The chooser follows
the session's dark GTK theme (`docs/img/screen-sharing.png`). Capture uses
shared memory today; it does not yet implement GPU-buffer transfers or the
newer ext protocol. This is not a hardware recording benchmark or a test of
specific OBS/Discord releases.

Repeated in-place compositor restarts left the QA console displaying stale
frames once, despite fresh grim captures and working IPC. A full reboot
restored the display; portal capture, queued capture locking and new dialog
rendering then passed again. The underlying restart-only cause is not yet
confirmed. Logs: `capture-tests.log`, `capture-protocol-qa.log`, `portal-qa.log`
and `portal-cancel-qa.log` under `build/logs/`.


Revision 9 image: `build/out/mindos-2026.09.07-r9-x86_64.iso`
(4,553,537,536 bytes), SHA-256
`7e4a30baf702da21b1e8aa3cb138fc54a0d5ae9d3b24f484d321d36dc89a8a21`.
It includes mindwm 26, mindshell 41, session 20 and base 13. The main VM
rebooted with these packages, returned to Settings and passed `mind health`
with no failed services (`capture-main-reboot.log`). Its preferences remain
preserved. Source changes remain uncommitted.

Revision 9 also booted from the ISO in the BIOS QA VM (Nehalem CPU, 1280×800).
The live session ran grim successfully, reported the new package versions and
passed `mind health` (`build/shots/r9-live-check.png`). Its installed disk was
not changed by this boot test. The separate QA guests were shut down after
testing; the main VM remains running. This live boot supplements the earlier
full installation tests rather than claiming another fresh install of r9.


## Graphics selection and installer bootstrap

Installer 9 checks numeric PCI display classes and NVIDIA's packaged current
support table before passwords or disk changes. It preserves the detected
AMD/Intel kernel driver and rejects legacy/unknown NVIDIA devices, including
mixed-generation systems. An explicit Mesa/Nouveau policy includes 32-bit
Vulkan without NVIDIA/CUDA packages. The current live image's table is used;
legacy proprietary branches and their gaming performance remain unverified.
See [graphics drivers](GRAPHICS.md).

Twelve fixture tests cover current, legacy, unknown, subsystem-specific,
hybrid and mixed-generation cards, stale/missing tables, PCI audio/USB
filtering, and explicit compatibility selection. A read-only scan of the
host identified its RTX 4090 and integrated AMD GPU correctly
(`build/logs/install-gpu-host.log`); no host driver or tuning was changed.
Eleven installer preflight checks still pass.

A complete minimal installation using installer 9 succeeded on a separate
24 GiB disk in the BIOS/Nehalem QA VM. The system booted with zero kernel
taint, no failed services and `mind health` reporting all good. No NVIDIA
DKMS or CUDA package was installed; the baseline snapshot was present.
The bootstrap transaction skips snap-pac until the installer creates the
root configuration and baseline. Its previous chroot warning did not occur
during the installer. A later QA-only guest-agent installation in the chroot
still emitted the upstream warning; it did not affect boot or snapshots.
Logs: `qa-graphics-install.log`, `qa-graphics-firstboot.log`. The original
BIOS QA disk was retained throughout this test.

Revision 10: `build/out/mindos-2026.09.07-r10-x86_64.iso`
(4,553,537,536 bytes), SHA-256
`cba8ce843c52d908b70de0c060289ae70d990daa3708b67c35cb7fba21b4da77`.
The image's package list confirms installer 9; its package includes the
read-only graphics helper and local graphics guide. The main VM received
the installer update. Both QA guests are shut down, and the BIOS guest's
original disk configuration was restored; the additional installation disk
is retained separately as `mindos-qa-graphics-install.qcow2`.
The fresh-install test used installer 9 from the shared package repository;
the most recent live ISO boot test used revision 9.


## Gaming input and accelerated VM follow-up

`mindwm` 0.1.0-27 uses shared native/nested pointer motion handling. Relative
motion now focuses the surface at the new position; absolute pointer devices
honour locks/confinement and produce incremental relative deltas. Pointer
bounds use actual output rectangles, including negative origins, vertical
arrangements and gaps, and safely handle disconnected outputs. Confinement
intersects surface bounds, the input region and the requested region, clips
motion across holes, and slides along boundaries. Cursor hints no longer warp
a still-locked pointer. Keyboard shortcut inhibition uses keyboard focus.
Locking the desktop cancels mouse grabs and clears mouse focus immediately.

A game startup fullscreen request can arrive before the first buffer, when
Space has not yet assigned the window to an output. Output selection now
handles that case and consistently uses the selected monitor for both its
geometry and fullscreen ownership. Moving fullscreen ownership to another
output clears the old association.

Validation: 47 Rust library tests passed, including five geometry regressions.
The portable build and live `pointer_client.c` / `test_pointer_vm.py` checks
passed on the accelerated BIOS QA VM. The live checks cover startup fullscreen,
constraint creation outside its region, later activation, two independent
absolute deltas while locked, unaccelerated PS/2 relative deltas, advisory hint
commits, confinement across an excluded strip, edge sliding, release and desktop
locking. No client motion reaches the game after the lock clears pointer focus.
Logs: `build/logs/pointer-tests.log`, `pointer-vm-tests.log`, and
`qa-pointer-*-passed.log`. The first development build deadlocked on nested
Smithay surface-state access; the final build reads state outside the constraint
callback and passes those live probes.

Xonotic 0.8.6-3 loaded Stormkeep through both native SDL Wayland and SDL X11 /
XWayland. Both report the virgl AMD renderer, and IPC confirms the respective
native/X11 window types and fullscreen state. Both clients joined the local match; movement, mouse-look and firing were
checked, including PS/2 relative motion under XWayland. Menus, fullscreen
toggling and compositor screenshots work. Logs are
`qa-xonotic-*-final.log`, with screenshots under `build/shots/qa-xonotic-*`.
This is functional game rendering/input coverage, not a frame-rate comparison,
Proton validation or a claim about native GPU latency/direct scanout.

One earlier accelerated idle run aborted QEMU: the host journal records an
AMD GPU fault, graphics-ring timeout and recovery for the QEMU process at
22:28:40 EDT on September 7; QEMU records a lost graphics context and exits
with SIGABRT. This happened on mindwm 26 before the input changes. Later pointer
and game checks completed with virgl enabled, but the host/virgl graphics fault
remains unresolved. The QA disk reopened after QEMU rebuilt its refcounts;
subsequent guest health checks passed. Keep hardware/accelerated stability
validation open.

The main VM has package 27 installed without restarting its running session;
the new compositor takes effect at its next login. Its existing preferences
were preserved.


The rebuilt image is `build/out/mindos-2026.09.07-r11-x86_64.iso`
(4,558,053,376 bytes), SHA-256
`03758c171edb0d53e3a9c120651ffb0c1d065eac6d6b823859a1717472c5337b`.
The embedded manifest confirms mindwm 27, shell 41, session 20, base 13,
installer 9 and kernel 7.2.3-2. Package 27 was booted and tested on the installed
QA system; this revision's ISO has not yet had a separate live-media boot test.

## Game tuning reliability and interface

`mindos-gaming` 0.1.0-6 replaces game DLLs through complete, flushed temporary
files on the same filesystem. It records before/after/original hashes before
publishing each file, keeps permissions and originals, rejects overlapping
writers, and reports partial multi-DLL completion. A launcher update is detected
by content; Restore refuses to overwrite it. Reapply uses the updated file as
the new original and archives the older original by hash. Changed/missing
backups, invalid history, and changed imported/downloaded library files produce
errors instead of silently resetting original tracking.

`mindshell` 0.1.0-42 adds game filtering/counts, groups tuning controls, removes
duplicate library choices, distinguishes failed scans from empty results, and
disables controls for the full command/rescan interval. Apply, Restore and
Reapply reflect current file state; scan errors remain visible and retryable.
The version picker can be closed and reopened with the keyboard. Disabled
buttons/selects are visually dimmed across Settings.

Validation: 18 isolated Python regressions passed, including disk-full staging,
record/rename/directory-sync failures, restoration recovery, launcher updates,
original archives, changed/missing backups, legacy records, malformed history,
links/path handling, multi-file partial completion and competing CLI writers.
Browser navigation, filtering, failed-scan retry, duplicate action prevention,
Apply/Restore, dialog keyboard handling and compact layout checks passed.
TypeScript checking and the production UI build passed; both packages built.
Logs: `build/logs/dlss-tests.log`, `ui-games-tests.log`, `gaming6-build.log` and
`mindshell42-build.log`.

The packaged WebKit page in the BIOS QA VM applied and restored a labelled,
synthetic 64-byte DLL fixture, then detected a simulated update and reapplied
with that update retained as the new original. The GUI's final Restore returned
version 100 (the simulated update), rather than the older version 97. These
fixtures prove file handling and interface integration, not DLSS image quality,
real-game compatibility or FPS gains. Screenshot: `docs/img/settings-games-modern.png`.
The native checks used software graphics to avoid the previously observed host
AMD/virgl reset.

The QA health check exposed another rollback lifecycle gap: snapshot 5, the
old root retained by the earlier normal-session restore test, was still writable.
The safe pruning hook correctly refused to delete it. Read-only inspection
confirmed that it was unmounted, not the default subvolume, and held only empty
`var/lib/machines` and `var/lib/portables` child subvolumes. The QA snapshot was
manually sealed read-only and cleanup retried. Automatic sealing after reboot
from a normal-session rollback remains open; the current `mindos-boot restore`
only seals immediately when running from an overlay snapshot boot.

The rebuilt image is `build/out/mindos-2026.09.07-r12-x86_64.iso`
(4,558,053,376 bytes), SHA-256
`1a1d6c1559ce382ab7b2a4a43d09dbda919f010d2bce3156660e5c46a597ecb7`.
It booted through UEFI into the live desktop at 1280×800 with no failed
system services. The live package query confirms shell 42, mindwm 27, base 13
and installer 9. The embedded local repository includes gaming 6; the gaming
bundle is optional and is not installed in the live environment. Settings ›
Games renders its missing-package state correctly there. Screenshots:
`build/shots/iso-r12-uefi-live.png`, `iso-r12-uefi-packages.png` and
`iso-r12-uefi-games.png`.

The main VM received shell 42 and the updated DLL helper, preserving its
preferences; its health check passed. The BIOS QA system's cleanup and health
checks passed after sealing the previously retained root. Test DLLs and their
temporary discovery configuration were removed after the native UI checks.

## Automatic sealing after rollback

`mindos-base` 0.1.0-14 adds a boot service that seals roots retained by
`mindos-boot restore` once they are unmounted. It runs before scheduled Snapper
cleanup, defers mounted/default/received roots, and preserves nested data and
child flags. User snapshots are not changed. The cleanup hook also checks roots
without children before allowing their deletion. Empty directories left by
Snapper after deletion are ignored by the boot scan.

All 18 disposable Btrfs integration checks passed, including sealing, a renamed
but still-mounted root, bind-mounted children, default/user roots, nested data,
symlinks and pruning failure recovery. Log: `build/logs/snapshot-seal-tests.log`.

The BIOS QA VM restored snapshot 32 while running from its normal root. The
retained root (33) stayed writable, including after manually invoking the new
service before reboot. After reboot, the normal root contained the baseline
marker; the boot journal records automatic sealing of 33. Snapper deleted that
retained root successfully. The temporary target snapshot and marker were then
removed; scheduled cleanup and the system health check passed with no failed
units. The final package also passed the service invocation after deletion had
left empty snapshot directories. Logs: `qa-rollback-seal-before.log`,
`qa-rollback-seal-after.log`, `qa-rollback-seal-cleanup-final.log` and
`qa-base14-final.log` under `build/logs`.

This closes the normal-session rollback lifecycle gap identified above for
restored systems with base 14 or later. A snapshot containing older system
packages still has those older tools and needs an upgrade for this service.
The main VM received the same final package and passed the service and health
checks without restarting its desktop (`build/logs/main-base14-install.log`).

ISO: `build/out/mindos-2026.09.07-r13-x86_64.iso` (4,571,600,896 bytes),
SHA-256 `dfdb02a764342f37f26458552d12805441cd706bca329136c366adf1cc5c1f81`.
The embedded manifest confirms base 14, shell 42, mindwm 27, installer 9 and
kernel 7.2.3-2. The image booted through UEFI into the live desktop with zero
failed services. As intended, sealing is skipped on the live image, which has
no installed `/.snapshots` directory. Screenshots:
`build/shots/iso-r13-uefi-live.png` and `iso-r13-uefi-health.png`.

## Gaming setup from Settings

Shell 43 replaces the missing-helper error with an authenticated **Install
gaming tools** action and a local guide. Installation stays busy while changing
settings pages, preventing another transaction in that window. Progress stays
visible until completion; cancellation and failures are retryable. The host
permits the fixed `mindos-gaming` repository install through pkexec.

The browser suite passes cancellation, failure/retry, duplicate clicks, leaving
and returning during installation, persistent progress, malformed helper output
and existing Games controls. TypeScript checking and the production build pass.
Native UEFI QA testing exercised the real authentication dialog, cancelled once,
then installed the gaming stack through the page. The final empty-library page
renders its controls correctly. `docs/img/settings-games-setup.png` shows the
native setup card; `build/shots/qa-gaming-setup-final.png` shows the final page.

This uncovered an older helper bug: an empty library printed human-readable
text before its JSON, so the host could not decode it. Gaming 7 emits just the
JSON array. The shell now rejects an unreadable response with a retryable error
instead of throwing while rendering. All 19 DLL regressions pass, including the
empty-library CLI response. Logs: `ui-gaming-setup.log`, `dlss-tests.log`,
`qa-gaming-setup-cancel.log`, `qa-gaming-setup-final.log` and
`qa-gaming-setup-ui.log` under `build/logs`.

The first install also pulled Arch's `linux` package through `ntsync-autoload`:
the custom kernel lacked the `NTSYNC-MODULE` provider. Kernel package 7.2.3-2.1
now advertises its existing built-in driver. This is a packaging subrelease
(supported by [PKGBUILD's version format](https://man.archlinux.org/man/PKGBUILD.5.en.html));
the kernel ABI stays `7.2.3-2-mindos`. The old and new vmlinuz hashes are identical:
`8257cfdcb3637ce68fbd5f9928801e00d2a378ca76c3e0f5ef401315f20b3d9a`.
All 5,727 module signature footers and 41 resolved configuration checks pass;
the package checker also verifies the provider and built-in driver.
The QA guest exposes `/dev/ntsync` and identifies the driver as built-in.
After resetting to a pre-install snapshot, the final GUI install completed
with gaming 7 and without the Arch `linux` package. This establishes dependency
resolution and device availability, not Wine synchronization benchmarks.
Logs: `kernel-2.1-check.log`, `qa-gaming-final-plan.log` and
`qa-gaming-setup-final.log`.

Resetting from a package snapshot exposed a copied pacman lock. Base 16 removes
that stale file only from the newly restored root, preserving the running root
and avoiding symlinked package database directories. A real UEFI test restored
post snapshot 36, which contained `db.lck`, while a labelled test lock remained
in the running root. After reboot, the copied lock was absent, automatic sealing
completed, and the subsequent GUI package install succeeded. Cleanup and health
checks passed. Logs: `qa-rollback-lock-before.log` and `qa-rollback-lock-after.log`.

The main VM received shell 43, base 16, the corrected helper and kernel packaging
update, and passed its health check without restarting the desktop. Its existing
gaming-helper installation is still managed directly rather than by installing
the gaming metapackage. Logs: `main-gaming-setup-update.log` and
`main-base16-install.log`.

Final image: `build/out/mindos-2026.09.07-r15-x86_64.iso`
(4,761,911,296 bytes), SHA-256
`b64bcb6fd05a002d733a46fab71eebabb1d60036f0bb82b0297ac249397fff70`.
Its manifest confirms kernel package 7.2.3-2.1, base 16, shell 43, mindwm 27
and installer 9. It booted through UEFI with no failed services, and the new
Games setup card rendered correctly at 1280×800. Screenshots:
`build/shots/iso-r15-uefi-health.png` and `iso-r15-games-setup.png`.
The gaming installation itself was validated on the installed QA system;
the live image does not preinstall the gaming bundle.

## Smaller release repository

`scripts/build-repo.py` selects the newest package for each name using pacman's
`vercmp`, including epoch and decimal package-release ordering. Only x86-64 and
architecture-independent packages are included. Historical archives remain in
`build/packages`; package signatures, when present, accompany the selected file.
Indexing happens in a temporary directory, followed by an atomic Linux directory
exchange. A failed build leaves the existing repository intact. `make iso` now
refreshes this repository automatically before staging the image.

Six integration checks with real `repo-add`/`vercmp` passed: epoch/subrelease and
architecture selection, source-archive retention, successful replacement, failed
indexing, corrupt archives, duplicate identities and empty input. The current
repository has 14 packages with 195,379,234 archive bytes; 426,520,494 bytes of
historical archives are kept outside the release repository. Logs:
`build/logs/repo-tests.log` and `repo-compact-final.log`.

Mind package 16 refreshes native repository metadata and applies pending system
updates with the installation (`pacman -Syu --needed`). Repository-only requests
install all missing requested packages in one transaction. Requests for already
installed packages make no changes. Five isolated helper tests passed for
batching, validation, full upgrades, failure handling and installed-package
no-ops. Failed repository-only requests never fall through to Flathub or the AUR.

The BIOS QA VM initially cached base 13, whose archive was no longer in the
compact repository. The new helper installed ripgrep and fd in one transaction,
updated to base 17/Mind 16/kernel packaging 2.1, and passed health checks with no
failed services. Repeating the request reported both tools as installed without
another transaction. Logs: `pkg-repo-tests.log`, `qa-repo-before.log`,
`qa-repo-install.log`, `qa-repo-install-health.log` and
`qa-repo-install-repeat.log`. The main VM received the helper and documentation
packages and passed health checks (`main-repo-update.log`).

Image: `build/out/mindos-2026.09.08-r16-x86_64.iso`, 4,350,066,688 bytes,
SHA-256 `43d9443a02e87a85e64e315944b21e245748b15b27212a73422cd2ce1ddc76a3`.
It is 411,844,608 bytes (8.65%) smaller than r15. The two manifests contain the
same 765 package names; version differences are base 16→17, Mind 15→16 and the
upstream llama-cpp package 0.4.0-1→0.4.0-2. The live image booted through UEFI,
reported no failed services, and contains exactly 14 local package archives.
Screenshots: `build/shots/iso-r16-uefi-live.png` and `iso-r16-health-mind.png`.

CPU-only first-response latency remains open. A simple request on the two-core
Nehalem BIOS QA VM was still processing after five minutes; its model server
was the old executable retained across the package update. The slot reported
2,048 prompt tokens and no decoded tokens at that point, with CPU activity and
no memory/swap pressure. A fresh live r16 session also started a request without
an immediate response. These observations do not establish a regression in the
new llama-cpp package, or a successful inference check. Both QA sessions are
left running for the response-time follow-up.

## CPU model acceleration and cancellation

Mind package 17 adds `ggml-blas` (OpenBLAS) alongside Vulkan. A profiler
attached to the two-core Nehalem BIOS QA VM found both inference threads
in the generic SSE4.2 quantized dot-product implementation during the original
long first request. With BLAS installed, the same model used OpenBLAS SGEMM.
A cold request completed prompt evaluation in 159.24 seconds for 4,514 tokens
(28.35 tokens/s), and a repeat request returned `ready`. The live r16 request
without BLAS reached its 600-second timeout. These whole-request observations
also differ in server lifetime and cache state, so they are not a controlled
speed ratio. Logs: `qa-llama-backtrace.log`, `qa-blas-backtrace.log`,
`qa-blas-timings.log`, `qa-blas-response-cached.log`; live result screenshot:
`build/shots/iso-r16-mind-result.png`.

A separate controlled `llama-bench` comparison used the same installed
llama-cpp 0.4.0-2 / ggml 0.23.0-2, Qwen3.5-2B-Q4_K_M model and two CPU threads,
with zero GPU layers. Each case ran two repetitions of 128 prompt tokens and
eight generated tokens, without warmup. The scalar case hid other backends
inside a private mount namespace; installed files and the running daemon were
unaffected. Mean prompt throughput rose from 9.52 to 34.81 tokens/s (3.66×);
generation remained 6.44 versus 6.45 tokens/s. This is a small CPU microbenchmark,
not a gaming FPS claim or a prediction for other hardware. Logs:
`qa-bench-scalar.log` and `qa-bench-blas.log`.

The HTTP client now checks cancellation while waiting for headers, response
chunks and error bodies, dropping the stream when cancelled. It finishes on
the SSE `[DONE]` marker even if the server keeps the socket open. All 20 Mind
Rust tests passed, including four local HTTP peer regressions for these paths
(`mind-cancellation-tests.log`). The installed Mind 17 package acknowledged
cancellation during real model processing in 0.003 seconds; the same client
connection then answered Status successfully, the model server logged the
cancellation, and systemd reported no failed services. The server may finish
its current compute batch before releasing the slot. Logs:
`qa-mind17-cancel.log`, `qa-mind17-cancel-server.log`, `qa-mind17-install.log`.
Base package 18 includes the updated local performance guide.

The main VM also received Mind 17, base 18 and the two BLAS packages and passed
its service/status checks. Its existing desktop and daemon process were kept
running, preserving the selected 4B model and preferences; the updated daemon
code takes effect on its next service start. Logs: `main-mind17-install.log`
and `main-mind17-health.log`.

Image: `build/out/mindos-2026.09.08-r17-x86_64.iso`, 4,358,582,272 bytes,
SHA-256 `c84c42019330a95b7be6b1f5a1d2462eb36935a212a10c54d4ca73944bfa11ff`.
The fresh UEFI live session contains Mind 17/base 18/ggml-blas 0.23.0-2/
OpenBLAS 0.3.34-1, reports no failed services, and answers the first `ready`
request successfully in 135.021 seconds. Its 4,425-token prompt evaluated at
32.83 tokens/s. This remains slow on the old two-core CPU, but now completes
within the normal request timeout without manual package changes. Log:
`iso-r17-live-inference.log`; screenshot: `build/shots/iso-r17-live-validated.png`.
The image adds about 8.5 MB to r16 and remains about 403 MB smaller than r15.

Both QA guests were shut down after validation, and the UEFI guest's normal
installed-disk boot configuration was restored. The main VM remains running.

## Suspend lock ordering

Shell 44 replaces the sleep watcher's fire-and-forget lock request with a
dedicated asynchronous compositor connection. It holds logind's delay inhibitor
until both lock and display-blanking acknowledgements arrive. The attempt has
a four-second deadline, logs failures and respects the sleep-lock preference.
On resume it reconfirms the lock and wakes the displays without requesting an
unlock. Base 19 includes the updated local shell guide.

All 34 shell Rust tests passed, including five sleep-watcher tests. Real Unix
socket peers establish that the inhibitor descriptor stays open through both
acknowledgements, a stalled peer times out, rejected/unconfirmed replies fail,
the disabled preference avoids compositor requests, and resume never unlocks.
Log: `shell44-build.log`. The package's ownership and key executable/UI modes
were checked after a fakeroot diagnostic; all archive entries are owned by
root and the binary/UI modes are 0755/0644.

Installed BIOS QA testing confirms that logind invokes the new path, the
compositor locks and blanks before suspension, and the watcher reacquires its
inhibitor and retains the lock after resume. However, full visual resume is
**not passed**: the virtio display remains inactive. S3 also produced QEMU
WATCHDOG/RESET events. A control run with `lock_on_sleep=false` reproduced the
inactive display, and enabling QEMU's `x-pcie-pm-no-soft-reset` property did
not resolve it. This resembles the upstream
[virtio GPU S3 report](https://gitlab.com/qemu-project/qemu/-/issues/2520); the
precise driver/firmware cause is not established here.

The QA configuration temporarily enabled S3 and omitted its virtiofs workspace
device, which otherwise refuses suspend. A separate suspend-to-idle test
returned with the same boot ID after an emulated power-button wake, retained
the lock and reacquired the inhibitor, but also failed visual restoration.
The RTC alarm did not wake that test as expected. Failed guest shutdowns after
these experiments required test-VM power/reset recovery. No sleep-mode or
QEMU-device changes were made to the distro defaults or the main VM.
Logs: `qa-suspend-resume.log`, `qa-suspend-compositor.log`,
`qa-suspend-qemu-events.log`, `qa-suspend-control-resumed.log`,
`qa-s2idle-before.log`, `qa-s2idle-resumed.log`, `qa-s2idle-full.log`.

After restoring the QA guest's original device configuration and cold booting,
ordinary lock → blank → wake returned a visible password screen. Entering the
test account's password unlocked it successfully, and service checks passed.
The saved compositor preferences compare exactly equal before and after all
experiments, and the default memory-sleep selection is restored. Screenshot:
`build/shots/qa-lock-blank-wake.png`; logs: `qa-lock-blank-wake.log`,
`qa-lock-wake-health.log`, `qa-suspend-prefs-before.log`,
`qa-suspend-prefs-after.log`.

Image: `build/out/mindos-2026.09.08-r18-x86_64.iso`, 4,358,582,272 bytes,
SHA-256 `fd25fc3a4d598fc905493fb6e8c6be6f967ffb79c03a4dfb9f31b6242ec55805`.
The live UEFI session booted with shell 44/base 19, no failed services and
the MindOS sleep delay inhibitor present (`build/shots/iso-r18-health.png`).
The main VM received shell 44/base 19 and passed service checks without
restarting its existing desktop; its running shell picks up the change on
the next session start. Logs: `main-shell44-install.log`,
`main-shell44-health.log`.

Both QA guests are shut down with their normal boot/device configurations
restored. The main VM remains running; no host sleep or GPU changes were made.

## Standard application autostart and session cleanup

Session 21 activates the graphical-session and XDG autostart targets after
publishing the compositor environment. It uses systemd's existing
[XDG autostart generator](https://github.com/systemd/systemd/blob/main/man/systemd-xdg-autostart-generator.xml)
to handle desktop entries, including user overrides and desktop exclusions.
The configuration search path is set through `environment.d`: importing a
client environment alone does not set the generator's lookup environment.
The network applet now uses a single standard desktop entry with `--indicator`.
The session wrapper stops managed applications on compositor exit or termination.

Two isolated wrapper tests pass, covering compositor exit status, forwarding
termination and target cleanup (`python3 scripts/tests/test_session_lifecycle.py`,
`build/logs/session-lifecycle-tests.log`). Installed BIOS QA login tests use a
real long-running probe and desktop entries in the system and user directories:

- One probe starts per login with the MindOS/Wayland environment.
- A user `Hidden=true` override suppresses the system entry; missing `TryExec`
  and GNOME-only entries do not run.
- The network applet starts once with `--indicator` from the MindOS override.
- Logout and session termination stop the probe and session targets even with
  lingering temporarily enabled. Normal compositor exit can leave GUI units
  marked failed after their Wayland connection closes; the next login starts
  them successfully and reports no failed user services.
- Test desktop entries are removed, lingering is restored to `no`, and a
  subsequent ordinary login has no failed system or user services.

Logs: `qa-autostart-second-login.log`, `qa-autostart-selection.log`,
`qa-autostart-termination.log`, `qa-session21-base21-health.log`.
Screenshot: `build/shots/qa-autostart-desktop.png`. Base 21 includes the local
guide for enabling, disabling and diagnosing startup applications.

## Concurrent boot-menu updates

A native package transaction exposed a race between pacman and snapshot boot
hooks: both used `/boot/limine.conf.new`, and one failed its final rename. Base
21 serializes all boot-menu and kernel-store mutations with a process-held
`flock`, including manual commands. Read-only status/list commands remain free
to run. This also protects the shared temporary directory used for kernel copies.

Two isolated tests pass (`python3 scripts/tests/test_boot_concurrency.py`):
configuration and snapshot writers wait for an existing lock, and 16 simultaneous
writers all succeed with a complete final menu. Eight concurrent real `config`
and `sync` commands on the installed BIOS QA system also succeed; the package
transaction completes without the previous rename error.
Logs: `boot-concurrency-tests.log`, `qa-base21-install.log`,
`qa-session21-base21-health.log`.

The installed BIOS guest rebooted successfully afterward with no failed system
or user services (`qa-base21-reboot-health.log`,
`build/shots/qa-session21-base21-reboot.png`). The main VM received session 21
and base 21 without restarting its running desktop or selected 4B model; its
service checks pass and performance mode is retained. Logs:
`main-session21-base21-install.log`, `main-session21-base21-health.log`.

Image: `build/out/mindos-2026.09.08-r20-x86_64.iso`, 4,358,582,272 bytes,
SHA-256 `52bb6013ea52471bb028b5bc5087337a77b1349b4b85979e27a871ea4a6587d8`.
It supersedes the intermediate r19 image, which lacks the boot-writer lock.
The fresh UEFI live session contains session 21/base 21, has no failed system
or user services, activates all graphical-session targets, and runs exactly
one network applet with the indicator flag. The sleep delay inhibitor is also
present. Log: `iso-r20-live-health.log`; screenshot:
`build/shots/iso-r20-health.png`.

Both QA guests are shut down after testing and retain their normal installed
boot configurations. The main VM remains running. Full visual suspend/resume
and real-hardware gaming performance remain open as documented above.

## Keyboard and mouse preferences

Compositor 28 and shell 45 add Settings → Keyboard & mouse. The page provides
layout presets/custom XKB names, repeat rate and delay, flat/adaptive/default
mouse acceleration, speed, handedness and scroll direction. Apply confirms the
compositor's reply; discarded edits and staged defaults do not change the active
settings. Save/error feedback stays visible when applying at the page bottom.
The page runs no recurring input poll. Base 22 carries the updated local guide.

All 49 compositor and 34 shell Rust tests pass, along with the TypeScript check
and browser suite. New input tests reject invalid values/unknown fields and
preserve unrelated input preferences during partial updates. Browser coverage
checks apply, staged defaults, discard, write rejection, reopening, compact
width and visible feedback. Logs: `input-compositor-tests.log`,
`shell45-build.log`, `input-ui-smoke.log`; screenshots:
`build/shots/ui-smoke/input.png`, `build/shots/ui-smoke/input-compact.png`.

Installed BIOS QA evidence:

- A Wayland protocol client initially receives physical key 21 as `y`; after
  selecting German it receives `z`. Repeat announcements change from 25/200
  to 37/425 (keys per second / milliseconds).
- Invalid layout, out-of-range speed/repeat and unknown settings are rejected;
  comparison confirms the previous preferences remain exactly intact.
- The real libinput mouse reports flat acceleration and speed 0.35. An injected
  relative delta of 100 produces accelerated motion 135 and raw motion 100.
  This checks preservation of unaccelerated motion, not game FPS or latency.
- A temporarily attached USB mouse inherits the saved settings, including
  handedness and scroll direction. It is detached afterward.
- Preferences compare equal across a full guest reboot. The original complete
  preference object is restored after testing.
- A mouse/keyboard-driven native Settings interaction changes repeat rate to
  40, confirmed through the compositor; that test change is restored too.

Logs: `qa-input-settings.log`, `qa-input-probe.log`, `qa-input-hotplug.log`,
`qa-input-persistence.log`, `qa-input-restored.log`,
`qa-input-native-ui-apply.log`. Screenshots: `build/shots/settings-input-native.png`
and `build/shots/settings-input-native-bottom.png`.

The flat-profile behavior follows
[libinput's pointer acceleration model](https://wayland.freedesktop.org/libinput/doc/latest/pointer-acceleration.html).
Touchpad gestures/tapping, physical multi-keyboard setups, and real gaming mice
still need hardware coverage. These per-user settings do not change the login
screen's system keyboard environment.

Image: `build/out/mindos-2026.09.08-r23-x86_64.iso`, 4,358,582,272 bytes,
SHA-256 `9d7a0624c33f9d9d4939d82ab3767560cb9ab4798aa4e06126f6927216e77c6e`.
The fresh UEFI live session boots with compositor 28/shell 45/base 22, no failed
system or user services, active session targets and the expected input-device
state. Its Input page renders correctly; the packaged guide and final opaque
save banner are present. Log: `iso-r23-live-health.log`; screenshot:
`build/shots/iso-r23-input.png`. The installed QA screenshot
`build/shots/settings-input-feedback-final.png` verifies readable confirmation
while the form is scrolled down.

The main VM received the final packages without replacing its running desktop,
Settings window or 4B model processes. The new Input feature becomes available
there after starting a new desktop session. Final service checks pass in both
the main and installed QA guests (`main-input-final-health.log`,
`qa-input-final-health.log`). Test preferences were restored, the temporary USB
mouse was removed, and both QA guests were shut down with their normal boot
configurations restored. The main VM remains running.

## Prebuilt NVIDIA modules and installation preflight

`linux-mindos-nvidia-open` 610.57.04-2 contains all five open NVIDIA modules,
built with Clang for `7.2.3-2-mindos` and signed after stripping with the
released kernel's build key. It pins `linux-mindos=7.2.3-2.1` and
`nvidia-utils=610.57.04`. The 9,348,962-byte package contains only compressed
modules and NVIDIA's license; no signing key is shipped. `make nvidia` now
performs the artifact checks automatically.

The verifier cryptographically checks each SHA-512 CMS signature and the
released kernel's virtio GPU module using the same public certificate. It
also checks ELF relocations, ABI, driver version and dependencies. The first
candidate inherited application compiler flags and failed native loading
with unsupported relocation 41; clearing those flags and rebuilding fixed
it. The new verifier rejects that earlier candidate. Only release 2 is staged
in the final repository/image. Logs: `nvidia-prebuilt-build.log`,
`nvidia-rejected-candidate-check.log`.

In the disposable installed QA guest, enabling module signature enforcement
causes a deliberately corrupted NVIDIA signature to fail with “Key was rejected
by service”. The valid module, after loading its declared `aead` dependency,
reaches NVIDIA initialization and returns “No NVIDIA GPU found.” Kernel taint
is 4096 (out-of-tree module), with no unsigned-module taint. This verifies
kernel acceptance, not GPU rendering, Secure Boot, FPS or physical-device
compatibility. Log: `qa-nvidia-signatures.log`.

Installer 10 resolves the entire proposed installation using an empty temporary
pacman database before formatting. It prefers prebuilt modules and falls back
to DKMS plus matching headers/Clang/LLVM/LLD when dependencies cannot resolve;
an explicit provider choice can require either path. There are 31 passing
isolated installer tests and 3 passing real-pacman tests against a disposable
repository with deliberately mismatched versions. Both complete gaming/developer
selections resolve against the real repositories. Logs:
`install-packages-real-tests.log`, `nvidia-real-resolution.log`,
`nvidia-real-dkms-resolution.log`.

Base 23 no longer requires kernel headers. Developer 5 retains them alongside
the full compiler stack. A fresh minimal Mesa installation onto a separate
24 GiB QA disk completes, creates snapshots, boots into the desktop and reports
zero failed system/user units. NVIDIA modules, DKMS, headers and Clang/LLVM/LLD
are absent from that installation. The original QA disk and preferences are
preserved. Logs: `qa-prebuilt-install.log`, `qa-prebuilt-installed-health.log`;
screenshot: `build/shots/qa-prebuilt-settings.png`.

Image: `build/out/mindos-2026.09.08-r24-x86_64.iso`, 4,180,836,352 bytes,
SHA-256 `57891774fbb99d3d7bab2e9d3e3728ace889e7559cbff6e87d30012b1e6e39b1`.
Compared with r23 it saves 177,745,920 bytes (169.5 MiB, 4.08%). Its fresh UEFI
live desktop has zero failed system/user units. All five prebuilt modules have
the expected version, ABI and signing key; DKMS, headers and Clang/LLVM/LLD
are absent. Real package preflight succeeds for Mesa and prebuilt NVIDIA
selections. Log: `iso-r24-live-health.log`; screenshot:
`build/shots/iso-r24-settings.png`.

The main VM receives base 23/developer 5/installer 10 while retaining its
existing NVIDIA provider and the same compositor, shell, Settings and selected
4B model processes. System/user service checks pass and performance mode stays
active (`main-prebuilt-final-health.log`). Both QA guests are shut down and
their original installed boot configurations restored after testing.

## Recent-window switching and fullscreen visibility

Compositor 29 adds stable recent-use window switching while Alt/Super is held,
reverse cycling with Shift, Escape cancellation and Alt+F4. Releasing the
modifier commits the selected window so a quick second Alt+Tab returns to the
previous app. The cycle includes visible windows; minimized windows remain in
the dock. The implementation retains window IDs and updates on focus/key events,
with no idle polling or thumbnail capture. Caps Lock no longer changes Super
shortcut meanings. Consumed key releases are tracked by physical keycode,
including when an exclusive layer takes focus or Shift is released first.

Focus changes now update the fullscreen rendering selection for the selected
window's output. Switching away from a fullscreen game reveals the chosen app
without changing the game's fullscreen state; returning restores the fullscreen
render path. XWayland windows are raised through both the compositor and XWM.
The new Settings → Desktop shortcut list and local shell guide ship in shell 46
and base 24.

All 54 compositor unit tests and the TypeScript check pass. The installed BIOS
QA guest logs in through the graphical greeter and runs the real protocol peers:

- Three Wayland windows maintain a stable held cycle; Shift reverses it,
  Escape returns to the original, and quick Alt+Tab switches back to the previous
  app. None receives the consumed Tab release when Shift is released first.
- Caps Lock + Super+M maximizes the expected window. Alt+F4 produces the native
  close request and the client exits.
- Wayland and XWayland fullscreen peers switch to a normal app and back.
  Screenshots compare the actual center pixel with each peer's distinct color;
  the fullscreen flags remain set while the other app is visible. The XWayland
  peer receives `WM_DELETE_WINDOW` on Alt+F4.
- A client using the shortcuts-inhibit protocol receives both press/release
  events for Alt+F4 and Alt+Tab and remains focused/open.
- Existing native pointer lock, raw relative motion, confinement, release and
  session-lock isolation checks still pass.

Logs: `shortcut-compositor-tests.log`, `qa-keyboard-tests.log`,
`qa-wm29-pointer-tests.log`, `qa-shortcuts-final-health.log`. Screenshots:
`build/shots/qa-keyboard-away-from-game.png`,
`build/shots/qa-keyboard-x11-away-from-game.png`,
`build/shots/qa-shortcuts-settings.png`. QA preferences compare exactly with the
saved original after the tests; all probes close and no system/user units fail.
These are functional virtual-device checks, not physical game/FPS/latency
benchmarks or multi-monitor hardware coverage.

Image: `build/out/mindos-2026.09.08-r25-x86_64.iso`, 4,180,836,352 bytes,
SHA-256 `dc9deff29d9f65f3ad1b5afd0ae4c79ab11738db9c880586dafd09c0b39babc4`.
The fresh UEFI live desktop has compositor 29/shell 46/base 24, all session
targets active and no failed services. The guide is present, Settings opens,
and Alt+F4 closes it and returns keyboard focus to the terminal. Logs:
`iso-r25-live-health.log`, `iso-r25-close-check.log`.

The main VM receives the package files with its preference hash, compositor,
shell, Settings and selected model process IDs unchanged; performance mode and
service checks remain healthy (`main-shortcuts-final-health.log`). The new
compositor behavior becomes available there at the next desktop session.
Both QA guests are shut down and retain their original installed boot
configurations after validation.

## Hardware media keys and fullscreen feedback

Compositor 30 adds volume, output/microphone mute, brightness and MPRIS playback
keys. One sleeping worker serializes external commands with execution/output
limits; the render thread only handles key transitions and cached feedback.
Held volume/brightness repeat after 400 ms, then every 80 ms after completion.
Release, session lock, keyboard removal and VT/session pause cancel repetition.
The dark card uses the existing fonts, stays above fullscreen content without
taking focus, and disappears after 1.8 seconds. Shell 47 lists the controls in
Settings → Desktop; session 22 adds playerctl and base 25 includes the guide.

All 58 compositor tests and the TypeScript check pass. Tests bound failing,
oversized and descendant-held command output and verify the audio/backlight
arguments. The installed BIOS QA desktop exercises real kernel input events:

- PipeWire output changes by 5%, held keys repeat and stop on release, keyboard
  volume caps at 100%, and a volume adjustment unmutes the output.
- A temporary PipeWire source verifies independent microphone mute/unmute.
- The card appears over a fullscreen Wayland peer, preserves keyboard focus
  and expires; screenshots check both the peer and overlay pixels.
- A shortcut-inhibited client receives both media key transitions without
  changing system volume. Celluloid responds to play/pause/resume/stop through
  MPRIS. Locking blocks media controls and cancels held repetition; unplugging
  the test keyboard also stops repetition.
- The guest has no backlight: the real brightness key displays “Unavailable”.
  Physical backlight adjustment, multi-monitor placement and GPU/FPS effects
  remain unverified; these virtual-device checks are functional coverage.

`scripts/tests/test_media_vm.py` uses the temporary guest-only uinput device
from `media_input.py`, restores audio and verifies the unchanged preferences.
It removes its player, virtual source, input device and silent audio file.
Logs: `media-compositor-tests.log`, `media-ui-check.log`, `qa-media-tests.log`.
Screenshots: `build/shots/qa-media-volume-fullscreen.png`,
`qa-media-expired.png`, `qa-media-brightness-unavailable.png` and
`qa-media-locked.png`. The main VM receives the packages while keeping its
compositor/shell/Settings/model PIDs, preference hash and performance mode
(`main-media-final-health.log`). The new compositor starts at its next session.

The existing native keyboard/fullscreen and pointer-lock/confine/session-lock
suites also pass (`qa-wm30-keyboard-tests.log`, `qa-wm30-pointer-tests.log`).
Image: `build/out/mindos-2026.09.08-r26-x86_64.iso`, 4,180,836,352 bytes,
SHA-256 `695f44ba489b55a213e274930230b87a7a0827ec878590c91a4ee60325fbb92b`.
A fresh UEFI live boot contains all four updated packages plus playerctl;
session targets are active and system/user failed-unit lists are empty.
A native volume key changes 40% to 45% and the card is visible over Settings
(`iso-r26-live-health.log`, `iso-r26-volume-check.log`,
`build/shots/iso-r26-media.png`). Normal VT2 → VT1 return with existing
applications restores the desktop.

At r26, one live-test edge case remained open (resolved for new defaults below): starting Settings from root's text
console through the user service manager while the graphical session is
inactive allows that process to acquire DRM master on this software-rendered
virtio guest. The compositor then reports page-flip permission errors on
return. `/sys/kernel/debug/dri/*/clients` identifies the Settings process as
master; closing that instance and returning to VT1 restores rendering without
restarting the compositor. Launching Settings from the active desktop avoids
the failure, including a subsequent console round trip. This is not a fix for
inactive-session GUI launches or the earlier full suspend/resume issue.
Evidence: `iso-r26-wm.log`, `iso-r26-drm-clients.log`, `iso-r26-logind.log`.
Both QA guests are shut down with their installed boot configuration retained;
the main VM remains running.

## Active-session graphics access

Installer 11 and the live-user hook stop granting blanket `video` membership.
The compositor already obtains its primary DRM devices through libseat/logind;
logind grants direct device access to the active user and removes that ACL when
the session becomes inactive. Render nodes remain accessible. This prevents
an inactive application from becoming DRM master and blocking display return.
Existing users' group memberships are not changed by package updates. Base 26
and the installer graphics guide explain how to adopt the new default.

`scripts/tests/test_session_graphics_vm.py` reproduces the r26 failure in the
installed BIOS guest: with its original video group, inactive Settings becomes
DRM master and the expected fullscreen pixels do not return. After removing
only that group and rebooting the QA guest, the same test passes. Settings maps
while inactive, never becomes master, and both the fullscreen peer and Settings
render on return. The compositor PID and preference hash stay unchanged across
the test. DRM client lists and screenshots supply direct evidence:
`qa-session-graphics-before.log`, `qa-session-graphics-after.log`,
`qa-session-graphics-clients-0.log`, `qa-session-graphics-clients-1.log`,
`build/shots/qa-session-graphics-restored.png`,
`build/shots/qa-session-graphics-settings.png`.

Without video membership, the native media, Wayland/XWayland keyboard/fullscreen
and pointer lock/confinement/session-lock suites all pass:
`qa-session-media-tests.log`, `qa-session-keyboard-tests.log`,
`qa-session-pointer-tests.log`. Installer checks report 31 passes and three
real-pacman tests skipped on this host; the latter are unrelated to this group
change (`session-installer-tests.log`). Shell syntax checks pass. The packaged
brightnessctl links libsystemd and uses the logind API; there are no group-based
brightness udev rules in its payload. Physical GPU/backlight and full visual
suspend/resume remain separate hardware gates.

Image: `build/out/mindos-2026.09.08-r27-x86_64.iso`, 4,180,836,352 bytes,
SHA-256 `dff155a89cbe0bcf79cdad8f8ae747f9de6e84977c0b0fc77789354c9273adbe`.
The fresh UEFI live user has the new groups. Starting Settings from root's
inactive graphical-session console does not acquire DRM master; returning to
VT1 renders the complete Settings window. All session targets are active and
no system/user units fail (`iso-r27-live-health.log`,
`build/shots/iso-r27-inactive-settings.png`). This resolves the inactive-launch
failure recorded under r26 for the new defaults.

The main VM receives installer 11/base 26 while its compositor, shell, Settings,
model PIDs, groups, preference hash and performance mode remain unchanged
(`main-session-final-health.log`). The QA account's original group membership
is restored exactly after testing, and both QA guests are shut down with their
normal installed boot configuration restored. The main VM remains running.

## Virtio suspend diagnosis and visual recovery

A new test with r27's user-access settings confirms that removing `video`
membership does not repair the old root-bus virtio suspend failure. The guest
retains the same boot ID/compositor and the lock after suspend-to-idle, but the
QEMU display remains black. The kernel reports an active 1920×1080 CRTC while
QEMU falls back to a black 1024×768 output. DRM permission errors are absent.
The RTC alarm still does not wake this guest; an emulated power-button event
resumes it. The broken display also prevents a clean shutdown in that run,
requiring a forced stop of the disposable QA guest after shutdown stalled.
Logs: `qa-sleep-access-before.log`, `qa-sleep-access-resumed.log`,
`qa-sleep-access-drm-state.log`, `qa-sleep-access-full.log`.

The kernel's virtio PCI suspend code checks the PCI PM No_Soft_Reset bit before
resetting the device; the virtio GPU driver has no freeze/restore callbacks to
recreate its resources. The previous no-reset experiment was ineffective:
QEMU reported the property enabled, but `setpci` found **no PM capability** on
the root-bus VGA device. QEMU only creates that capability behind a PCIe root
port. Moving the GPU behind the port freed by removing virtiofs exposes PMCSR
`0008`, and the device survives suspend-to-idle. These details come from the
released kernel source and the matching behavior of the running QEMU device;
upstream references are linked in `docs/DEV-VM.md`.

The reproducible fixture generator `scripts/tests/prepare_suspend_vm.py` writes
an alternate QA-only XML and preserves the original. The native test
`scripts/tests/test_suspend_vm.py` verifies real kernel suspend entry/exit,
unchanged boot/compositor/application identities, visible lock restoration,
password unlock and a surviving terminal that accepts a new command. It
restores the selected sleep mode and checks that preferences are unchanged.
The original kernel 7.2.3-2.1, compositor 30 and shell 47 need no code changes.

Suspend-to-idle passes, including visible 1920×1080 lock recovery and terminal
survival. Logs: `qa-sleep-pcie-capabilities.log`,
`qa-suspend-s2idle-tests.log`, `qa-suspend-s2idle-journal.log`. Screenshots:
`build/shots/qa-suspend-s2idle-locked.png` and
`build/shots/qa-suspend-s2idle-unlocked.png`.

S3 deep sleep also restores the display, accepts the password and preserves
the application initially. However, the extended observation detects a delayed
VM reset. It is **not an end-to-end S3 pass**. The previous short-run success
was insufficient, and the reusable test now waits another 55 seconds to detect
this failure. The Q35 guest exposes an iTCO watchdog despite systemd's runtime
watchdog being disabled; this is investigated separately from the display
failure. Screenshots `qa-suspend-deep-locked.png` and
`qa-suspend-deep-unlocked.png` establish only the initial recovery.

No physical GPU, host sleep policy, main-VM process, or distro sleep default
was changed. The existing r27 image remains the tested release artifact;
this pass adds diagnostic tooling and narrows the remaining validation gap.
Physical GPU/laptop sleep, VRR and Steam/Proton performance still need a
dedicated hardware test environment.

The extended S3 run records QEMU `WATCHDOG {"action":"reset"}` about
47 seconds after wake, followed by RESET and a new boot. The guest reports
its watchdog inactive and systemd's RuntimeWatchdogUSec is zero, so this is
not the normal runtime-watchdog policy being exercised. The exact firmware/
emulation trigger remains unresolved. Logs: `qa-suspend-deep-tests.log`,
`qa-suspend-deep-confirm-events.log`, `qa-suspend-deep-journal.log`. The test
now includes a 55-second observation margin and explicitly fails if the guest
agent disappears, the boot changes, or the compositor restarts.

After testing, the QA account's original groups and exact preference hash are
restored. The original XML boots again with virtiofs mounted, the original
sleep capability selection, and no failed system/user units
(`qa-suspend-restored-health.log`). Both QA guests are then shut down. The main
VM remains running with its original compositor/shell/model PIDs, preference
hash, performance mode and healthy services (`main-suspend-audit-health.log`).

## Physical SSD installation

The user selected the Samsung 980 PRO 1 TB, serial `S5P2NG0R743731F`, and
explicitly confirmed erasing its unused 326 GiB Steam library after a read-only
inspection. r27 was used to install MindOS with gaming and developer bundles,
the NVIDIA 610.57.04-2 prebuilt module package, and early KMS for both the
RTX 4090 and AMD integrated graphics. The install uses hostname `mindos`,
timezone `America/New_York`, and user `morvoso` with the existing CachyOS
login credential. The credential was transferred locally in hashed form and
verified without printing it; root stays locked and autologin stays off.

The SSD now has a 1 MiB BIOS boot partition, a 1 GiB FAT EFI partition and
a 930.5 GiB Btrfs partition with separate root/home/log/cache/snapshot
subvolumes. An initial recovery snapshot and boot files are present. The
extracted live environment needed its udev database and package keyring
initialized; installation then resumed using the downloaded packages.
Firmware writes were kept separate from the installer to preserve boot order.

The existing CachyOS Limine configuration was backed up, then received one
appended entry, **MindOS (Samsung 980 PRO)**, which chainloads the new SSD's
own EFI loader by partition GUID. Every previous entry and default setting
remains byte-for-byte intact. The installed host entry tool parses the new
menu successfully. A separate firmware entry, Boot0000, was added after the
existing Boot0003 and Boot0004 entries; the boot order is `0003,0004,0000`.
The other two SSD partition tables and CachyOS EFI binaries match their
pre-install backups.

A transient UEFI QA guest booted the actual installed SSD through a disposable
QCOW2 overlay and reached the `morvoso` login screen. QEMU explicitly reported
the physical SSD backing node read-only. This verifies the installed boot
path and greeter, not NVIDIA rendering or physical gaming performance.
The guest shut down normally; its overlay, temporary firmware and installer
environment were removed, and the SSD's original device ownership restored.
All SSD filesystems are unmounted and the host has not been rebooted.

Evidence: `build/logs/hardware-install-980-finish.log`,
`hardware-installed-checks.log`, `hardware-boot-overlay-nodes.json`,
`hardware-host-menu-tree.log`, `hardware-boot-menu.log`, and
`hardware-firmware-entry.log`. Screenshot:
`build/shots/hardware-ssd-firstboot.png`. Boot configuration and partition-table
backups are under the root-readable `build/hardware-install/backup/` directory.
To start hardware testing, reboot and select **MindOS (Samsung 980 PRO)**;
sign in as `morvoso` using the existing CachyOS password.

## Gaming desktop and compatibility fixes — 8 September 2026

This subsequent pass builds compositor 31, shell 48, base 27, gaming 8,
Mind 18, apps 4 and installer 12, plus Octopi 0.19.0-1 and qt-sudo 2.4.1-1.
These are package artifacts in `build/packages` and the local package repository;
the r27 ISO and physical SSD described above have not been updated by this pass.

The Mind popup now selects one output at invocation, loads application metadata
and icons off the rendering thread, reuses idle render buffers and uses an
analytic shadow instead of the expensive blur. Empty search shows clickable
gaming/software quick launches with icons and Linux/Windows labels. A real
Pixman HiDPI render measured **43.9 ms** in a release test on the build host.
This is a first-render measurement, not end-to-end input latency or a physical
GPU benchmark. The test also checks a second output gets no popup, 30 idle
frames retain their buffer ID, wake recreates buffers, and mouse/Enter launch.

Display wake now invalidates compositor overlays and resets display buffers;
session reactivation resets connector state. Shell desktop and panel webviews
reload their visual resources while authentication and application state remain
alive. The NVIDIA open-module defaults use kernel suspend notifiers for VRAM
preservation. Physical NVIDIA/AMD display power cycles and suspend still need
hardware verification; this VM pass cannot establish the reported physical
artifact bug is eliminated.

Default builds/installs and Settings omit the developer bundle and controls.
GPU detection handles NVIDIA, AMD, Intel and virtual/hybrid systems; incompatible
NVIDIA auto-selection offers an interactive compatibility choice before disk
changes. CUDA, OBS and Discord are optional. Octopi is a default application
with matching dark styling and entry points in Mind, the taskbar and Settings.
Old layouts migrate away from redundant network/VPN widgets when they already
contain the system tray; deliberate layouts saved at the new version are kept.

The real Octopi regression also passed a graphical install and removal of
`figlet`, each authenticated through qt-sudo. It found two integration bugs
that are now fixed: the compositor reaps launched processes, and activating
or clicking a parent keeps its native/Wine transient dialogs above it. The
test deliberately clicks exposed parent content while confirmation is open,
then completes the transaction. Exact app IDs rank ahead of auxiliary tools
in Mind search, and Octopi's notifier is hidden from app search. Logs:
`gaming-software-vm.log` and `gaming-octopi-transactions.log`.

Windows file associations open a graphical Install/Run app flow. Desktop entries
launch through GIO so Wine's quoting and path handling survive intact. A real
Win32 probe in the disposable BIOS QA guest launched from Mind, reported the
Windows type, published an XEmbed tray icon, hid to its tray and restored on
click. It survived display blank/wake and password unlock. The tray controller
places the pointer on the desktop before replaying a click; its earlier click
at the exact screen corner was an invalid fixture for the real panel location.
A later fast close/restore run also intermittently timed out with the pointer
on the desktop. The final passing controller waits one second after the window
unmaps so Wine can consume withdrawal before restore. This narrows the issue
but does not establish that immediate close-to-tray/restore is reliable; that
timing race remains a limitation. No arbitrary Windows app or anti-cheat
compatibility guarantee follows.
Darling remains experimental and macOS GUI/tray integration is not enabled.

Validation: compositor **60** Rust tests and shell **35** Rust tests passed;
installer tests passed **34**, including **3** real pacman cases in the
disposable root build container; Windows helper/GIO tests passed **3**. TypeScript checking, production UI
build and browser smoke checks passed, including Software/Gaming navigation,
launch actions and compact layouts. Build logs are `build/logs/gaming-*.log`;
the real desktop probe records `gaming-windows-vm.log`. Screenshots include
`build/shots/gaming-mind-quicklaunch.png`, `gaming-windows-test.png` and
`gaming-software-search.png`. Native package builds and the QA guest exercise
the compiled binaries; no personal desktop, physical installation or host
graphics policy was changed.
