# Desktop workspaces

MindOS now uses the supplied modernist design across its desktop, library,
gaming utilities, companion, login and Plymouth boot screen: square panels,
charcoal and green in dark mode, light stone and orange in light mode, bold headings and monospace
metadata. Regular browser, file, video and chat applications remain normal
windows. Dark grey is the dominant dark-mode surface, with green reserved for
highlights and soft shadows. The wallpaper is entirely static; the animated
canvas and its toggle have been removed. Dark/light mode and image selection
remain in Settings → Wallpaper.

Desktop library panels use translucent backdrop blur on hardware renderers.
Software renderers use cached frosted wallpaper bitmaps instead of live blur;
the small bitmap is created once per theme and never animated. Separate shell app
windows use the existing cached frosted-wallpaper layer with a translucent
charcoal tint, so they do not need a continuous capture/blur loop. This is not
compositor-wide blurring of other applications behind a window; third-party
clients keep control of their content. The Start control, shelf, tray, native
title bars and Mind launcher share the neutral grey surfaces. Focused native
window frames add a cached green glow around the outside.

Browser fixture previews: [Gaming Center](img/gaming-center.png) ·
[Frame history](img/gaming-history.png) · [Companion](img/gaming-companion.png).
Installed systems use native service data.

[Work preview](img/productivity-desktop.png) · [Embedded Settings](img/desktop-settings.png)

## Gaming and Work

The header carries the mode switch: one recessed track with two stops
(**Gaming** and **Work**, the productivity desktop) and a lit thumb that slides
onto the live one. It is a radio group -- the arrow keys move between the stops
-- and below 1100px the labels drop, leaving the two glyphs. Switching modes
rebuilds the workspace with a brief 240 ms fade/slide. Reduced-motion and
GameMode disable the transition. No animation runs at rest. The header reads
`MINDOS // <panel title>` and has an icon-only theme switch; hardware readings
appear only in the System panel.

Gaming has the library, sessions, downloads, friends and system tools.
Work has everyday application launchers, recent files from Documents,
persistent desktop notes and this monitor's open windows. Missing applications
open Software settings. The mode and notes are saved in `desktop.workspace` in
layout.json. Switching modes does not close applications or change power profiles.

Settings and Gaming Center open inside the main desktop panel, including launches
from desktop entries and `mindshell --app`. The session socket forwards these
requests to the shell; standalone windows remain a fallback when no shell runs.
The Back button returns to the mode's home panel. Companion stays a separate
window so it can be pinned beside a game.

Only the primary monitor shows the desktop panels, navigation and shelf. Other
monitors show the wallpaper and application windows. Changing the primary display
moves the workspace. Tiling and Columns use persistent monitor ownership,
independent window lists and scroll offsets. Columns are clipped to their own
monitor for drawing and input. Dragging to another monitor transfers ownership;
disconnected monitors fall back to a connected display.

## Screens and connections

| Screen / feature | Implementation |
| --- | --- |
| Library | Installed Steam, Heroic Epic/GOG, Lutris and native desktop games; cached Steam art, source/search/sort/favorites, launch/focus, held-session resume, completion and cold-storage status |
| Desktop sidebar | Local Mind, real CPU/GPU/memory, existing power profiles, managed sessions, Steam manifest download progress, connected Steam friends and launcher shortcuts |
| Gaming Center | Sessions, saves, storage, downloads, party, audio, frame history/comparison, connections, local activity and real systemd boot information |
| Companion | Local/direct HTTPS video, saved per-game guide links and notes, playback controls, right-third snap with game left two-thirds, display selection, restore positions, audio ducking |
| Super+Space | Existing Mind bar now indexes installed games plus session, save and companion actions; natural-language help still uses the configured Mind model |
| Login and lock | Dark/light static wallpaper, standard controller navigation and on-screen password keyboard; greetd/PAM authentication; library or software-rendered recovery session; held-session resume after successful unlock |
| Boot | Plymouth progress and elapsed duration, rolling real status messages, password/question prompts; full systemd timing and slow services in Gaming Center → Activity |

