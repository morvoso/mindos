# Focused regression checks

Run from the repository root. These checks do not change the host's tuning
or install packages:

Gaming desktop checks: `python3 scripts/tests/test_games.py` validates local
launcher discovery and launch handoff in an isolated home. After building the
UI, `node scripts/tests/gaming-smoke.mjs` checks library actions, light/dark
preferences, static wallpaper, frosted glass and responsive desktop/login layouts.
It also checks Gaming/Work switching, persisted notes, embedded Settings,
primary-output migration and wallpaper-only secondary desktops.
The same smoke test covers Gaming Center sessions, account errors, save backups,
storage review, per-app audio, frame graphs, companion notes and the password
keyboard. `python3 scripts/tests/test_play.py` checks file integrity/recovery,
session requests, credentials, removed capture endpoints and audio leases in isolated fixtures.
`scripts/tests/gaming-native-session.py`, `gaming-native-snap.py` and `gaming-native-audio.py` run **inside
a disposable QA VM as its desktop user**, and exercise real cgroups, compositor snapping and PulseAudio.
Screenshots are written to `build/shots/gaming-smoke`.

```sh
python3 scripts/tests/test_session_lifecycle.py
python3 scripts/tests/test_boot_concurrency.py
python3 scripts/tests/test_perf.py
python3 scripts/tests/test_install_check.py
python3 scripts/tests/test_install_gpu.py
python3 scripts/tests/test_fstab_paths.py
python3 scripts/tests/test_pkg_repo.py
python3 scripts/tests/test_windows_apps.py
node scripts/tests/screensaver-engine.mjs
npm --prefix mindshell/ui run check
npm --prefix mindshell/ui run build
node scripts/tests/ui-smoke.mjs
cargo test --locked --manifest-path mindd/Cargo.toml
```

The browser checks need Chromium and a recent Node with built-in WebSocket
and TypeScript stripping (Node 26 was used). They use temporary browser
profiles, mock system actions and save screenshots in `build/shots/ui-smoke`.

The Mind Rust tests include real local HTTP peers that stall before headers,
between stream events and inside error bodies. Cancellation must finish within
two seconds and close the stream. A completed SSE response must finish on
`[DONE]` even if the server keeps its socket open.

Repository construction checks run in the Arch build box with real `repo-add`
and `vercmp`, using disposable package archives:

```sh
scripts/buildbox.sh python3 scripts/tests/test_build_repo.py
```

They check epochs/subreleases, architecture selection, archive retention and
preserving the published repository when staging fails. `test_pkg_repo.py`
uses fake package managers and bypasses root checking only in a temporary copy
of the helper. It verifies full upgrade transactions, batching, input validation,
failure handling and installed-package no-ops without changing the host.

Kernel checks inspect the resolved configuration and packaged modules:

```sh
python3 scripts/tests/check_kernel_config.py build/makepkg/linux-mindos/src/linux-7.2.3/.config
python3 scripts/tests/check_kernel_package.py build/packages/linux-mindos-7.2.3-2.1-x86_64.pkg.tar.zst
```

The package check uses Python 3.14's standard-library Zstandard reader. It
checks every module signature footer, rejects packaged signing keys, and
verifies that the NTSYNC-MODULE provider matches the built-in driver.
The VM boot check separately establishes that a module's signing key is trusted.

`test_snapshot_prune.py` needs root, Btrfs utilities, `btrfsutil` and loop
mount support. Run it in a test VM. It creates its own disposable 512 MiB
filesystem under `/var/tmp`; it does not operate on the VM's root snapshots:

The checks cover sealing an unused retained root after rollback, preserving
mounted/default/user roots and nested data, and pruning empty systemd child
subvolumes. A renamed but still-mounted root remains writable until unmounted.

```sh
python3 scripts/vm/vdrive.py exec 'python3 /home/morvoso/mindos/scripts/tests/test_snapshot_prune.py'
```

Adjust the guest share path for a different VM user. The mindshell Rust tests also run as
part of its package build; mindshell needs the Arch build box's WebKitGTK
libraries. See `docs/VALIDATION.md` for measured results and remaining hardware
checks. Passing these checks does not establish a game FPS improvement.


`capture_client.c` probes a running compositor. Generate its protocol header
and C file from the cached upstream XML, then compile in the portable build box:

```sh
wayland-scanner client-header build/cargo-home/registry/src/*/wayland-protocols-wlr-*/wlr-protocols/unstable/wlr-screencopy-unstable-v1.xml build/capture-protocol.h
wayland-scanner private-code build/cargo-home/registry/src/*/wayland-protocols-wlr-*/wlr-protocols/unstable/wlr-screencopy-unstable-v1.xml build/capture-protocol.c
scripts/buildbox.sh bash -c 'cc -O2 -Ibuild -o build/capture-client scripts/tests/capture_client.c build/capture-protocol.c $(pkg-config --cflags --libs wayland-client)'
```

