# mindwm — the MindOS compositor

`mindwm` is the Wayland compositor MindOS boots into. It is a fork of
[Smithay](https://github.com/Smithay/smithay)'s reference compositor
(`anvil`, Smithay 0.7) with MindOS behaviour layered on top:

* **DRM/KMS session** via libseat/logind, libinput, GBM + EGL/GLES (NVIDIA
  and Mesa both work through the normal Linux driver stack), multi-GPU aware,
  direct scanout for fullscreen games. `--winit` runs it nested for development.
* **Gaming pointer input:** relative mice, absolute tablets and the nested
  backend share mouse-lock/confinement handling. Confinement follows the
  surface input region, slides along edges and prevents jumps across holes.
  Output bounds support negative, stacked and gapped monitor arrangements.
  Session locking drops mouse grabs and focus; keyboard shortcut inhibition
  follows the focused application. Startup fullscreen works before the first
  buffer arrives, including output selection on multi-monitor layouts.
* **XWayland** built in, so X11 games and launchers (Steam, Proton/Wine, Lutris)
  run unchanged. A game that opens in borderless fullscreen (it sets
  `_NET_WM_STATE_FULLSCREEN` before mapping, or opens undecorated at exactly
  one display's size) is fullscreen on that display from the first frame. The
  focused X11 window is published as `_NET_ACTIVE_WINDOW`, which is how Wine
  knows its game is in the foreground: without it the game gets no keys and
  its cursor clip is never applied. Wine may move the focus between its own
  windows with a `_NET_ACTIVE_WINDOW` request; another program may not take it.
  Both need the XWM changes in `mindwm/vendor/smithay/MINDOS-PATCHES.md`.
* **Three window layouts**, switched from the shell's top bar (the icon next
  to the clock), with `Super+T`, or in Settings › Desktop, and remembered
  across sessions: *Floating* (like KDE: windows keep their size, open
  centred and cascade, drag them by the title bar), *Tiles* (like Hyprland's
  dwindle layout: every new window splits the focused tile) and *Columns*
  (like Niri: windows line up on a strip that scrolls sideways). Dialogs
  always float, centred over their parent. Fullscreen requests get direct
  scanout.
* **Server-side decorations in the MindOS look**: a 30 px translucent title
  bar with rounded top corners, the window title and minimise / maximise /
  close glyphs, a cyan line under the bar of the focused window. Drawn for every toplevel that negotiates
  server-side decorations through xdg-decoration (Qt, foot, SDL/libdecor,
  Chromium, Firefox, ...), for X11 windows that are not undecorated, and for
  the shell's own app windows (`mindos-*` app ids). GTK applications keep the
  bars they draw themselves, like on KDE and GNOME.
* **Dark glass theme.** The desktop is the MindOS void (`#05070a`) with
  off-white text (`#e6edf3`) and one electric-cyan accent (`#19e3ff`); the
  Mind bar and the title bars are rounded, translucent dark cards with a
  light hairline and a soft accent glow (`docs/THEME.md`). Red is reserved for the kernel console, the boot menu and
  the boot stages and never appears in the session.
* **The startup screen.** From the moment the compositor takes over from
  Plymouth until the shell's desktop is on screen, `mindwm` draws the boot
  splash again: the glowing `MINDOS` wordmark, a progress line with a moving
  head, `STARTING THE DESKTOP`, the sweeping hairline and the HUD corners
  (`[theme] show_wordmark`). The key hints join it if the wait passes six
  seconds. The whole screen disappears the frame the shell maps its desktop.

  What counts as "the desktop is up" is the `mindshell-desktop` namespace on
  any layer, not a surface on the background layer (`shell::desktop_up`), plus
  a 1.5 s latch: the shell moves that one surface between the background and
  the top every time the user asks for the home screen (Super + D), and a
  client that remaps a surface to change layer leaves a frame or two with
  nothing there. Watching the background layer alone put the boot splash back
  on screen for those frames, in the middle of a crossfade. The latch is short
  on purpose -- a shell that actually dies brings the startup screen back.

  It is drawn on the event loop, so what it costs comes out of the time the
  shell it is waiting for needs to start. Everything fixed -- the wordmark,
  the caption, the hints, the corners -- is rasterised once **per output
  geometry** and kept (`startup_art`, logged as `startup screen drawn`); the
  two pieces that move are drawn into the buffer they already have, so no
  texture is allocated per frame. One frame for two displays costs about 30 µs
  of CPU, near enough nothing at 240 Hz.

  Both halves of that are load-bearing, and both were once wrong. A single
  cache slot keyed by output geometry means two displays of different size or
  scale evict each other every frame: the wordmark is display type a couple of
  hundred pixels tall, and re-measuring and re-rasterising it took 95% of a
  core in `rasterize_runs` -- on the one thread that also serves every Wayland
  client and all input. The shell could not start, so the screen stayed up, so
  it kept paying. `gtk::init` went from 4 ms to as much as 13 s and the desktop
  from half a second to twenty-three; the mouse queued behind splash frames.
  The tests in `src/mindbar.rs` hold both contracts: the artwork survives two
  outputs alternating, and a frame stays under the budget.
* **The Mind bar** (`Super+Space`): a software-rendered overlay that is both an
  application launcher and the front end of `mindd`, the local LLM daemon.
* **Idling**: the screensaver, the automatic lock and switching the displays
  off, all counted from the last input event and enforced here (see *Idling*
  below).
* **A shell IPC socket** (`MINDWM_SOCKET`) through which `mindshell`, the
  HTML/TypeScript desktop, lists and drives windows; see `docs/SHELL.md`.

## Focus

Game mode means no clicking around: a window that maps (Wayland or X11) gets
keyboard focus immediately, and when the focused window closes or crashes the
top-most remaining window takes over. Clicking a window still focuses and
raises it; `Super+Tab` cycles, and Super held with the mouse wheel walks the
tiling order on the screen the pointer is on.

Layer-shell surfaces (the shell's panels, popups, wallpaper) follow the
usual rules: a `top`/`overlay` surface with *exclusive* keyboard
interactivity gets every key (the Mind bar popup), one with *on-demand* gets
the keyboard the moment it appears and whenever it is clicked (a context
menu or the layout picker can be dismissed with Escape straight away), and
one with *none* (a plain panel) never takes focus, and clicking it does not
hand the focus to the window underneath either. `bottom`/`background`
surfaces only get focus when clicked and set to on-demand, so clicking the
wallpaper does not steal the keyboard from a game. When a popup that held the
keyboard goes away, the top-most window gets it back, so a question to Mind
does not leave the terminal deaf.

The pointer is re-aimed once per event-loop turn: when a panel maps, a popup
disappears or a window closes under a pointer that has not moved, the next
click still lands on whatever is there now.

Windows can be **minimised** through the IPC (the taskbar): a minimised window
leaves the space, so it is not rendered, gets no input and no frame callbacks
(a minimised game stops rendering), and comes back where it was, on top and
focused. A fullscreen window that is minimised gives its output's direct
scanout back and takes it again when restored.

**Maximised means the usable area**: the output minus the exclusive zones of
layer-shell panels, minus the strip the shell's own bar covers
(`shell::usable_area`). The shell draws that bar inside its desktop window,
which is anchored to every edge and so reserves nothing of itself; it reports
the height instead (`desktop_bar` IPC, `shell::DesktopBar`), and the strip
only counts while a desktop is up on that output, so a shell that goes away
hands the screen back. The rest of the home screen comes and goes with the
windows, but the bar does not — it is on screen in both views — which is why
its room is always held back. A bar that is missing, nonsense, or so tall it
would leave a window under `MIN_ROOM` of height gives the screen back whole:
a wrong measurement must not cost the user the screen. A game is the one
exception: `maximized_area` gives it the whole output, panels included,
because a borderless game is only maximised and is meant to cover everything —
and the panels on that screen go away for as long as it does (below), so there
is nothing left to leave room for. When a panel appears, resizes or goes away,
every maximised window on that output is re-fitted. Tiles, columns, snap zones
and new-window placement all take the same area.

**A layer surface holds its edge while it resizes.** Panels change thickness in
place — a taskbar flyout makes the shell's panel window a couple of hundred
pixels taller for as long as the card is up, edit mode adds a strip — and
`LayerMap::arrange` moves the surface's rect the instant that request commits,
a frame or two before the client has a buffer of the new size. Drawing the old
buffer from the new origin throws a bottom-anchored bar up the screen and back
again: a flash on every hover. So `render.rs` assembles the space's elements
itself rather than calling Smithay's `space_render_elements`, and while a
layer's buffer and its arranged rect disagree it draws the buffer against the
edge the surface is anchored to (`layer_render_loc`). Nothing moves until the
buffer is the size it claims to be. Input still follows the arranged rect,
which is what you want: the card takes the pointer as soon as there is room for
it and stops as soon as there is not.
**A game with the screen takes the panels with it.** Panels are not drawn over
a fullscreen window, and a game is not always fullscreen: it may be maximised
(borderless), or ask for neither and simply size itself to the display. So
every turn of the event loop `refresh_game_screens` asks, per output, whether a
game — a window under Wine/Proton or a `steam_app_*` class — covers the whole
of it (`covers_screen`), and records the answer on the output
(`shell::GameScreen`, read back with `shell::game_screen`). While it is true,
top-layer surfaces on that screen are neither drawn nor clickable nor able to
take the keyboard, exactly as under a fullscreen window: the bar must not sit
over the bottom of a game, and an invisible bar that still swallows the pointer
would be worse than a visible one. The overlay layer is left alone — the lock
screen, the greeter and the menus live there — and summoning the home screen
over the game (Super+D, which puts the desktop window on the top layer) brings
the panels back with it, since that is what the user asked to look at. The
answer is cached on the output rather than worked out in the renderer because
the renderer already holds the layer map; asking it again there would deadlock.

## The pointer

mindwm draws the pointer itself. A client either attaches its own cursor
surface, or — through `wp_cursor_shape_v1`, which GTK 4 and most toolkits now
prefer — names a shape and lets the compositor draw it. Named shapes come from
an XCursor theme (`src/cursor.rs`), animated frames included, loaded on first
use and cached per `(shape, frame)`. That is what keeps one pointer across the
whole desktop: GTK 4 no longer reads XCursor themes of its own, so without the
protocol its windows would show GTK's built-in cursors.

Both kinds of pointer hang off their hotspot — the pixel in the image that is
actually the point being clicked. A client's cursor surface carries its own;
a named shape takes the theme's (`xhot`/`yhot` in the XCursor file), which for
the arrow sits a pixel or two in from the top-left corner but for every resize
and text shape sits dead centre. Drawing either from its top-left corner would
put the point half a cursor up and to the left of the arrows the user aims
with, and window edges would only be caught by overshooting them.

The theme and its nominal size are one desktop-wide setting kept in GSettings; `main`
resolves them once at startup — `XCURSOR_THEME` / `XCURSOR_SIZE` from the
session if set, else the preferences file, else `[theme].cursor_theme` /
`cursor_size` from `mindwm.toml` (`MindOS`, 24) — and puts them back in the
environment, so every program the compositor starts agrees with it.

Settings › Desktop › Pointer changes it live: the shell writes GSettings for
the applications and sends `set_prefs { cursor_theme, cursor_size }`, which
reloads the compositor's own cursor and updates the environment for whatever
it starts next.

## Window layouts and decorations

`src/layout.rs` owns the three modes; the current one is saved in
`$XDG_STATE_HOME/mindos/mindwm.json` (`src/prefs.rs`, with the other
preferences the shell edits: whether the Mind bar shows its tool lines, the
primary output, per-output settings) and announced to the shell as a
`layout_mode` event.

* **Floating** (`floating`, like KDE). Every window keeps the size it asks
  for and opens centred on the output it appears on; a second window that
  would land on the first is cascaded 40 px down and right. Drag by the title
  bar to move, `Super+M` maximises to the usable area (the output minus the
  panels), `Super+F` fills the screen. `Super+Shift+Left/Right` snaps the
  window to that half of the screen (again to release it), `Super+Shift+Up`
  maximises and `Super+Shift+Down` gives it back, and `Super+R` steps it
  through a half, seven tenths and nine tenths of the screen about the middle
  it already has. `[layout].open_maximized = true`
  brings back the old game mode where every new window opens maximised.
* **Tiles** (`dwindle`, like Hyprland). Every window is a tile; a new one
  splits the focused tile along its longer side, so windows spiral inwards.
  `Super+arrows` move the focus, `Super+Shift+arrows` swap tiles,
  `Super+Shift+F` floats the focused window (and tiles it again), and
  `Super+R` steps the focused tile's share of its split through a third, a
  half and two thirds. A window
  remembers the size and place it had before it became a tile: switching back
  to floating (or floating the window itself) puts it back there at once,
  from the compositor, not left to the application's next redraw. A window
  born as a tile gets a centred window three fifths of the output instead.
* **Columns** (`columns`, like Niri). Windows are columns on an endless strip
  that scrolls sideways to keep the focused one in view; `Super+R` cycles a
  column through a third, a half, two thirds and the full width.

Every window can be resized without a modifier: an 8 px ring just outside a
window frame (28 px of it at each end counts as the corner) is the resize
handle, and the pointer changes to the matching arrow over it. The ring is
entirely outside the window, so no client loses a pixel of its own to it, and
it is the only resize handle windows the compositor decorates have — a
terminal draws none of its own. A floating window resizes from the edge under
the pointer; in the tiling modes the ring is the gap between two tiles, and
dragging it moves the divider they share, as `Super+right-drag` does.

`Super+T` cycles the modes. Tiling modes keep `gap` pixels between tiles and
`outer_gap` from the edge of the usable area. Dialogs (toplevels with a
parent, X11 transient and utility windows) float in every mode, centred over
their parent. Maximised and fullscreen windows leave the tiling while they
are maximised.

![Columns: the strip scrolled to the third, focused column](img/shell-columns.png)
![Tiles: dwindle split, close-only title bars](img/shell-tiles.png)

In the two tiling modes the layout owns every tile's place and size, so a
tile's title bar carries only the close glyph (no minimise, no maximise, no
double-click to maximise; `Super+Shift+F` floats the window, which brings
the full set back). The columns strip does not jump:
when the focus moves to a column that is off-screen or half visible, from a
click, `Super+Left/Right`, a new window or the dock, the strip slides there
in 260 ms (ease-out, recomputed every frame). Clicking a dock icon of a
running app focuses that window; in floating mode a second click minimises
it, in the tiling modes tiles are never minimised.

The title bar (`src/shell/ssd.rs`) is a 32 px glass card rendered on the CPU
with the same tokens and fonts as the shell: a translucent fill, a light sheen
over its top half, a 1 px light line along its top edge and a cyan line along
its bottom edge when the window has the focus (a faint white one when it does
not). It is only redrawn when its title, focus, hover or width changes; drag
it to move the window, double-click to maximise (floating windows), and the
glyphs on the right minimise, maximise and close (close only on a tile).

Every decorated window that is not maximised also gets a frame
(`src/shell/frame.rs`): a soft drop shadow, 40 px wide on a floating window
and 12 px on a tile, with a 1 px light ring around the window. The shadow is
darker on the focused window. The frame is a nine-piece set of small images
(four corners, four strips) drawn once per look and scale, then stretched on
the GPU around each window, so a window of any size costs the same eight small
textures to composite. The window itself is drawn over the frame.
Maximised and fullscreen windows have no frame; windows that draw their own
decorations get neither the bar nor the frame.

The ring around a lone window is nearly invisible on purpose, which is the
wrong answer where two tiles meet: over the dark desktop, two dark windows
either side of an 8 px gap read as one wide window. So in the tiling modes
each tile's frame draws a **seam** — the same 1 px band, in `[theme].seam`
(`#edf2f8` by default) at a much higher opacity — on the sides that face
another tile, and nowhere else. The outer edges of a tiling stay hairlines,
and a floating window has no seams at all. `src/layout.rs` works out which
sides touch (`seams_against`: within the layout gap, and overlapping along
the shared edge) as it arranges each output, and the frame cache keys on the
mask, so the eight textures are still shared by every tile of the same shape.
The colour does not follow the focus: both tiles bracketing a gap draw their
half of it, and a pair of matching lines is what makes the gap read as a gap.

A window that asks for
client-side decorations gets none from the compositor; one that never asks
gets none either, except the shell's app windows (`mindos-settings`), which
open undecorated so they get the same bar as
everything else. `Super+Shift+D` toggles the decoration mode of the focused
window.

## Keybindings

| Keys | Action |
|------|--------|
| `Super+Space` | Open/close the Mind bar: Mind is the launcher |
| `Super+W` | The shell's overview (`shortcut overview`); the built-in window preview when no shell is connected |
| `Super+Enter` | Terminal (`[apps].terminal`, default `kitty`) |
| `Super+D` | Bring the shell's home screen forward over the windows, or send it back (`shortcut desktop`). With nothing open it is already the whole screen |
| `Print` / `Super+Shift+S` | Select an area to save and copy; Escape cancels |
| `Shift+Print` | Save and copy all displays |
| `Alt+F4` / `Super+Q` | Close the focused window |
| `Super+F` | Toggle fullscreen on the focused window |
| `Super+M` | Toggle maximize on the focused window |
| `Super+T` | Next window layout (floating → tiles → columns) |
| `Super+Shift+F` | Float / tile the focused window (tiling modes) |
| `Super+R` | Step the focused window's size: a column's width, a tile's share of its split, or a floating window's share of the screen |
| `Super+←↑↓→` | Focus the window in that direction |
| `Super+Shift+←↑↓→` | Move (swap) the focused tile in that direction; a floating window snaps to that half, maximises (up) or is released (down) |
| `Super+Tab` / `Alt+Tab` | Switch recent visible windows; hold the modifier to keep cycling |
| `Super+Shift+Tab` / `Alt+Shift+Tab` | Cycle backward; Escape restores the original window |
| `Super+1..9` | Move the pointer to output *n* |
| `Super+Shift+D` | Toggle server/client-side decorations on the focused window |
| `Super+L` | Lock the screen |
| `Ctrl+Shift+Escape` | Task Manager (raises the open one instead of starting a second) |
| `Super+Shift+E`, `Ctrl+Alt+Backspace` | Quit the compositor (ends the session) |
| `Ctrl+Alt+F1..F12` | Switch virtual terminal |
| `Super+Shift+P` / `Super+Shift+M` | Output scale up / down |
| `Super+Shift+R` | Rotate the output under the pointer |
| `Super+Shift+W` | Built-in window preview (all windows scaled side by side) |
| `Super` + mouse wheel | Step through the windows in the layout order (tiles and columns; nothing in floating) |
| Drag a window's outer edge | Resize it (floating) or move the divider it shares with the next tile (tiling) |

Most shortcuts use `Super`; `Print` and `Shift+Print` capture screenshots, and
`Ctrl+Shift+Escape` opens the Task Manager the way it does everywhere else.
Window switching keeps a stable recent-use order while Alt/Super is held.
Releasing the modifier confirms the selected window; a quick second Alt+Tab
returns to the previously used window. Minimized windows remain in the dock.
Switching away from a fullscreen game reveals the selected application while
preserving the game's fullscreen state for the return trip. Caps Lock does
not alter Super shortcuts, and consumed shortcut keys keep their releases
even when Shift is released first.
For `Super` shortcuts, Super on its own
does nothing, so a tap on it goes to the focused application like any other
key. Other keys reach the application, and clients that use the
keyboard-shortcuts-inhibit protocol get everything.

## Hardware media controls

Unmodified volume, output mute, microphone mute, brightness and playback keys
work without opening a panel. Volume changes in 5% steps, unmutes the output
and caps keyboard adjustments at 100%. Microphone mute targets the default
input independently. Volume and brightness repeat after 400 ms while held;
release stops repetition. Brightness uses the first backlight device and keeps
at least one hardware brightness unit; keyboard LEDs are excluded.

A small dark feedback card appears on the focused window's display (or the
pointer's display), including over fullscreen games. It reuses the bar's fonts
and cached rendering, fades out after 1.8 seconds and never takes focus.
Commands run on one sleeping background worker with bounded execution and
output; unavailable devices or players show “Unavailable”. There is no idle
media polling. Session lock clears feedback and cancels held repetition;
shortcut-inhibited clients keep their keys. Controls are disabled in kiosk mode.

WirePlumber's `wpctl` controls the default audio devices.
[`brightnessctl`](https://github.com/Hummer12007/brightnessctl) handles backlights;
[`playerctl`](https://github.com/altdesktop/playerctl) controls the first available
MPRIS player. Both ship with `mindos-session`; no extra media daemon is needed.

## Screenshots and screen sharing

`Print` (or `Super+Shift+S`) selects an area; `Shift+Print` captures all displays.
Images are saved privately in Pictures/Screenshots and copied as PNG to the
clipboard. Escape cancels without creating a file. Settings → Desktop lists
these shortcuts. Shortcut-inhibited clients keep their keys.

Applications can request a monitor through the ScreenCast portal. The dark
chooser names the available displays and requires Share display or Cancel.
The session uses xdg-desktop-portal-wlr for Screenshot/ScreenCast and GTK for
file dialogs and settings. Capture runs only on request and is refused while
locked, blanked, in the greeter or from security-context-marked clients.

The compositor currently provides wlr-screencopy v3 with shared-memory XRGB
buffers. It caches offscreen targets and waits for damage between unchanged
frames; unused targets expire after two seconds. Portal capture defaults to
60 fps independently of the monitor refresh rate. To tune it, create
`~/.config/xdg-desktop-portal-wlr/config`:

```ini
[screencast]
max_fps=30
chooser_type=dmenu
chooser_cmd=mindos-share-chooser
```

Restart the user portal services or log out/in after changing portal settings.
GPU-buffer capture and the newer ext-image-copy-capture protocol remain open
work; hardware recording performance and individual OBS/Discord integrations
are not established by the VM PipeWire test.

## The Mind bar

* Type to filter installed applications (XDG desktop entries from
  `$XDG_DATA_DIRS`); `Enter` launches the highlighted one, `↑`/`↓`/`Tab` select.
* Anything that is not an application, a query prefixed with `?`, or
  `Shift+Enter`, is sent to Mind. While the request is in flight the header
  reads *THINKING* with a breathing dot and the conversation ends in an
  animated *Thinking* line (the panel redraws every 60 ms until the first
  token). The answer streams in; tool calls show as `⚙ run: nvidia-smi`,
  results as `✓ run_command: …`.
* When Mind wants to change the system (install packages, restart a service,
  edit a config) and autopilot is off, the bar shows *Mind wants to: …* and
  waits for `Y` or `N`. The daemon's policy layer decides what needs confirmation
  and what is forbidden outright; see `docs/ARCHITECTURE.md`.
* `!command` runs a shell command in the session. `Esc` cancels a running
  answer, then closes the bar. `Ctrl+L` clears the conversation.
* Mind can act inside the session through *client tools* the compositor
  registers on connect: `launch_app`, `open_terminal`, `run_in_terminal`.
* Mind answers in Markdown, so its lines are drawn as styled runs
  (`src/markdown.rs`): `**bold**` and headings in the heavy face, `` `code` ``
  and fenced blocks in the accent-coloured mono face, bullets as `•`, links as
  *text (url)*, and the markers themselves gone. Anything it does not
  recognise — an unclosed `*`, an underscore inside a word — stays as it was
  written. Only Mind's own lines are read as Markdown; what you typed is drawn
  as you typed it.

The bar talks to `mindd` over `/run/mindos/mind.sock` (newline-delimited JSON,
see `mindd/src/proto.rs`). If the daemon is not running the bar still works as a
launcher and says so in its status line; it reconnects automatically.

## Idling

The compositor keeps the idle clock, because it is the only process that sees
every key, click and gesture, and it is the one that can switch a display off
and keep windows off the screen. The shell draws the screensaver and the lock
screen; `src/idle.rs` decides when.

Everything is counted from the last input event, in seconds, `0` meaning
never:

```text
 input ────────────── screensaver ────── lock ────── displays off
        idle.screensaver      idle.lock       idle.blank
```

The timings live with the other preferences (`Prefs::idle` in
`$XDG_STATE_HOME/mindos/mindwm.json`), so Settings › Screen changes them with
the ordinary `set_prefs` request and they survive a restart. One calloop timer
is armed for the next deadline; nothing is armed while the session is held
awake or when every timeout is *never*.

* **The screensaver.** The stage goes to `screensaver`, the shell puts a
  `mindshell-lock` overlay on every output and draws a game there. The next
  key or click takes it away — and is swallowed, so the key that woke the
  screen is not also typed into whatever was underneath. The pointer is
  hidden for as long as the stage is not `active`: a client only chooses a
  cursor when the pointer moves, and moving it is exactly what ends the
  screensaver, so leaving it to the shell would park an arrow in one place
  for hours.
* **The lock.** `set_locked` drops the keyboard focus and closes the Mind bar.
  From then on `surface_under`, the focus handling and the render pass only
  consider overlay layer surfaces in the `mindshell-lock` namespace, the
  output is cleared to opaque black behind them, and the IPC refuses anything
  that would start or focus a program. `Super+L` locks; only `Ctrl+Alt+F1..F12`
  still works, so a locked machine can still be recovered from a text console.
  There is no `ext-session-lock-v1` here: GTK — and therefore the shell —
  cannot speak it, so the compositor owns the lock state itself.
* **The displays.** The stage goes to `blank` and the DRM backend clears every
  surface (`DrmCompositor::clear`), which is a real DPMS off, not a black
  picture. Clearing stops the page flips that drive the render loop, so
  rendering is skipped while blanked and restarted with a fresh buffer when
  the screen comes back. A nested session (`--winit`) has no displays to
  switch off, so the screensaver and the lock still work there but blanking
  does nothing.

Two protocols hang off the same clock: `ext-idle-notify-v1`, so a program can
be told how long the session has been idle, and `zwp_idle_inhibit_v1`, so a
video player or a game can hold all of it off. The shell holds the session
awake the same way (`inhibit_idle`) while GameMode reports a running game.
It also asks for the game's screen to be cleared (`game_scene`): see
*A game gets a screen to itself* below.
Both only count while *Stay awake while something is playing* is set.

Where it all stands is broadcast as the `idle` event
(`{ stage, locked, inhibited, saver }`) and can be asked for with `get_idle`;
`lock`, `unlock`, `wake` and `blank` drive it from the shell.

## A game gets a screen to itself

When GameMode reports a running game the shell also sends `game_scene`, and
the compositor moves everything else off the game's screen onto the others:
the chat window, the browser playing something and the terminal all end up
next door, where they can be seen, instead of stacked behind the game.

The game's screen is the one with a fullscreen window, failing that the one
with a maximised window (a borderless game is only maximised), then the
focused window's screen, then the pointer's. On that screen everything moves
except the game itself (whatever is fullscreen or maximised), the focused
window when nothing there is fullscreen or maximised, and the launchers — Steam, Heroic, Lutris, Bottles, gamescope and the
`steam_app_NNN` ids XWayland gives a Steam game — since a launcher on that
screen is quite possibly what started the game a second ago. Everything else
goes to the nearest other screen; a tile is placed by the layout there, and a
floating window keeps the offset it had from its old screen's corner, trimmed
to fit.

None of it is permanent. Each window that moves remembers where it came from
and what geometry it had (`TileData::away`), and `game_scene` with `on: false`
puts it all back when the game ends — including after a game that crashes,
since the counter falls either way. A window the user has moved somewhere
else in the meantime is left where they put it, and a window whose old screen
has been unplugged stays put too. With one display there is nowhere to move
anything to and the request does nothing.

The shell waits `GAME_SETTLE` (2.5 s) after the counter goes up before asking,
because GameMode counts the launcher's process before the game has a window,
let alone a fullscreen one, and the compositor picks the screen by looking at
the windows. A game that starts and stops inside that wait never moves
anything. Coming back is immediate.

Windows opened *during* a game are not steered: the desktop is the user's, and
dragging something onto the game's screen on purpose should stick.

## The shell IPC

`mindshell` (and anything else in the session) talks to the compositor over
`$XDG_RUNTIME_DIR/mindwm-<wayland socket>.sock`, exported as `MINDWM_SOCKET`
to every program mindwm starts and imported into `systemd --user` by
`session-startup`. Newline-delimited JSON, one request per line, replies echo
the request's `id`; subscribers get `windows`, `outputs`, `layout_mode`,
`prefs`, `idle`, `shortcut`, `mindbar` and `mind_status` events, and can change the
layout mode, the preferences and the outputs (mode, scale, position,
rotation, VRR, primary) from the Settings app. The full request/event tables are in `docs/SHELL.md`
("The compositor IPC"); `src/ipc.rs` implements them.

* Window ids are stable for the life of a window and follow creation order.
  Override-redirect X11 windows (menus, tooltips) and toplevels that have not
  drawn yet are not listed.
* Snapshots are compared after every event-loop turn and only sent when a
  listed field changed, so an idle desktop costs nothing.
* A stalled subscriber (4 MiB of unread events) or one that sends a line over
  64 KiB is disconnected.

Try it by hand:

```sh
printf '{"id":1,"type":"subscribe"}\n{"id":2,"type":"terminal"}\n' | socat - UNIX-CONNECT:"$MINDWM_SOCKET"
```

## Configuration

`/etc/mindos/mindwm.toml`, overlaid by `~/.config/mindos/mindwm.toml`
(and `$MINDWM_CONFIG`):

```toml
[startup]
# spawned through `sh -c` once the Wayland socket and XWayland are up;
# WAYLAND_DISPLAY and DISPLAY are set in their environment
exec = ["/usr/lib/mindos/session-startup"]

[apps]
terminal = "kitty"

[layout]
mode = "floating"        # floating | dwindle | columns (the saved choice wins)
gap = 8                  # pixels between tiles
outer_gap = 8            # pixels between the tiles and the usable area
open_maximized = false   # floating mode: open every new window maximised

[mind]
socket = "/run/mindos/mind.sock"
autopilot = false      # true: apply "change" actions without asking

[theme]
background = "#05070a"   # the MindOS void
foreground = "#e6edf3"
accent = "#19e3ff"       # Mind bar lines, selection, wordmark glow
seam = "#edf2f8"         # the side of a tile that touches another tile
show_wordmark = true     # the startup screen, until the shell's desktop is up
cursor_theme = "MindOS"  # the pointer; Settings > Desktop > Pointer overrides both
cursor_size = 24

[session]
kiosk = false            # true for the login screen (mindos-greeter): no Mind bar,
                         # no launcher, no shortcut/IPC that starts a program

[graphics]
direct_scanout = "matching"  # any | matching | off, see "The repaint loop" below
```

`MIND_SOCKET` in the environment overrides `[mind].socket`; `MINDWM_CONFIG`
names one more file loaded last (the greeter uses
`/etc/mindos/greeter/mindwm.toml`).

## Layout of the sources

| File | What it does |
|------|--------------|
| `src/main.rs` | Backend selection (`--tty-udev` on a TTY, `--winit` nested, auto-detected) |
| `src/config.rs` | Config loading and merging |
| `src/mindbar.rs` | Mind bar state machine and CPU rendering (panel, results, conversation, startup screen) |
| `src/text.rs` | fontdue text rasteriser into premultiplied BGRA memory buffers; embedded Inter (body and labels), JetBrains Mono, Orbitron (display) and DejaVu Sans (glyph fallback); macOS-style compositing (gamma-corrected coverage, subpixel glyph placement); anti-aliased rounded rectangles, chamfered rectangles and glow lines |
| `src/ipc.rs` | The shell IPC socket: framing, request/event types, calloop wiring, request handlers |
| `src/markdown.rs` | Just enough Markdown for the Mind bar: inline emphasis, code, headings, bullets, fences and links → styled spans |
| `src/launcher.rs` | Desktop-entry index and ranking |
| `src/mind.rs` | Threaded client for `mindd`; events arrive through a calloop channel |
| `src/edid.rs` | Minimal EDID parser for output make/model (replaces libdisplay-info) |
| `src/layout.rs` | The three window layouts: tile order per output, dwindle and column geometry, focus/move by direction, floating toggles |
| `src/prefs.rs` | Preferences the shell edits (`$XDG_STATE_HOME/mindos/mindwm.json`): layout mode, Mind tool lines, primary output, pointer theme and size, per-output settings, the idle timings |
| `src/idle.rs` | Idling: the stage machine (active → screensaver → blank), the lock state, the calloop deadline timer, `ext-idle-notify` and `zwp_idle_inhibit` |
| `src/shell/mod.rs` | New-window placement (`initial_state`, `centered`, `cascade`, `pointer_output_area`), usable-area relayout when layer-shell exclusive zones change |
| `src/shell/ssd.rs` | Server-side decorations: the title bar renderer and its pointer handling |
| `src/shell/xdg.rs`, `src/shell/x11.rs` | xdg-shell and XWayland window management |
| `src/input_handler.rs` | Keybindings (`process_keyboard_shortcut`), Super+wheel window stepping, layer focus rules and Mind bar key routing |
| `src/cursor.rs` | The pointer the compositor draws for a named shape: XCursor lookup, animation frames, and `configure` (the session-wide `XCURSOR_THEME` / `XCURSOR_SIZE`) |
| `src/render.rs` | Output element assembly: cursor, Mind bar overlay, windows, startup screen; layer surfaces pinned to their anchored edge while they resize (`layer_render_loc`) |
| `src/udev.rs`, `src/winit.rs` | DRM/KMS and nested backends (from anvil) |

## Development

```sh
cd mindwm
cargo build --release
# nested, inside any Wayland/X11 session; talks to a test daemon if MIND_SOCKET is set
MIND_SOCKET=/run/user/1000/mind-test.sock ./target/release/mindwm --winit
# preview the Mind bar and startup screen rendering without a display
# (writes bar-empty/bar-launcher/bar-chat/desktop .ppm files)
MINDWM_PREVIEW_DIR=/tmp/preview cargo test --release --lib renders_preview
# IPC framing/reply/socket unit tests
cargo test --release --lib ipc
```

For a nested run without the MindOS session scripts, point `MINDWM_CONFIG`
at a file with `[startup] exec = []` and your terminal in `[apps]`; the IPC
socket then appears as `$XDG_RUNTIME_DIR/mindwm-wayland-<n>.sock`.

## The repaint loop, and what happens when a display stops

Each output draws on its own. One frame is in flight at a time: mindwm renders
it, hands it to the kernel, and the page flip's vblank event arms the timer for
the next one. An output with nothing to draw stops repainting and ticks once a
second instead; anything that changes the screen pulls that tick forward.

The weakness of that design, which mindwm shares with niri, Hyprland and KWin,
is that the vblank is the only thing that continues the loop. A page flip the
kernel accepts but never completes leaves the output waiting for an event that
is not coming, and the display freezes for the rest of the session while every
other output keeps working. It happened on MindOS on 2026-09-10, on the 4K
240 Hz primary, with the compositor alive and idle and nothing in the kernel
log.

The second monitor froze the same day, with the deadline and the watchdog
already in place, and neither of them said a word. That one was not a lost
vblank: the CRTC took no atomic commits at all, so there was no flip to time
out, and the output held a repaint timer that was not going to fire for a very
long time — which the watchdog read as a healthy loop. The schedule had run
away, one frame at a time, from a presentation timestamp aimed into the future:
each frame is planned one refresh after the last target, so a wrong target is
inherited by every frame after it and never comes back. Nothing capped the
resulting delay, and switching the displays off and on kept the poisoned target,
which is why that recovery bought one or two frames and then froze again. The
four points below about timers and timestamps are the answer to that, and the
reason the watchdog now asks when a timer will fire rather than whether one
exists at all.

The journal from that morning also carries the loop's fingerprint, twice,
minutes before the display stopped:

```
WARN calloop::loop_logic: Received an event for non-existent source
```

That is a repaint timer that had already fired being removed before its event
was dispatched, which happens whenever a repaint kicked off from elsewhere
supersedes an armed one. The event is dropped, and in the old code a repaint
that then failed armed nothing in its place. A vblank that arrives early is
held back by a timer of its own, and that timer used to be removed by the very
callback it was dispatching, which drops the same source twice and is the
second way that warning is earned; it now forgets its token and lets calloop
drop it once.

So mindwm no longer trusts the vblank alone:

* **Every page flip has a deadline.** Thirty frames, and never less than a
  second. If it passes, the output is reset: its surface is cleared, its
  buffers dropped, and it is drawn again from scratch, which is the same thing
  that switching the displays off and on does. KWin waits the same second and
  gives the same reason, that a second "should always be longer than any real
  pageflip can take, even with PSR and modesets"; unlike KWin, which logs and
  keeps waiting, mindwm resets the output.
* **No error stops the loop.** A frame the hardware refuses (busy, out of
  slots, momentarily owned by someone else, a rejected atomic test) schedules
  the next frame instead of waiting. Only a paused session stops repainting,
  and its resume handler starts every output again. Nothing in the render path
  panics any more: a lost rendering context retries once a second rather than
  taking the session down.
* **The scan-out that froze an output loses the privilege.** If the frame that
  never reached the screen had a client's own buffer on the primary plane, that
  output composes from then on and says so in the journal. A client's buffer on
  the plane waits on a fence only that client can signal, and the kernel will
  not time it out; a composed frame waits on the GPU instead, where the driver
  does. A game that has to be composed costs a little latency. A monitor that
  never updates costs everything.
* **A slow watchdog covers the rest.** Every half second, an output that is lit
  but has neither a timer armed nor a frame in flight, or a flip older than two
  seconds, is reset the same way. It is the backstop for the deadline itself
  failing to arm.
* **What the watchdog really asks is whether a frame was drawn.** Every theory
  about which timer should have fired or which event went missing is a theory
  that can be wrong, and both freezes got past a watchdog that reasoned about
  the loop's bookkeeping instead of its output. Every lit display repaints at
  least once a second, so an output that has drawn nothing for two seconds has
  stopped — no matter how healthy its timers look — and is reset. That check
  needs to know nothing about *why*, which is what makes it the one that
  catches the freeze nobody has thought of yet.
* **An armed timer only counts as alive until it is due.** The watchdog used to
  take the existence of a repaint timer as proof that a frame was coming. It is
  not: what matters is *when* it fires. An output whose timer is more than a
  second past due, for two ticks in a row, has stopped, and is reset like any
  other. Two ticks, so that a loop merely blocked for a moment — a modeset, a
  slow GPU — is not reset out from under itself.
* **No repaint is ever scheduled more than a second out.** The delay before the
  next frame is worked out from the last presentation time, so a bad
  presentation time makes it arbitrarily long. A frame is milliseconds and the
  idle tick is a second, so anything longer is a mistake by definition and is
  clamped to the idle tick. Drawing a second late is recoverable; not drawing
  again is what this whole loop exists to prevent.
* **A vblank time that is not near the clock is not believed.** Each frame is
  aimed one refresh after the last target, and the one after that at one refresh
  after *that*, so a single timestamp from the future is not a late frame: it is
  a schedule that runs away and takes the output with it. A reported time more
  than 50 ms ahead of the clock, or more than a second behind it, is dropped in
  favour of the clock, and the journal says so once.
* **A change is drawn now if the repaint already armed is far off.** A commit
  used to pull the repaint forward only when the output was on its idle tick.
  Now it does so whenever nothing is in flight and the armed repaint is more
  than 200 ms out, which is the same judgement the watchdog makes, a little
  earlier and without resetting anything.
* **Every state change that strands a flip drops it.** Switching the displays
  off, and resuming after a VT switch or suspend, both cancel the flip in
  flight rather than let its deadline fire on a display that is off on purpose.
  This is what aquamarine calls invalidating the frame, and Hyprland's comment
  on it describes exactly the black-screen-after-resume this avoids.
* **A device that will not take frames is taken again.** Resuming a session
  activates each DRM device, and that is allowed to fail — another compositor
  still holding the master, a device still coming back from suspend. It used
  to fail silently and leave every display on that GPU dark for the rest of the
  session, with the watchdog skipping the device precisely because it was
  inactive. Now a device that is still inactive two seconds into a running
  session is activated again, every two seconds, and each success throws away
  the state that belonged to before and repaints every output on it.
* **A display coming or going never takes the others with it.** Plugging a
  monitor in, unplugging one and changing a mode all used to reach for a
  renderer with `unwrap`, so a GPU that was busy at that moment ended the
  session. They handle it now. Restoring the modifiers after an unplug
  modesets every other display on the same GPU, which strands whatever frame
  each of them had in flight: they are all repainted instead of waiting a
  second for their deadlines. A mode change throws away the frame times from
  the old refresh rate, which would otherwise aim the next frame at a vblank
  from a cadence that no longer exists.

### When every display stops at once

Everything above watches one display, and every bit of it runs on the
compositor's single event loop. So none of it can help with the other freeze:
the loop itself stopping. Input, repaints, clients, the watchdog that would
have reported it — all of them are that one thread, and all of them stop
together. It has never been seen here, but it is the only failure left that
would take both monitors at once, and it cannot be reasoned about from inside.

Any call that waits inside a source's callback can do it. The XEmbed tray host
was the clearest: it read every icon back over X11 every 400 ms, and an X11
request that wants an answer blocks until XWayland sends one. That is a
round-trip landing in the middle of a frame — at 240 Hz the whole frame is
4.17 ms — forever, for as long as the session lasts. Worse, XWayland is itself
a Wayland client of this compositor, so the two can wait on each other: the
compositor blocked reading an X reply is a compositor that is not reading
XWayland's Wayland socket, and XWayland blocked writing to it is an XWayland
that will never send the reply. Neither ever moves again.

So the tray host runs on its own thread now, with its own X11 connection and
its own event loop, and the compositor's end of it is two channels: commands
out, icons in. Nothing on the compositor's side of those channels touches
X11, so an XWayland that stops answering costs the tray icons and only the
tray icons. This is what KDE does too, with `xembedsniproxy` as a separate
process entirely.

What is left of the pattern is smithay's own X11 window manager, which does
make round-trips on the loop. Unlike the tray they are event-driven — an X11
window mapping, resizing, setting a property — rather than a timer that fires
whether or not anything happened, so the exposure is a great deal smaller,
but it is not nothing, and it is upstream.

Which is why a thread outside the loop watches its pulse. The loop stores a
timestamp every turn; the thread wakes four times a second and reads it. Two
seconds of silence is reported — the loop is never quiet for that long even
with nothing to do, because the output watchdog's own timer wakes it twice a
second — and the report says *where* the loop is stopped, because the kernel
will tell any thread of a process where its siblings are:

```
ERROR mindwm::watchdog: the event loop has not turned: every display is
  frozen until it does silent_ms=8811 stopped_at=read waiting in
  sock_wait_data on socket:[41283]
```

Blocked in a read on a socket is a different bug from blocked in an ioctl on
the card, and without that line the difference is hours. It only reports; it
does not act. Killing whatever the loop is waiting on would as often turn a
hitch into a crash, and a stall long enough to see is a bug to fix rather than
a state to recover from. `get_graphics` carries the worst one of the session
as `loop_stall_ms`, which is zero on a session that has never stopped.

### How late the repaint starts

Waiting before repainting is what keeps latency down: a client driven by frame
callbacks only draws once the compositor has, so every millisecond the
compositor waits is a millisecond fresher the frame that reaches the screen.
Anvil waits a flat 0.6 of a frame. At 240 Hz that is 2.5 ms of a 4.17 ms frame
and leaves 1.7 ms to render, commit and have the kernel take it; miss that and
the frame lands a whole refresh late, which is what stutter is.

So mindwm measures instead. Each output keeps the time its last thirty-two
repaints took and starts the next one early enough for the slowest of them,
plus half a millisecond for the kernel, never later than anvil's 0.6 and never
leaving clients less than a tenth of the frame. A quiet desktop keeps anvil's
latency; a 4K output under load starts earlier of its own accord and stops
dropping frames, and comes back down when the load does.

The other half of frame pacing is not doing the work at all. Every client
commit asks every output for a repaint, and a display with nothing on it that
changed still has to collect its elements and work out that it has no damage
before it can say so. Three clients drawing at 240 Hz would have each display
do that seven hundred times a second, on the one thread that also has to render
the display that did change. So a repaint pulled forward by a change is pulled
no further forward than one frame after the last one: the update is on screen
within a refresh either way, and the time goes to the output that is moving.
An output with a frame in flight is untouched by this — its vblank already
paces it — and a fullscreen game still goes straight to the screen on commit.

`get_graphics` over the IPC socket reports what actually happened, per output:
frames presented, how many repaints it took to produce them, how many landed a
refresh or more late, how long the repaints are taking now and at their worst,
and how many times the output has been reset.

```sh
python3 -c 'import os,socket
s=socket.socket(socket.AF_UNIX); s.connect(os.environ["MINDWM_SOCKET"])
s.sendall(b"{\"id\":1,\"type\":\"get_graphics\"}\n"); print(s.recv(65536).decode())'
```

`late_frames` is the number to watch: a display that is smooth counts almost
none, and one that counts them steadily is one whose repaint does not fit in
its frame.

`[graphics] direct_scanout` chooses how much of a frame an output may hand
straight to the display hardware:

| value | meaning |
| --- | --- |
| `any` | a fullscreen client's buffer goes to the plane whatever its format, so an 8-bit game still scans out on a 10-bit display |
| `matching` | only a buffer whose format the plane already has, which is what Smithay defaults to (the default) |
| `off` | compose every frame |

`MINDWM_DISABLE_DIRECT_SCANOUT=1` in the environment forces `off` without
touching the config, for a machine that will not behave.

`matching` is the default because it is what mindwm ran before `any` arrived
on 2026-09-09, and the first display to stop with a client buffer on its plane
did so the next day. With a 10-bit swapchain, `any` changes the primary
plane's format every time a game starts or stops scanning out: a popup over
the game is enough. The price is that most games hand over 8-bit buffers, so
on a 10-bit display they are composed, at the cost of one more copy of the
screen per frame.

niri defaults to the relaxed format matching of `any`, also with a 10-bit
swapchain, and offers `restrict-primary-scanout-to-matching-format` and
`disable-direct-scanout` as debug switches. Hyprland disables direct scan-out
by default and only enables it, on `auto`, for a fullscreen window that
declares itself a game. KWin re-decides per frame and proves each choice with
a test-only atomic commit first, falling back to composing on any rejection.

When an output does freeze, the journal is the place to start:

```sh
journalctl -b _COMM=mindwm | grep -i 'resetting the output'
```

The line names the output, how long the flip waited, whether a client buffer
was being scanned out, and how long ago a frame last reached the screen. An
output can also be recovered by hand without ending the session, by switching
the displays off and on over the IPC socket:

```sh
python3 -c 'import os,socket,time
s=socket.socket(socket.AF_UNIX); s.connect(os.environ["MINDWM_SOCKET"])
s.sendall(b"{\"id\":1,\"type\":\"blank\"}\n"); time.sleep(0.3)
s.sendall(b"{\"id\":2,\"type\":\"wake\"}\n"); time.sleep(0.3)'
```

(`socat - UNIX-CONNECT:"$MINDWM_SOCKET"` does the same where socat is
installed; it is not part of a MindOS install.)

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

Everything mindwm draws itself (title bars, window frames and shadows, the
Mind bar, the desktop wordmark) is rasterised on the CPU into small images
that are uploaded once and composited by the GPU together with the windows,
so a decoration costs nothing per frame beyond a textured quad. The GLES
context is requested at high priority so the desktop keeps its frame rate
while a game is running.

## Boot hand-over

`greetd.service` gets a drop-in from `mindos-session`
(`/usr/lib/systemd/system/greetd.service.d/mindos.conf`) so the hand-over from
the Plymouth splash to the compositor never shows the red kernel VT. Upstream
greetd is ordered after `plymouth-quit-wait.service`, so Plymouth would restore
the text VT before the session starts. The drop-in conflicts with
`plymouth-quit.service`/`plymouth-quit-wait.service` and starts right after
`plymouth-start.service`; before greetd starts it runs `plymouth deactivate`
(plymouthd drops DRM master but the splash stays on screen, so mindwm never
races it for the display) and `/usr/lib/mindos/greetd-vt`, which turns tty1
into white-on-void. That matters because greetd resets its VT to text mode and
clears it before every session: the cleared console is what is visible between
the splash and mindwm's first frame, and it is now the same dark colour as the
desktop instead of boot red. Once greetd is up, `plymouth quit --retain-splash`
ends plymouthd. If greetd fails, `plymouth-quit.service` runs (`OnFailure=`) so
the console is reachable.

greetd's greeter is `mindos-greeter`: this compositor in kiosk mode showing
`mindshell --app greeter`, the MindOS login screen (see docs/SHELL.md, *The
login screen*). It runs as the `greeter` user, so a compositor crash there
simply restarts the login screen.

In a user's session a crash does not end the login either. `mindos-session`
starts the compositor again when it is killed by a signal (the loop watchdog
aborts, SIGABRT) or exits non-zero (70 is the compositor giving up on a panic it
cannot get past), after stopping `mindos-session.target` so the shell, the
portals and the autostart applications come back against the new display, as
at login. The windows do not survive it. A logout (exit 0) or a signal to
`mindos-session` itself ends the session as before, and so does a failure after
three restarts inside ten minutes, which returns to the login screen rather
than restart a compositor that cannot stay up. Each restart is in
`journalctl -t mindos-session`, with the status, the signal and the count.

## Debugging on the live ISO

* `journalctl -t mindwm` has the compositor log (`mindos-session` pipes it
  through `systemd-cat`).
* `ls $XDG_RUNTIME_DIR/mindwm-*.sock` shows the shell IPC socket; `socat` or
  `nc -U` and a `{"type":"get_windows"}` line show what the shell sees.
* `Ctrl+Alt+F2` is a root shell on the live ISO; in QEMU with
  `-serial file:...` anything redirected to `/dev/ttyS0` lands in that file.
* A crash restarts the compositor in the same login (`journalctl -t
  mindos-session` says why and how often). A failure after three restarts
  inside ten minutes drops back to greetd, which shows the MindOS login screen
  on VT 1 (`journalctl -t mindos-greeter` for its compositor, `journalctl -t
  mindshell` for the page). greetd only runs the autologin `initial_session`
  once per boot; to re-run it after fixing something, `rm /run/greetd.run &&
  systemctl restart greetd`; without the `rm` the restart shows the login
  screen.
* A VT switch pauses the session: rendering stops until the VT comes back
  (no repaint retries while inactive).
* To try a new build without rebuilding the ISO, ship the stripped binary
  on a second disk (`mke2fs -q -F -t ext4 -d dir img`, attached as
  `/dev/vdb`) and from the root shell run `mount /dev/vdb /mnt && cp
  /mnt/mindwm /usr/bin/mindwm.new && mv -f /usr/bin/mindwm.new
  /usr/bin/mindwm`, then restart greetd as above. Copy-then-rename matters:
  copying straight over the running binary fails with "Text file busy" and
  leaves the old one in place, so check `md5sum /usr/bin/mindwm`.