Open **Gaming** from the desktop, **Session / Saves / Companion** from a game's
library card, or run `mindshell --app gaming`, `mindshell --app companion` or
`mindshell --app library`. Keep ordinary files, browser and Discord on the
existing dock. Layout edit mode retains the existing panel/widget editor.

## Sessions and tuning

Gaming Center → Sessions provides the exact launch wrapper for the selected
game. For native Steam, place it in **Properties → General → Launch Options**:

```text
mindos-play run 'steam:APPID' -- %command%
```

For Lutris use its command-prefix field with the prefix before `%command%`.
For other launchers, their wrapper must execute `mindos-play run GAMEID --`
around the actual game command. Existing options belong after `--`. The helper
does not rewrite provider configuration or assume that launching a URI moves an
already-running launcher's children into its own cgroup. Flatpak launchers need
a host-access wrapper compatible with their sandbox; a host-only command is
not automatically available inside Flatpak.

The wrapper starts a MindOS-owned systemd user scope, GameMode and MangoHud
when available, and records the session. **Hold** freezes that scope; **Resume**
thaws it. Other applications are unaffected. The UI reports the actual
transition time. The scope belongs to `mindos-session.target` and ends with the
desktop session. This is in-memory holding, not a game checkpoint that survives
logout, reboot, a GPU reset or a power loss. Online games may disconnect and
some games/anti-cheat systems may not tolerate pausing.

A per-game FPS limit is applied by MangoHud at the next managed launch.
Existing Quiet/Balanced/Performance system profiles and DLSS/FSR/XeSS controls
remain available. Frame History reads real MangoHud CSV logs, graphs sample
frame times and reports average FPS, p99 frame time and slow samples. Very long
traces use at most the most recent 100,000 valid samples. Comparisons require
two recordings of the same game. Mind can receive the measured analysis;
there are no fabricated frame-cost or temperature-saving predictions. MangoHud
logging depends on the game renderer and launcher environment.

## Saves and storage

Set the game's **save folder** in Saves. Backups create separate versions and
verify file contents with SHA-256. Restore verifies the selected revision,
backs up current saves, and keeps the replaced folder. Local and synced-folder
backups appear together. A cloud connection here means an existing folder
managed by your sync client; this does not impersonate Steam Cloud or change
provider cloud-sync settings. Completion can be entered manually or imported
from Steam achievement progress, which may differ from story completion.

Set an existing **cold storage folder** in Connections. Storage shows the
source, destination, required bytes and available space before the Move button.
Moving a closed game copies and verifies its files, then replaces its original
path with a symlink; restoring performs the reverse. A frozen game is still
running and must be closed first. Launcher paths remain connected; cold games
can still be launched from the slower drive. Save directories are separate.
Concurrent managed operations are locked. The original copy is retained until
verification and switching succeed. Interrupted copies and `.mindos-original`,
`.mindos-link` or `.before-restore-*` recovery paths are deliberately retained
on failures; inspect these before retrying rather than deleting the only copy.
Keep the destination drive mounted while a linked game is in use.

## Steam, downloads and party

Connections accepts a SteamID64 and Web API key. The key stays in the user's
mode-0600 configuration file, never in process arguments or status responses.
Steam friend/profile privacy and API permissions still apply. Friends refresh
from the official Web API, with a one-minute local cache. Chat and joining open
Steam or the friend's profile for the user to act; the shell sends no messages
or invitations automatically. Discord and other voice applications remain
available through their native clients.

Download byte counts come from Steam's installed manifest counters. The shell
does not claim a rate or “playable now” threshold it cannot measure. Steam
manages its queue; Heroic and Lutris queue controls open their native clients.
Amazon/sideloaded Heroic libraries remain available through Heroic itself.

## Video and audio

The companion accepts a local video path or a direct HTTPS media URL. Streaming
site pages and guides open in the normal browser, which can also be snapped.
Per-game guide links and notes persist locally. Media is streamed through the
native host through a per-process loopback capability URL with byte-range
support for seeking, without buffering an entire recording in shell memory.
Decoder support comes from GStreamer. Picture in
picture is offered where the WebKit build implements it; compositor pinning is
available independently. Snap controls work on an explicitly chosen display
and restore prior window geometry/fullscreen state. Dragging a title bar to
a display edge also snaps; dragging away detaches it. Games must support
windowed resizing.