Run `timeout 15 /path/to/build/capture-client damage` as the desktop user
in the QA VM, with `WAYLAND_DISPLAY` and `XDG_RUNTIME_DIR` set. `damage`
requires a static desktop for half a second; it checks that the second frame
waits for a change. `invalid` and `reuse` check protocol errors (the intentional
client disconnect is success). `lock` prints `PENDING`; lock the session from
a separate controller and it checks that the queued frame fails without
another image. These are interactive integration probes, not unattended CI.

`portal_capture.py` is a second interactive QA probe, run as the session user.
It opens the real portal chooser and saves one PNG frame through PipeWire:

```sh
python3 /path/to/scripts/tests/portal_capture.py --output /tmp/portal-frame.png
python3 /path/to/scripts/tests/portal_capture.py --cancel
```

Choose Share display for the first command and Cancel/Escape for the second.
The QA guest needs `python-gobject`, `gstreamer`, `gst-plugin-pipewire` and
`gst-plugins-good`; those testing tools are not added to the minimal image.

`pointer_client.c` and `test_pointer_vm.py` exercise mouse lock/confinement
with real Wayland clients and QEMU tablet events. Use a disposable 1920×1080
VM with the repository shared at `/home/qatest/mindos`. The probe temporarily
occupies the screen and changes focus; it exits automatically after 60 seconds.
Generate protocol files from the cached `wayland-protocols` XML:

```sh
python3 - <<'PY'
import pathlib, subprocess
root = next(pathlib.Path('build/cargo-home/registry/src').glob('*/wayland-protocols-0.32.*/protocols'))
for name, xml in [('pointer-constraints', 'unstable/pointer-constraints/pointer-constraints-unstable-v1.xml'),
                  ('relative-pointer', 'unstable/relative-pointer/relative-pointer-unstable-v1.xml'),
                  ('xdg-shell', 'stable/xdg-shell/xdg-shell.xml')]:
    for mode, suffix in [('client-header', 'h'), ('private-code', 'c')]:
        subprocess.run(['wayland-scanner', mode, str(root / xml), f'build/{name}-protocol.{suffix}'], check=True)
PY
scripts/buildbox.sh bash -c 'cc -O2 -Ibuild -o build/pointer-client scripts/tests/pointer_client.c build/pointer-constraints-protocol.c build/relative-pointer-protocol.c build/xdg-shell-protocol.c $(pkg-config --cflags --libs wayland-client xkbcommon)'
python3 scripts/tests/test_pointer_vm.py --dom mindos-qa-bios
```

The controller checks delayed activation outside the requested region,
incremental relative deltas while locked, no motion from a committed cursor
hint, confinement across an excluded strip, sliding along its edge, and release.
It also checks PS/2 unaccelerated relative input, startup fullscreen, and
mouse focus isolation when the session locks. This is a functional protocol
test, not a mouse-latency or frame-rate benchmark.


`python3 scripts/tests/test_dlss.py` runs isolated upscaler-swapper tests.
Synthetic versioned DLL fixtures cover atomic replacement, original retention,
permissions, launcher updates, archived originals, corrupted/missing backups,
failed record writes, partial multi-file changes, legacy records, path/link
handling and overlapping CLI writers. No installed game or network is used.
`ui-smoke.mjs` also covers Games filtering, scan failures/retry, duplicate
activation, Apply/Restore, version dialogs and compact layouts.

The pointer probe also logs Wayland keymaps, translated key presses and repeat
rate/delay events. Run it with `observe` to inspect input without a pointer
constraint. The UI smoke suite checks Input apply, discard, reset staging,
write rejection and persistence between page visits at normal/compact sizes.

`python3 -m unittest discover -s scripts/tests -p 'test_install*.py'` checks
disk/input validation, graphics selection and clean-target package resolution.
The real pacman cases require root inside the disposable build box:

```sh
scripts/buildbox.sh --root python3 scripts/tests/test_install_packages_pacman.py
```

They create a tiny temporary repository with intentionally mismatched NVIDIA
dependencies, check automatic fallback and explicit failure, and install nothing.
The resolver uses an empty temporary package DB and removes it afterward.

`make nvidia` runs the artifact verifier after building. It can also be invoked
independently (adjust all three paths together for a new kernel/driver):

```sh
scripts/buildbox.sh python3 scripts/tests/check_nvidia_package.py \
  build/packages/linux-mindos-nvidia-open-610.57.04-2-x86_64.pkg.tar.zst \
  build/packages/linux-mindos-7.2.3-2.1-x86_64.pkg.tar.zst \
  build/makepkg/linux-mindos/src/linux-7.2.3/certs/signing_key.x509
```

It verifies CMS signatures using the same public certificate for a released
kernel module and all five NVIDIA modules, checks ABI/version/dependencies,
rejects unsupported x86-64 relocations, and allows only modules and the license
in the payload. A real GPU test is still required for graphics performance.

