# mindshell — the MindOS desktop shell

`mindshell` is the MindOS desktop environment: panels, launcher, taskbar,
system tray, clock and desktop widgets. It is one small Rust process (the
*host*) that opens layer-shell windows on the compositor and renders every
window with WebKitGTK; the user interface itself is HTML/CSS/TypeScript
(`mindshell/ui`). The host owns everything that needs the system (D-Bus,
the compositor IPC, files, processes); the UI owns everything visual and is
hot-reloadable.

```
mindwm ──(layer-shell + IPC socket)── mindshell host ──(bridge)── WebKit UI
                                          │
                                          ├── StatusNotifier (tray) on D-Bus
                                          ├── desktop entries + icon themes
                                          ├── wpctl / nmcli / sysfs / procfs
                                          └── ~/.config/mindos/shell/layout.json
```

Design rules: minimal, dark, futuristic. One accent (electric cyan). Red is
reserved for the kernel/boot stages and never appears in the shell. No blur,
no animations that cost GPU time while a game runs. Everything is a *widget*
in a *container* (panel or desktop); the layout is data (`layout.json`) and
the whole desktop is rebuilt from it, so *edit mode* is just editing that
data with a nicer UI.

## Processes and files

| What | Where |
|---|---|
| host binary | `/usr/bin/mindshell` |
| UI bundle (built by esbuild) | `/usr/share/mindos/shell/ui/` (`index.html`, `app.js`, `app.css`, `fonts/`) |
| default layout | `/usr/share/mindos/shell/layout.json` |
| user layout | `~/.config/mindos/shell/layout.json` |
| host config | `/etc/mindos/shell.toml`, `~/.config/mindos/shell.toml` |
| user service | `mindos-shell.service` (systemd --user, `Restart=on-failure`), started by `/etc/xdg/mindos/autostart/50-mindshell` |
| log | `journalctl --user -u mindos-shell` |

Environment: `WAYLAND_DISPLAY` (from the compositor), `MINDWM_SOCKET` (the
compositor IPC socket, exported by mindwm to everything it spawns and imported
into `systemd --user` by `session-startup`), `MINDSHELL_UI_DIR` (override the
UI bundle location, for development), `MINDSHELL_DEVTOOLS=1` (enable the
WebKit inspector, `mindshell --devtools` does the same).

`shell.toml`:

```toml
[shell]
icon_theme = "breeze-dark"      # any installed XDG icon theme; hicolor is the fallback
hardware_acceleration = "always" # always | never (WebKit compositing policy)
terminal = "foot"

[popups]
launcher_width = 720
launcher_height = 560
```

## Layout (`layout.json`)

```json
{
  "version": 1,
  "panels": [
    {
      "id": "bottom", "output": "*", "edge": "bottom",
      "size": 48, "length": 100, "align": "center", "margin": 0,
      "layer": "top", "opacity": 0.92,
      "widgets": [
        { "id": "start", "type": "start", "config": {} },
        { "id": "tasks", "type": "taskbar", "config": { "pins": ["firefox.desktop", "foot.desktop", "steam.desktop"] } },
        { "id": "sp1", "type": "spacer", "config": { "expand": true } }
      ]
    },
    {
      "id": "top", "output": "*", "edge": "top",
      "size": 30, "length": 100, "align": "center", "margin": 0,
      "layer": "top", "opacity": 0.92,
      "widgets": [
        { "id": "mind", "type": "mind", "config": {} },
        { "id": "sp2", "type": "spacer", "config": { "expand": true } },
        { "id": "tray", "type": "tray", "config": {} },
        { "id": "audio", "type": "audio", "config": {} },
        { "id": "net", "type": "network", "config": {} },
        { "id": "bat", "type": "battery", "config": {} },
        { "id": "clock", "type": "clock", "config": { "seconds": false, "date": true, "hour24": true } }
      ]
    }
  ],
  "desktop": {
    "wallpaper": { "mode": "builtin" },
    "widgets": [
      { "id": "d-clock", "type": "desktop-clock", "output": "*", "x": 64, "y": 64, "w": 320, "h": 120, "config": {} }
    ]
  }
}
```

* `panel.output`: `"*"` means one instance of the panel on every output;
  otherwise a connector name (`DP-1`, `Virtual-1`).
* `edge`: `top | bottom | left | right`. `size` is the thickness in logical
  pixels (also the exclusive zone). `length` is a percentage of the edge
  (100 = full width). `align`: `start | center | end` when `length < 100`.
  `layer`: `top` (normal) or `bottom` (windows cover it, like a dock that
  hides under maximized windows). `margin`: distance from the edge.
