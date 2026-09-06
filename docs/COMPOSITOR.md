# mindwm — the MindOS compositor

`mindwm` is the Wayland compositor MindOS boots into. It is a fork of
[Smithay](https://github.com/Smithay/smithay)'s reference compositor
(`anvil`, Smithay 0.7) with MindOS behaviour layered on top:

* **DRM/KMS session** via libseat/logind, libinput, GBM + EGL/GLES (NVIDIA
  and Mesa both work through the normal Linux driver stack), multi-GPU aware,
  direct scanout for fullscreen games. `--winit` runs it nested for development.
* **XWayland** built in, so X11 games and launchers (Steam, Proton/Wine, Lutris)
  run unchanged.
* **Game mode by default.** A new window fills the output it opens on; dialogs
  (toplevels with a parent, X11 transients/utility windows) keep their size and
  are centred over their parent. Fullscreen requests get direct scanout.
  Client-side decorations are the default; nothing draws a title bar over a game.
* **MindOS red.** The desktop background is `#8c1010` with white text, matching
  the kernel console, GRUB and Plymouth. When no window is open the "MindOS"
  wordmark and key hints are drawn behind everything.
* **The Mind bar** (`Super+Space`): a software-rendered overlay that is both an
  application launcher and the front end of `mindd`, the local LLM daemon.

## Focus

Game mode means no clicking around: a window that maps (Wayland or X11) gets
keyboard focus immediately, and when the focused window closes or crashes the
top-most remaining window takes over. Clicking a window still focuses and
raises it; `Super+Tab` cycles.

## Keybindings

| Keys | Action |
|------|--------|
| `Super+Space` | Open/close the Mind bar |
| `Super+Enter` | Terminal (`[apps].terminal`, default `foot`) |
| `Super+Q` | Close the focused window |
| `Super+F` | Toggle fullscreen on the focused window |
| `Super+M` | Toggle maximize on the focused window |
| `Super+Tab` / `Alt+Tab` | Cycle windows |
| `Super+1..9` | Move the pointer to output *n* |
| `Super+Shift+E`, `Ctrl+Alt+Backspace` | Quit the compositor (ends the session) |
| `Ctrl+Alt+F1..F12` | Switch virtual terminal |
| `Super+Shift+P` / `Super+Shift+M` | Output scale up / down |
| `Super+Shift+W` | Window overview |

Only `Super` combinations are consumed by the compositor; games see every
other key unmodified, and clients that use the keyboard-shortcuts-inhibit
protocol get everything.

## The Mind bar

* Type to filter installed applications (XDG desktop entries from
  `$XDG_DATA_DIRS`); `Enter` launches the highlighted one, `↑`/`↓`/`Tab` select.
* Anything that is not an application, a query prefixed with `?`, or
  `Shift+Enter`, is sent to Mind. The answer streams in; tool calls show as
  `⚙ run: nvidia-smi`, results as `✓ run_command: …`.
* When Mind wants to change the system (install packages, restart a service,
  edit a config) and autopilot is off, the bar shows *Mind wants to: …* and
  waits for `Y` or `N`. The daemon's policy layer decides what needs confirmation
  and what is forbidden outright; see `docs/ARCHITECTURE.md`.
* `!command` runs a shell command in the session. `Esc` cancels a running
  answer, then closes the bar. `Ctrl+L` clears the conversation.
* Mind can act inside the session through *client tools* the compositor
  registers on connect: `launch_app`, `open_terminal`, `run_in_terminal`.

The bar talks to `mindd` over `/run/mindos/mind.sock` (newline-delimited JSON,
see `mindd/src/proto.rs`). If the daemon is not running the bar still works as a
launcher and says so in its status line; it reconnects automatically.

## Configuration

`/etc/mindos/mindwm.toml`, overlaid by `~/.config/mindos/mindwm.toml`
(and `$MINDWM_CONFIG`):

```toml
[startup]
# spawned through `sh -c` once the Wayland socket and XWayland are up;
# WAYLAND_DISPLAY and DISPLAY are set in their environment
exec = ["/usr/lib/mindos/session-startup"]

[apps]
terminal = "foot"

[mind]
socket = "/run/mindos/mind.sock"
autopilot = false      # true: apply "change" actions without asking

[theme]
background = "#8c1010"
foreground = "#ffffff"
show_wordmark = true
```

`MIND_SOCKET` in the environment overrides `[mind].socket`.

## Layout of the sources

| File | What it does |
|------|--------------|
| `src/main.rs` | Backend selection (`--tty-udev` on a TTY, `--winit` nested, auto-detected) |
| `src/config.rs` | Config loading and merging |
| `src/mindbar.rs` | Mind bar state machine and CPU rendering |
| `src/text.rs` | fontdue text rasteriser into premultiplied BGRA memory buffers (embedded DejaVu Sans) |
| `src/launcher.rs` | Desktop-entry index and ranking |
| `src/mind.rs` | Threaded client for `mindd`; events arrive through a calloop channel |
| `src/edid.rs` | Minimal EDID parser for output make/model (replaces libdisplay-info) |
| `src/shell/mod.rs` | Game-mode placement (`game_mode_initial_state`, `pointer_output_area`) |
| `src/shell/xdg.rs`, `src/shell/x11.rs` | xdg-shell and XWayland window management |
| `src/input_handler.rs` | Keybindings (`process_keyboard_shortcut`) and Mind bar key routing |
| `src/render.rs` | Output element assembly: cursor, Mind bar overlay, windows, wordmark backdrop |
| `src/udev.rs`, `src/winit.rs` | DRM/KMS and nested backends (from anvil) |

## Development

```sh
cd mindwm
cargo build --release
# nested, inside any Wayland/X11 session; talks to a test daemon if MIND_SOCKET is set
MIND_SOCKET=/run/user/1000/mind-test.sock ./target/release/mindwm --winit
# preview the Mind bar rendering without a display
MINDWM_PREVIEW_DIR=/tmp/preview cargo test --release --lib renders_preview
```

## GPUs, software rendering and virtual machines

On a real boot mindwm picks the udev "primary" GPU (the boot VGA device), keys
its renderer by that device's render node and drives every other DRM device
(iGPU, DisplayLink, ...) by rendering on the primary and copying frames over.
If the primary device has no hardware-accelerated EGL implementation (a VM
with `virtio-vga` and no virgl, or a GPU without a Mesa driver), mindwm falls
back to Mesa's software rasterizer on that device instead of refusing to
start; the journal then says `rendering with Mesa software rasterizer`. If
the udev choice cannot be initialised at all, the first device that did come
up becomes the primary. Clients get no dmabuf/`wl_drm` in software mode and
use `wl_shm`, which is fine for terminals and the installer.

Set `ANVIL_DRM_DEVICE=/dev/dri/cardN` in the session environment to force a
specific device.

## Debugging on the live ISO

* `journalctl -t mindwm` has the compositor log (`mindos-session` pipes it
  through `systemd-cat`).
* `Ctrl+Alt+F2` is a root shell on the live ISO; in QEMU with
  `-serial file:...` anything redirected to `/dev/ttyS0` lands in that file.
* A crash drops back to greetd, which shows the text greeter (agreety) on
  VT 1. greetd only runs the autologin `initial_session` once per boot; to
  re-run it after fixing something, `rm /run/greetd.run && systemctl restart
  greetd`.
* A VT switch pauses the session: rendering stops until the VT comes back
  (no repaint retries while inactive).
* To try a new build without rebuilding the ISO, ship the stripped binary
  on a second disk (`mke2fs -q -F -t ext4 -d dir img`, attached as
  `/dev/vdb`) and from the root shell run `mount /dev/vdb /mnt && cp
  /mnt/mindwm /usr/bin/mindwm.new && mv -f /usr/bin/mindwm.new
  /usr/bin/mindwm`, then restart greetd as above. Copy-then-rename matters:
  copying straight over the running binary fails with "Text file busy" and
  leaves the old one in place, so check `md5sum /usr/bin/mindwm`.