Capture, screenshot and OBS recording/replay controls have been removed from the shell and gaming helper. Existing personal media files are preserved.

The mixer lists real PipeWire/PulseAudio application streams. A selected game
stream can be lowered by 40% relative to its original volume during companion
playback. Pause, end and close restore it. A renewed lease and independent
watchdog restore abandoned ducking after approximately 12–14 seconds, including
when the companion crashes. A reused stream with a different process ID is not
restored to another app's volume. Standard-mapped controllers navigate only the
focused shell window; A activates/opens the keyboard, B dismisses it and Y
focuses search. The on-screen keyboard submits the same authenticated form as
a physical keyboard. A button press never bypasses a password.

## State and packaging

- Appearance/favorites: `~/.config/mindos/shell/layout.json`; existing native
  watcher propagates changes between windows. Greeter appearance is temporary.
- Gaming configuration: `~/.config/mindos/play.json` (mode 0600).
- Session records, CSV traces, metadata, backups, storage index and activity:
  `~/.local/share/mindos/play/` (XDG overrides are respected).
- `mindos-games` discovers local launcher libraries. `mindos-play` is a
  structured JSON helper with an explicit action dispatcher; native requests
  use stdin, not shell interpolation. It ships in `mindos-gaming` with the
  `play` Python modules. Gaming Center/Companion desktop entries ship with
  `mindshell`, recovery/library login entries with `mindos-session`, and boot
  styling with `mindos-theme`. Rebuild the initramfs when applying a Plymouth
  theme to an installed system through the normal package/boot workflow.

The native Mind bar's accent also follows `theme.accent` in `mindwm.toml`;
existing custom values take precedence over the new green default.
Third-party applications keep their own themes. The screen studies' universal subsecond
resume, automatic game-save discovery, arbitrary embedded websites, complete
cross-provider social/download APIs and speculative performance predictions
are not platform capabilities this implementation can promise.

## Verification

```sh
npm --prefix mindshell/ui run check
npm --prefix mindshell/ui run build
python3 scripts/tests/test_games.py
python3 scripts/tests/test_play.py
node scripts/tests/gaming-smoke.mjs
node scripts/tests/ui-smoke.mjs
scripts/buildbox.sh bash -c 'cd mindshell && CARGO_HOME=/work/build/cargo-home cargo test --release'
scripts/buildbox.sh bash -c 'cd mindwm && CARGO_HOME=/work/build/cargo-home cargo test --release --lib'
```

`gaming-native-session.py` runs inside a disposable QA VM as the desktop user,
and verifies a real counter stops and resumes in a systemd scope. Native QA also
covers Wayland snapping (including edge dragging, detaching and fullscreen
restoration), the WebKit bridge, media playback and byte ranges,
Plymouth rendering and MangoHud frame-time logging.
Browser smoke tests use explicit fixtures;
those fixtures never supply account or performance data in the native host.
Physical controller mappings, live Steam credentials, Flatpak wrapper setups
and individual games' pause/anti-cheat behavior require their actual devices,
accounts and installations.

The browser regression check verifies that no live wallpaper canvas or toggle
is created, and that desktop panels have a translucent blur effect. Native QA
also measures idle CPU after the desktop has settled.

Protocol references: [Steam Web API](https://partner.steamgames.com/doc/webapi/ISteamUser),
[Steam achievements](https://partner.steamgames.com/doc/webapi/ISteamUserStats),
[MangoHud configuration](https://github.com/flightlessmango/MangoHud/blob/master/data/MangoHud.conf),
[Heroic launch protocol](https://github.com/Heroic-Games-Launcher/HeroicGamesLauncher/blob/main/src/backend/protocol.ts).

Workspace checks cover note persistence, embedded navigation, primary-display migration, static secondary desktops, and independent monitor ownership/column geometry. The QA VM currently has one virtual display; two-display layout behavior is covered by compositor regression tests and browser output fixtures.

`workspace-native.py` passed in the QA VM: three real GTK windows retained their output IDs while focusing through Tiles and Columns; CLI Settings and Gaming Center launches created no application windows.
