# Windows programs on MindOS

Windows programs use Wine, with Proton for Steam games. MindOS integrates
supported programs into the launcher, taskbar and system tray. Compatibility
is app-specific; this does not promise that every Windows program works.

Double-click an EXE/MSI in Files, or use **Settings → Software → Choose installer**.
Choose **Install** for setup programs or **Run app** for portable EXEs. The
helper creates a themed prefix, opens the installer, and publishes shortcuts.
If an installer creates no shortcut, a file chooser can add its installed EXE.
The Mind popup shows app icons, clickable quick launches and Linux/Windows
labels. Its index refreshes in the background, so installs appear without
restarting the desktop. GIO handles desktop-entry paths and escaping.

## What runs where

| piece | job |
|---|---|
| `mindwm` (`src/procinfo.rs`) | knows whether a window's process runs under Wine or Proton (`/proc/<pid>/exe` is a Wine loader, or `WINELOADER` is in its environment) and sends `wine: true` for it in the `windows` IPC snapshot |
| `mindwm` (`src/xtray.rs`) | hosts the XEmbed system tray: owns `_NET_SYSTEM_TRAY_S0` on XWayland, docks icon windows into hidden containers, reads their pixels back through Composite and publishes them as `tray` IPC events; `tray_click` replays a click with XTest under the real pointer |
| `mindshell` (`src/apps.rs`, `src/app.rs`) | reads `StartupWMClass` from desktop entries so Wine's generated entries match their windows, flags entries whose Exec runs `wine`/`proton`, merges the compositor's tray icons (`x11:<window>` ids) with the StatusNotifier ones |
| `mindshell` UI (`widgets/taskbar.ts`) | draws the badge on Wine groups and says "Windows application (Wine)" in the tooltip |
| `mindwm` (`src/launcher.rs`, `src/mindbar.rs`) | the Mind bar tags Wine entries `WINDOWS` |
| `mindos-win` (package `mindos-gaming`) | one themed Wine prefix per program |

## Installing a program: `mindos-win`

```
mindos-win install ~/Downloads/npp.8.9.8.Installer.x64.exe --name "Notepad++"
mindos-win list
mindos-win run notepad-plus-plus
mindos-win remove notepad-plus-plus
```

`install` creates `~/.local/share/mindos/win/<name>/` (a 64-bit prefix booted
without the Mono/Gecko download prompts), applies the theme, runs the
installer (`--silent` passes `/S` or `/qn`), waits for Wine to settle and
lists the desktop entries Wine's menu builder generated from the program's
Start Menu shortcuts. Those entries carry `WINEPREFIX` in their Exec line,
so they belong to that prefix, show up in the dock and the Mind bar at once,
and are removed again by `mindos-win remove`.

`run` starts an entry (the first one, or the one whose name matches the
argument) or, given an `.exe` path, that program in the prefix, and stays in
the foreground until the prefix's wineserver has exited, so a launcher or a
script sees the program's lifetime rather than Wine's. `exec` runs any
Windows command in the prefix (`mindos-win exec notepad-plus-plus winecfg`), and
`tricks` is winetricks in the prefix. `theme` re-applies the look, with
`--dpi N` for HiDPI (also accepted by `install` and `create`).

## The look inside programs

`/usr/share/mindos/win/theme.reg` sets the classic-control colours to the
HUD palette (window `#0a0d12`, buttons `#161d26`, text `#e6edf3`, highlight
`#0aa7c2`), replaces the Windows UI fonts (MS Shell Dlg, Segoe UI, Tahoma)
with Inter and Consolas with JetBrains Mono, turns on grayscale font
smoothing, and flags the dark app theme for programs that read it. Wine
ships no dark msstyles theme, so the classic renderer with these colours is
the dark theme; programs that draw their own chrome (Notepad++'s editor,
browsers) keep their own.

## Tray icons

Wine implements `Shell_NotifyIcon` with the X11 XEmbed tray protocol, not
StatusNotifier. The compositor's tray host in `mindwm/src/xtray.rs` is a
plain X client of the compositor's own XWayland:

1. it owns the `_NET_SYSTEM_TRAY_S<n>` selection and advertises a 32-bit
   visual, so icons draw with alpha;
2. a dock request reparents the icon into a 24x24 override-redirect
   container parked off screen, maps it and sends `XEMBED_EMBEDDED_NOTIFY`;
3. every 400 ms each container is read back (`NameWindowPixmap` +
   `GetImage`; every X11 toplevel is composite-redirected by the window
   manager, so the pixmap exists off screen) and, when the pixels or the
   name changed, a `tray` event goes to the shell;
4. the shell shows the icon in the tray widget next to the StatusNotifier
   items; a click sends `tray_click`, the compositor moves the container
   under the pointer for 600 ms, warps the X pointer there with XTest and
   replays the button, so the program sees a click at the real cursor and
   opens its menu there. Wheel events map to X buttons 4 to 7.

Without a tray host Wine falls back to a floating "system tray" window
of its own, which the compositor would tile; with it, that window never
appears.

Hiding to the tray is the program unmapping its own window. The compositor
then sets the window's `WM_STATE` to Withdrawn (ICCCM bookkeeping Smithay
leaves out): Wine waits for that before it considers the hide finished, and
without it the program's next `ShowWindow` is never turned into an X map
request, so the window would not come back from the tray.

## Limits

* Kernel-driver software and many anti-cheat systems do not work under Wine;
  game support depends on the publisher and its compatibility configuration.
* Wine's own Wayland driver has no tray support; programs started through
  `mindos-win` use the X11 driver.
* XEmbed icons have no menu protocol of their own, so the shell's right
  click is replayed as a right click on the icon, which opens the program's
  own menu.
* The QA probe passes close-to-tray/restore with one second for Wine to settle
  after hiding. An immediate restore intermittently missed the click in the
  VM; this timing race still needs investigation.
* The graphical Windows handler offers Install or Run app and creates a Mind
  shortcut for portable programs. The original EXE must remain at its chosen
  location. `mindos-win run NAME path/to/program.exe` is also available.

## macOS applications

Reliable, seamless macOS GUI support is not available. Darling's own
[status page](https://www.darlinghq.org/) describes basic experimental support
for simple graphical applications. MindOS does not preinstall it or associate
DMG/APP files with a handler that would imply broader support. macOS tray
integration is therefore unimplemented. Prefer an app's Linux or Windows build.