`test_keyboard_vm.py` uses real Wayland and XWayland clients on an empty,
disposable QA desktop. It checks stable recent-window cycling, reverse/Escape,
consumed releases after modifiers change, Caps Lock, both native close protocols,
fullscreen switching by comparing rendered pixels, and shortcut inhibition.
Build its peers in the container (reuse the xdg-shell protocol generation above):

```sh
python3 - <<'PY'
from pathlib import Path
import subprocess
root = next(Path('build/cargo-home/registry/src').glob('*/wayland-protocols-0.32.*/protocols'))
xml = root / 'unstable/keyboard-shortcuts-inhibit/keyboard-shortcuts-inhibit-unstable-v1.xml'
for mode, suffix in [('client-header', 'h'), ('private-code', 'c')]:
    subprocess.run(['wayland-scanner', mode, str(xml), f'build/keyboard-shortcuts-inhibit-protocol.{suffix}'], check=True)
PY
scripts/buildbox.sh bash -c 'cc -O2 -Ibuild -o build/keyboard-client scripts/tests/keyboard_client.c build/xdg-shell-protocol.c build/keyboard-shortcuts-inhibit-protocol.c $(pkg-config --cflags --libs wayland-client)'
scripts/buildbox.sh cc -O2 -o build/x11-keyboard-client scripts/tests/x11_keyboard_client.c -lX11
python3 scripts/tests/test_keyboard_vm.py --dom mindos-qa-bios
```

The guest needs the repository shared at `/home/qatest/mindos` and the normal
user session running. The controller releases held keys, restores Caps Lock
if it enabled it, and closes its probes even after failure. Probe processes
also have a 180-second timeout. These are functional checks, not latency/FPS
benchmarks or physical GPU validation.

`test_media_vm.py` uses a temporary virtual input device in the QA guest to
check volume/mute, held-key cancellation, fullscreen feedback, MPRIS playback,
lock isolation and device removal. `test_session_graphics_vm.py` checks that
an application launched while the graphical session is inactive cannot take
DRM master with the new user-group defaults. Both need the disposable QA
session and shared probe binaries; they temporarily change focus and restore
their test state.

Suspend tests need a different VM device configuration because virtiofs
refuses sleep and a root-bus virtio GPU loses its display resources. Follow
the complete [suspend fixture procedure](../../docs/DEV-VM.md#suspend-testing-in-a-disposable-vm)
before running `prepare_suspend_vm.py` and `test_suspend_vm.py`. Preserve the
original XML and use only a disposable QA guest. Current results and the
remaining delayed S3 reset are recorded in
[validation](../../docs/VALIDATION.md#virtio-suspend-diagnosis-and-visual-recovery).

`test_windows_apps.py` uses fake Wine and a real GIO desktop launch to check
prefix validation, metadata, shortcut discovery/removal, and argument escaping
with spaces, quotes, backslashes, dollar signs, backticks and percent signs.
It does not install Wine or change the host's desktop entries.

The real Windows integration probe needs the updated gaming, compositor,
shell and Octopi packages in a disposable QA guest, logged in as `qatest`
with the repository shared at `/home/qatest/mindos`. Build the Win32 peer
with MinGW (in a disposable build container):

```sh
x86_64-w64-mingw32-gcc -O2 -DUNICODE -D_UNICODE -mwindows \
  scripts/tests/windows_tray_client.c -o build/windows-tray-probe.exe -lshell32
```

In the guest's graphical user session, first run
`mindos-win create mindos-tray-probe`. Then on the host run:

```sh
python3 scripts/tests/test_windows_vm.py --dom mindos-qa-bios
```

The controller assumes the disposable fixture password `mindos`, exercises
Mind search/launch, the Windows badge, a real Wine tray icon, close-to-tray,
tray restore, display blank/wake and password unlock, and opens Octopi.
It removes its shortcut and stops its prefix's Wine processes in cleanup.
It leaves the test prefix available for another run. Never use this controller
with a personal desktop. This is a functional probe, not a guarantee that every
Windows app or anti-cheat game works.

The compositor's `single_output_cache_clicks_and_wake_recovery` Rust test uses
the Pixman renderer to check HiDPI prompt rendering, isolation to one output,
cached idle frames, cache recreation after wake and mouse/keyboard launch.
Use `cargo test --release --locked --manifest-path mindwm/Cargo.toml
single_output_cache_clicks_and_wake_recovery -- --nocapture` in the build box
to print its first-render measurement.

`test_software_vm.py --dom mindos-qa-bios` exercises the real graphical
software manager in the same disposable fixture. Start with no app windows,
the packaged default Octopi window size/position, and `figlet` uninstalled.
It searches Mind for Octopi, checks the launch includes its dark stylesheet,
installs and removes the small `figlet` package through the UI and password
dialog, and checks that clicking/activating the parent keeps its confirmation
dialog above it. The fixture needs repository/network access. If interrupted,
inspect the pending transaction before retrying; the test deliberately does
not force-remove a package after a failed graphical action.