* Widgets are ordered left→right (or top→bottom on vertical panels). A
  `spacer` with `expand: true` pushes what follows to the far end.
* Desktop widgets have a position/size in logical pixels on their output.
* Unknown widget types render as an "unavailable" placeholder and are kept.

### Widget types (v1)

| type | container | what |
|---|---|---|
| `start` | panel | the MindOS button; opens the launcher popup |
| `taskbar` | panel | pinned apps + running windows; click focus/minimize, middle-click new instance, right-click pin/unpin/close |
| `spacer` | panel | flexible or fixed gap (`expand`, `size`) |
| `clock` | panel | time (+ date); click opens the calendar popup |
| `tray` | panel | StatusNotifierItems; left-click activate, right-click menu, scroll |
| `audio` | panel | default sink volume; scroll adjusts, click opens the slider popup, middle-click mutes |
| `network` | panel | wired/wifi state |
| `battery` | panel | charge state (hidden when no battery) |
| `mind` | panel | Mind (mindd) status; click toggles the Mind bar |
| `sysmon` | panel | compact CPU / memory / GPU bars |
| `power` | panel | power menu button |
| `desktop-clock` | desktop | large clock + date |
| `desktop-sysmon` | desktop | CPU / memory / GPU graphs |
| `desktop-notes` | desktop | a sticky note (plain text, stored in the widget config) |

Adding a widget type = one TypeScript module registering `{ type, name,
description, containers, defaults, create(ctx) }` in the widget registry.

## Windows the host creates

One WebKit view per window; all views share one web process (`related-view`)
and one `mindos://shell/` origin.

| kind | layer-shell | where |
|---|---|---|
| `desktop` (one per output) | `background`, anchored to all edges, exclusive −1, keyboard `none` (`on-demand` while in edit mode) | wallpaper, desktop widgets, edit-mode toolbar, right-click menu |
| `panel` (one per panel × output) | `top`/`bottom` per layout, anchored to the panel edge (+ both sides when `length` = 100), exclusive zone = `size` + `margin`, keyboard `none` | the panel and its widgets; in edit mode the window is enlarged by 140 px toward the screen centre (exclusive zone unchanged) to show the panel settings strip |
| `popup` (transient) | `overlay`, anchored to all edges (full output, transparent), keyboard `exclusive` when `keyboard: true` else `on-demand` | launcher, calendar, audio slider, power menu, tray menus, widget catalog, widget settings, context menus. Clicking the transparent area or pressing Escape closes it |

Every window loads `mindos://shell/app/index.html?kind=<kind>&id=<id>&output=<name>`
(`&popup=<name>&arg=<json>` for popups). `?kind=preview` renders every
window stacked on one page: it is what `make shell-preview` screenshots in
Chromium for design work without a compositor.

## The bridge (`window.mindos`)

Injected into every view before any script runs.

```ts
mindos.call(method: string, params?: object): Promise<any>   // request → host
mindos.on(event: string, cb: (payload: any) => void): () => void
mindos.window: { kind, id, output, popup?, arg? }            // parsed from the URL
```

Transport: `window.webkit.messageHandlers.mindos.postMessage(JSON.stringify({id, method, params}))`;
the host answers with `window.mindos._reply(id, ok, payload)` and pushes
events with `window.mindos._dispatch(event, payload)`. When the page is not
inside the host (a browser), `src/mock.ts` installs a fake host with sample
data so the UI can be developed in Chromium/Firefox.

### Methods

