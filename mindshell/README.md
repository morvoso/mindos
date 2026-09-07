# mindshell

The MindOS desktop shell host. One small Rust process that owns GTK4
layer-shell windows on every output (a desktop, the dock and the top bar,
transient popups) or, with `--app`, one ordinary window (the Settings
app) or the login screen (`--app greeter`, one overlay per output), renders the TypeScript UI in `ui/` with WebKitGTK 6.0, and
exposes the system to that UI through `window.mindos`: the compositor (mindwm
IPC socket: windows, layout mode, outputs, preferences), the Mind daemon,
the StatusNotifier tray, desktop entries, icons, files, wallpapers, audio,
network, battery, power and statistics. `docs/SHELL.md` is the contract; this
file is the host's operating notes.

## Layout of the crate

| file | what |
|---|---|
| `src/main.rs` | arguments (`--devtools`, `--ui-dir`, `--app <name> [--page <page>] [ARG]`), logging, GTK init, signals, main loop |
| `src/app.rs` | `App`: state, window bookkeeping, bridge dispatch (`dispatch`), host events, popups |
| `src/windows.rs` | `ShellWindow`: layer-shell geometry per window kind, monitors, transparency CSS |
| `src/bridge.rs` | the `window.mindos` bootstrap script, request parsing, reply/dispatch JS, window URLs |
| `src/scheme.rs` | the `mindos://shell/` URI scheme (`/app`, `/icon`, `/tray`, `/file`, `/thumb`) |
| `src/ipc.rs` | compositor IPC client (newline JSON over `$MINDWM_SOCKET`), auto-reconnect; layout mode, outputs and preferences requests |
| `src/mind.rs` | client for the Mind daemon socket (`/run/mindos/mind.sock`): `mind.request` pass-through, `models` / `download` events |
| `src/fs.rs` | the desktop folder: directory listings with MIME types and thumbnails, trash, `gio open`, wallpaper folders |
| `src/portal.rs` | the Wallpaper portal backend (`org.freedesktop.impl.portal.Wallpaper` on `org.freedesktop.impl.portal.desktop.mindos`): "Set as Background" in Files / Image Viewer writes the layout's `desktop.wallpaper` |
| `src/tray.rs` | StatusNotifierItem/DBusMenu through `system-tray` on a tokio thread; pixmaps as PNG |
| `src/apps.rs` | desktop-entry index (XDG data dirs + Flatpak exports), Exec cleaning, detached spawn |
| `src/icons.rs` | icon lookup (theme, `IconThemePath`, pixmaps), percent-encoding |
| `src/system.rs` | `systemctl` power, `/proc` statistics, `nvidia-smi`, `wpctl`, `nmcli`, sysfs battery |
| `src/layout.rs` | `layout.json` model, validation (`sanitized`), atomic save, reset |
| `src/config.rs` | `shell.toml` model and overlay of `/etc` + `~/.config` |
| `data/` | default `layout.json`, `shell.toml`, `mindos-shell.service`, `50-mindshell` autostart, the `mindos-settings` / `mindos-displays` / `mindos-wallpaper` desktop entries, `mindos.portal` and the D-Bus activation file of the Wallpaper portal backend |
| `ui/` | the TypeScript UI (built by `ui/build.sh <outdir>` with esbuild) |

## Runtime model

* **Windows.** One `gtk::Window` per (kind, id, output). All WebViews share one
  web process (`related-view`) and the `mindos://shell` origin, with their own
  `UserContentManager` so the host knows which view sent a message. Windows
  are re-created when monitors appear or disappear (`gdk::Display::monitors`),
  and panels are re-fitted in place when only their geometry changes.
* **Bridge.** `bridge::BOOTSTRAP` is injected at document start. A page calls
  `mindos.call(method, params)`; the host answers with
  `mindos._reply(id, ok, payload)` and pushes events with
  `mindos._dispatch(event, payload)` (`evaluate_javascript`). Bad JSON is
  logged and dropped, never fatal.
* **Threads.** GTK owns the main thread. The compositor client, the tray
  (tokio), the desktop-entry scanner, the audio poller and the GPU sampler run
  on their own threads and post `HostEvent`s through an `async_channel` that a
  `glib::spawn_future_local` loop drains. Blocking helpers (`nmcli`, `wpctl`,
  `systemctl`) run through `app::blocking` so the UI never stalls.
* **No compositor IPC?** The shell still runs: `windows` is empty, `mind` is
  `{connected:false}`, launches fall back to `sh -c` in the shell's own
  session, and the client keeps retrying the socket with backoff.
* **Popups.** Full-output transparent overlay windows. The `anchor` passed to
  `popup.open` is already in output-local logical pixels (the UI adds the
  panel window's origin); the host hands it to the popup unchanged as
  `mindos.window.anchor` and the popup places itself. Escape closes a popup
  at the host level; clicking the transparent area is the UI's job. A closed
  window stays alive, hidden, for 1.5 s so a menu that closes itself and then
  runs its action still gets that request through.
* **Edit mode.** Panels grow by 140 px toward the screen centre (exclusive
  zone unchanged) and the desktop gets on-demand keyboard focus.
* **Fit panels.** A panel with `length: 0` (the dock) starts one pixel wide;
  the UI measures its widgets and calls `panel.fit`, and the host resizes the
  window in place, keeping the alignment. Nothing is drawn by GTK: the panel
  background, if any, is the UI's.
* **App windows.** `mindshell --app settings|files` is a second entry point
  in the same binary: one `gtk::Window` without decorations (the compositor
  draws the title bar), app id `mindos-<name>`, the same bridge and scheme,
  no layer-shell. The process ends when the window closes or the UI calls
  `app.close`; the shell spawns them through `shell.openApp`.

## Building and running

```sh
# host
scripts/buildbox.sh bash -c 'cd mindshell && CARGO_HOME=/work/build/cargo-home cargo build --release && cargo test --release'
# package (host + UI bundle + service + autostart)
scripts/buildbox.sh bash -c 'cd packages/mindshell && makepkg -sf --noconfirm --skippgpcheck'
# run against a UI checkout instead of the installed bundle, with the inspector (F12)
MINDSHELL_UI_DIR=~/mindos/mindshell/ui/dist mindshell --devtools
```

Logs: `journalctl --user -u mindos-shell` (`RUST_LOG=debug` for the bridge
traffic; the UI's `console.*` output lands there too). `systemctl --user
restart mindos-shell` restarts the shell without touching the session.

Files at runtime: `/usr/share/mindos/shell/ui` (bundle),
`/usr/share/mindos/shell/layout.json` and `~/.config/mindos/shell/layout.json`
(layout; the user file wins, `layout.reset` deletes it), `/etc/mindos/shell.toml`
and `~/.config/mindos/shell.toml` (config), `~/.local/share/mindos/shell` and
`~/.cache/mindos/shell` (WebKit storage).