| method | params → result |
|---|---|
| `shell.state` | → `{ user, host, uptime, outputs, windows, focused, apps, tray, layout, editMode, config }` |
| `shell.ready` | the view has rendered its first frame |
| `shell.setEditMode` | `{ enabled }` → broadcasts `edit_mode` |
| `shell.exec` | `{ cmd }` runs a command line in the session (`sh -c`) |
| `shell.reload` | reloads every view (development) |
| `layout.get` | → `layout` |
| `layout.save` | `{ layout }` → validates, writes the user file, rebuilds windows, broadcasts `layout` |
| `layout.reset` | back to the default layout |
| `popup.open` | `{ name, keyboard?, anchor?: {x,y,w,h,edge}, arg? }` → opens (or moves) the popup on the calling window's output |
| `popup.close` | `{ name }` (or none = the calling popup) |
| `popup.toggle` | as `open`, but closes when already open |
| `windows.focus` | `{ id }` (unminimises) |
| `windows.close` | `{ id }` |
| `windows.minimize` | `{ id }` |
| `windows.toggleMinimize` | `{ id }` |
| `apps.list` | → `[{ id, name, comment, exec, icon, categories, terminal }]` (`icon` is a `mindos://shell/icon/...` URL) |
| `apps.launch` | `{ id }` or `{ exec, terminal }` |
| `tray.items` | → `[{ id, title, tooltip, icon, status, hasMenu }]` |
| `tray.activate` / `tray.secondaryActivate` | `{ id, x, y }` |
| `tray.scroll` | `{ id, delta, orientation }` |
| `tray.menu` | `{ id }` → `[{ id, label, enabled, type: "item" \| "separator" \| "submenu", toggle?: "checkmark" \| "radio", checked?, icon?, children? }]` |
| `tray.menuClick` | `{ id, item }` |
| `mind.toggle` | opens/closes the compositor's Mind bar |
| `mind.status` | → `{ connected, ready, model }` |
| `system.power` | `{ action: "shutdown" \| "reboot" \| "suspend" \| "logout" }` |
| `system.stats` | → `{ cpu, memUsed, memTotal, gpu?: { util, temp, mem, memTotal, name }, load, uptime }` |
| `audio.get` | → `{ volume, muted, sink }` |
| `audio.set` | `{ volume }` (0..1.5) |
| `audio.toggleMute` | |
| `network.status` | → `{ connected, kind: "ethernet" \| "wifi" \| "none", ssid?, iface, ip? }` |
| `battery.status` | → `{ present, percent, charging, timeToEmpty? }` |
| `icons.resolve` | `{ name, size }` → URL |

### Events

| event | payload |
|---|---|
| `windows` | `{ windows: [...], focused }` (full snapshot on every change) |
| `outputs` | `{ outputs: [...] }` |
| `apps` | `{ apps }` (desktop database changed) |
| `tray` | `{ items }` |
| `layout` | `{ layout }` |
| `edit_mode` | `{ enabled }` |
| `popup_state` | `{ name, open, output }` |
| `mind` | `{ connected, ready, model }` |
| `audio` | `{ volume, muted }` |
| `shortcut` | `{ name }` forwarded from the compositor (`launcher`, `overview`) |

`mindos://shell/icon/<name>?size=N` serves an icon from the configured theme
(`hicolor` fallback, SVG or PNG); an absolute path in place of `<name>` serves
that file. `mindos://shell/tray/<id>` serves a tray item's pixmap.
`mindos://shell/app/...` serves the UI bundle; `mindos://shell/app/fonts/...`
serves the bundle's own `fonts/` first and falls back to
`/usr/share/fonts/mindos` and the system font directories.

### Host implementation notes

What `mindshell` (the Rust host in `mindshell/`) does beyond the tables above:

* **Panel `output`.** `*` = every output, a connector name = that output, and
  `primary` (or an empty string) = the first output (top-left in GDK's
  monitor list, which is also `outputs[0]` in `shell.state`).
* **Popup anchors pass through.** `popup.open` / `popup.toggle` take the
  `anchor` in output-local logical pixels, as the UI conventions below say
  (the UI adds the panel window's origin itself); the host does not translate
  it. The popup receives it as `mindos.window.anchor` (the `&anchor=<json>`
  URL parameter) and inside `mindos.window.arg.anchor`, and places itself.
* **Popups.** One window per popup name across all outputs; opening the same
  name from another output moves it (a `popup_state` close for the old
  output, then an open). Re-opening an already open popup on the same output
  re-navigates the view only when its URL (arg/anchor) changed. `popup.toggle`
  replies `{ name, open, output }`. Escape closes a popup at the host level
  (a GTK key controller in the capture phase); clicking the transparent area
  is the UI's `popup.close`. A closed window is unmapped at once but its view
  lives on, hidden, for 1.5 s: a menu that calls `popup.close` and then runs
  its action (`shell.setEditMode`, `popup.open`, `shell.exec`, ...) still gets
  that request answered.
* **`shortcut` events are forwarded only.** The host broadcasts the
  compositor's `shortcut` to every view and opens nothing itself; the `start`
  widget on the first output answers `launcher`. Without a shell connected the
  compositor falls back to its own Mind bar for the Super tap.
* **Extra methods.** `windows.unminimize`, `windows.toggleFullscreen`,
  `windows.toggleMaximize`, `mind.open`, `mind.close`, `outputs.list`.
  `shell.exec` also accepts `terminal: true`.
* **Extra fields.** `shell.state` adds `compositor` (IPC connected),
  `devtools`, `config.icon_size`; `mind` adds `open` (the Mind bar state) and
  merges whatever the compositor answers to a `mind_status` request or sends
  as a `mind_status` / `mind` event; `audio` adds `available` (false when
  `wpctl` is missing); each app carries `iconName` next to the resolved
  `icon` URL; tray items carry `isMenu` and `app` (the item's D-Bus id);
  `system.stats` gives `load` as `[1m, 5m, 15m]`, `cores`, and `gpu` from
  `nvidia-smi` sampled every 2 s on a thread (null without a GPU).
* **Launching.** `apps.launch` and `shell.exec` go through the compositor's
  `launch` request when the IPC is connected (so the program gets the
  session's environment); otherwise the host runs `sh -c` itself, detached in
  a new session, using `terminal` from `shell.toml` for terminal entries.
* **Icons.** `icons.resolve` answers `null` when nothing matches; `apps.list`
  falls back to `application-x-executable`. The theme is picked at startup
  (the configured one, then breeze-dark, breeze, Papirus-Dark, Papirus,
  Adwaita, hicolor) by looking for `<dir>/<theme>/index.theme`. Tray items
  prefer a themed icon name (also via the item's `IconThemePath`) and fall
  back to their pixmap, served as PNG from `mindos://shell/tray/<id>?v=N`.
* **Unknown compositor events** are broadcast to the views under their own
  name, so the compositor can grow events without a host change.
* **Edit mode** re-fits desktop and panel windows in place (`edit_mode` is
  broadcast after the geometry changed); the panel window grows by 140 px
  toward the centre, the exclusive zone stays `size + margin`.
* **Robustness.** Bad bridge JSON is logged and dropped; a crashed web
  process reloads its view after 500 ms; monitors appearing or disappearing
  re-create the windows (debounced 300 ms so the connector name has arrived);
  the compositor socket is retried with backoff and every state that came
  from it is cleared on disconnect. `GDK_BACKEND=wayland` is forced and the
  host exits with status 1 when the compositor has no layer-shell.
* **Service.** `mindos-shell.service` uses `KillMode=mixed` so only the host
  gets SIGTERM (WebKit's helper processes follow it), `Restart=on-failure`
  with a 5-per-minute limit. `50-mindshell` imports `WAYLAND_DISPLAY`,
  `DISPLAY` and `MINDWM_SOCKET` into `systemd --user` and restarts the unit;
  without a user manager it runs `mindshell` detached.

## The compositor IPC (mindwm)

mindwm listens on `$XDG_RUNTIME_DIR/mindwm-<wayland-socket>.sock` and exports
the path as `MINDWM_SOCKET` to the programs it starts. Newline-delimited JSON;
each request may carry an `id` that the reply echoes.

Requests → replies (`{"id":1,"ok":true,"result":{...}}` or `{"id":1,"ok":false,"error":"..."}`):

| type | fields | result |
|---|---|---|
| `subscribe` | | `{}` then a `windows`, an `outputs` and a `mindbar` event immediately, and every change afterwards |
| `get_windows` | | `{ windows, focused }` (same shape as the `windows` event) |
| `get_outputs` | | `{ outputs }` |
| `focus` | `window` | raise, unminimise and focus |
| `close` | `window` | |
| `minimize` / `unminimize` / `toggle_minimize` | `window` | |
| `toggle_fullscreen` / `toggle_maximize` | `window` | |
| `mindbar` | `action: toggle \| open \| close` | |
| `overview` | `action: toggle` | |
| `launch` | `exec`, `terminal?` | spawn in the session |
| `terminal` | | open the configured terminal |
| `quit` | | end the session |

Events (`{"event":"...", ...}`):

| event | fields |
|---|---|
| `windows` | `windows: [{ id, title, app_id, focused, fullscreen, maximized, minimized, x11, output }]`, `focused: id \| null` |
| `outputs` | `outputs: [{ name, make, model, x, y, width, height, scale, refresh }]` (logical pixels; `refresh` in Hz, e.g. `240.0`) |
| `shortcut` | `name`: `launcher` (a tap on Super alone), `overview` (`Super+W`). Only sent while someone is subscribed; without a shell the compositor opens its own Mind bar / window preview instead |
| `mindbar` | `open: bool` |

Window ids are stable for the life of a window and follow creation order.
Snapshots are sent whenever any listed field changes (map, unmap, title,
focus, state); override-redirect X11 windows (menus, tooltips) and toplevels
that have not drawn yet are not listed. Errors look like
`{"id":1,"ok":false,"error":"no such window: 9"}`; a request line over 64 KiB
or 4 MiB of unread events disconnects the client.

## Development

```sh
# UI only, in a browser with the mock host
cd mindshell/ui && npm install && npm run dev        # esbuild --watch → dist/
chromium dist/index.html?kind=preview

# host + UI in the dev VM (docs/DEV-VM.md)
make packages && make repo
scripts/vm/vdrive.py exec 'pacman -Syu --noconfirm && systemctl --user -M morvoso@ restart mindos-shell'
scripts/vm/vdrive.py shot shot.png
```

## UI conventions (mindshell/ui)

What the TypeScript side assumes beyond the tables above. The host has to
match these; everything else is internal to the bundle.

**Build.** `npm run build` writes `dist/` (`index.html`, `app.js`, `app.css`,
`fonts/*.ttf`); the host serves that directory as `mindos://shell/app/`.
`npm run check` type-checks, `npm run dev` rebuilds on change, `npm run shot`
renders the preview page with headless Chromium into `shots/`
(`CHROMIUM=/path/to/chrome` to override the binary). No runtime dependencies;
esbuild and TypeScript are dev-only.

**Bridge fallback.** If `window.mindos` is missing `call`/`on` at load time
the UI builds them itself on top of
`window.webkit.messageHandlers.mindos`, so the host only has to inject the
message handler and call `window.mindos._reply(id, ok, payload)` /
`window.mindos._dispatch(event, payload)`. `shell.ready` carries
`{ kind, id, popup }`. Methods that fail reject with the host's error string.

**Anchors.** `popup.open`/`popup.toggle` receive the anchor twice: as the
top-level `anchor` (for the host) and inside `arg.anchor` (for the popup
page). Coordinates are output-local logical pixels. The popup places itself
8 px away from the anchor on the side opposite `anchor.edge` (a `bottom`
panel opens upwards, `top` downwards, `left`/`right` sideways), centred on
narrow anchors; without an `edge` the anchor is treated as a pointer position
(context menus). A missing anchor centres the popup on the output.

**Panel geometry.** For anchors and the preview the UI computes each panel
window as: thickness `size` (+140 px in edit mode), `margin` from its edge,
length `length`% of the edge placed by `align` (`start` = left/top corner,
`end` = right/bottom, `center`). Vertical panels are inset by the exclusive
zones (`size + margin`) of the horizontal panels on the same output, so the
host must create top/bottom panel surfaces before left/right ones. A
`length` < 100 panel should be anchored to its edge plus the `align` side
(or the edge only for `center`) with the window sized to `length`% of the
output.

**Popups and their `arg`.**

| name | arg |
|---|---|
| `launcher` | none (size from `config.popups.launcher_width/height`, default 760×560) |
| `calendar` | `{ hour24? }` |
| `audio` | none |
| `power` | none |
| `context-menu` | `{ title?, items: [{ label, icon?, disabled?, separator?, danger?, action? }] }` |
| `tray-menu` | `{ id, title? }` (calls `tray.menu` itself) |
| `widget-catalog` | `{ target: { kind: "panel", id } \| { kind: "desktop", output } }` |
| `widget-settings` | `{ target: { kind: "panel", id, widget } \| { kind: "desktop", widget } }` |

An `action` is one of `{ call, params }`, `{ popup, arg?, keyboard? }`,
`{ editMode }`, `{ exec }` or `{ pin: { panel, widget, app, pinned } }`; menus
run it through the host so a popup window can act on another window's
behalf.

**`shortcut` events.** The host forwards them to every view; the `start`
widget on the first output (per `outputs` order) toggles the launcher, so the
host must not open the launcher itself as well. `layout.reset` is expected to
broadcast a `layout` event like `layout.save` does.

**Wallpaper.** With `desktop.wallpaper.mode = "image"` the desktop sets
`background-image: url(file://<path>)` (a path that already has a scheme is
used verbatim). The host must allow `file:` images from the `mindos://` origin
or rewrite the path to a `mindos://shell/...` URL before handing the layout
to the UI.

**Widget settings.** Widgets declare `settings: { key: { label, type, min?,
max?, step?, options?, help? } }` with `type` one of `boolean | number |
string | text | enum | list`; `widget-settings` builds its form from that
(falling back to the types of `defaults`). `list` values are string arrays,
one item per line in the form.

**Preview page.** `?kind=preview` renders one 1920×1080 output scaled to the
window. Extra query flags: `edit=1` (edit mode), `popup=a,b` (open popups by
clicking their widgets), `battery=1`, `vertical=1` (adds a left panel),
`labels=1` (task titles), `demo=1` (shows the hover chrome on the task bar
and the first desktop widget for screenshots).
