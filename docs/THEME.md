# The MindOS look

The shell's current design is documented in [Gaming desktop](GAMING-DESKTOP.md).
It has dark/light modes, square surfaces, orange accents and an optional live
circuit background. Native application themes and boot visuals remain
configured by the components described below.

MindOS has distinct boot and desktop stages:

* **Boot stage: white on MindOS red.** The boot loader and the kernel console
  paint white text (`#ffffff`) on MindOS red (`#8c1010`). Red means "the
  machine is still booting".
* **System stage.** Plymouth and the compositor retain their graphite/cyan
  configuration. The shell uses warm graphite or light stone, orange action
  buttons and Inter text, with the game library as its desktop home.

## Boot stage (red)

| Stage | How | Where |
| --- | --- | --- |
| Limine (installed system) | `term_background: 008c1010`, white foreground and palette, the selected entry inverted to red on white; `mindos-boot config` folds the file into `/boot/limine.conf` | `packages/mindos-theme/limine-theme.conf` |
| GRUB (UEFI ISO) | `set color_normal=white/black` on a `background_color 140,16,16` menu (black is transparent in gfxterm), highlight `red/white` | `iso/grub/grub.cfg` |
| syslinux (BIOS ISO) | red menu with white text | `iso/syslinux/` |
| Kernel console | `linux-mindos` carries a patch that makes the VT default attribute white on red and sets the palette's red to `#8c1010`, so every message from the first kernel line onwards is white on red. On a stock kernel the same look comes from `vt.color=0x4f vt.default_red=... vt.default_grn=... vt.default_blu=...` | `packages/linux-mindos/` |
| Virtual consoles after boot | `mindos-console-theme.service` re-applies the colours to tty1–6, so the tty2 recovery shell stays red | `packages/mindos-theme/console-theme` |
| Login screen | orbital graphite like the desktop: cyan arcs, a dark card and clear Inter text; rendered by the shell's own UI stack (`mindshell --app greeter`) under mindwm in kiosk mode | `mindshell/ui/src/greeter.ts`, `docs/img/greeter.png` |

## System stage (orbital graphite)

### Tokens

| Token | Value | Used for |
| --- | --- | --- |
| void | `#080a0e` | screen clear colour, deepest background |
| bg-0 | `#0d1016` | panels, the Mind bar |
| bg-1 | `#141820` | raised surfaces, progress track |
| hairline | `#2a3442` | 1 px borders and separators |
| line-strong | `#3c4a5c` | emphasised borders |
| fg | `#edf2f8` | text |
| fg-dim | `#adb8c9` | secondary text |
| fg-faint | `#8b98ac` | captions, hints |
| accent | `#67dce5` | the one accent: progress, selection, focus, glow |
| accent-dim | `#48b9c4` | accent on dark surfaces |
| mind | `#a78bfa` | anything the Mind (LLM) says or does |
| warn | `#ffb454` | tool calls, confirmations |
| danger | `#ff5d8f` | errors (deliberately not red) |
| ok | `#3ddc97` | success |

### Surfaces

Settings uses opaque graphite for predictable text contrast. Cards are a step
lighter than the page; navigation has a subdued wallpaper tint. Cyan identifies
focus, selection and primary actions. Secondary text is light slate, and
Inter carries all labels, headings and descriptions. Orbitron is reserved for
the desktop and login wordmarks. The overview has quick links and a button
that launches the configured native terminal.

The default bottom shelf is 64 px tall, floats 8 px from the screen edge,
and has an opacity setting of 0.9. Resting panel contents stay at 94 % opacity
so status text remains readable. Existing customized layouts are preserved.
Popups, the shelf and login retain dark tinted glass with fine borders and
short interaction transitions. Reduced-motion and gaming quiet mode continue
to suppress animation.

| Where | Surface |
| --- | --- |
| Desktop widgets | dark tint over `backdrop-filter: blur(28px)` |
| Panels and popups | cached wallpaper crop from `glass.ts` under the dark tint |
| Window frames and Mind bar | compositor-drawn graphite with cyan focus accents; title bars are 36 px tall with 14 px Inter text |
| Settings | opaque page and cards; a nearly opaque sidebar tint |
| GTK applications | matching named colors in `mindos-apps/gtk-{3,4}.0.css` |
| Terminal | Kitty at 97 % opacity, 12 pt JetBrains Mono and 14 pt padding, configured in `packages/mindos-session/kitty.conf` |

The built-in **MindOS Circuit** wallpaper uses diagonal CSS gradients in
`--aurora` and a togglable procedural canvas in `live-background.ts`. Animation
is capped at 20 updates per second and pauses for GameMode, fullscreen windows,
hidden WebViews and reduced motion. Custom image wallpapers remain static.
The earlier aurora remains available as the packaged `wallpaper.png`, generated
by `mindos-theme/gen-assets.py`.

Kitty merges `/etc/xdg/kitty/kitty.conf` before the user's
`~/.config/kitty/kitty.conf`, so personal preferences take precedence
([Kitty configuration loading](https://sw.kovidgoyal.net/kitty/invocation/#cmdoption-kitty-config)).
The compositor shortcut, Mind launcher, desktop menu, dock, Files menus and
live welcome use Kitty. Shell commands are quoted and run through `sh -c`
inside the terminal. The compositor and shell terminal settings can still be
overridden; existing `/etc` configuration changes may produce `.pacnew` files
when upgrading and should be merged in the usual way.

### Dark mode for applications

Every toolkit is told the desktop is dark, from `mindos-session`:

| Toolkit | Mechanism |
| --- | --- |
| GTK 3 / GTK 4 | `/etc/mindos/xdg/gtk-{3,4}.0/settings.ini` (via `XDG_CONFIG_DIRS`): `gtk-theme-name=Adwaita` with `gtk-application-prefer-dark-theme=1` (the built-in dark variant; GTK 3 has no theme *named* Adwaita-dark and would fall back to light), the same name in the gschema override for GTK 3 on Wayland, which reads it from GSettings, `breeze-dark` icons |
| libadwaita, GTK 4, Firefox, Electron | the Settings portal: `/usr/share/xdg-desktop-portal/mindos-portals.conf` picks `xdg-desktop-portal-gtk`, which reports `org.gnome.desktop.interface color-scheme` — defaulted to `prefer-dark` by `/usr/share/glib-2.0/schemas/90_mindos.gschema.override` (also the Inter / JetBrains Mono font names) |
| libadwaita, GTK 4 | the MindOS colours: `/usr/share/mindos/gtk/gtk-4.0.css` (`mindos-apps`) sets libadwaita's named colours (`--accent-bg-color`, `--window-bg-color`, `--headerbar-bg-color`, … and the `@define-color` names for older apps) to the theme tokens: teal accent with dark text, bg-1 windows, bg-0 views, bg-2 header bars and popovers, hot pink destructive. GTK reads `gtk.css` only from `~/.config/gtk-4.0/`, so `/etc/xdg/mindos/autostart/10-mindos-gtk-css` writes a one-line `@import` there on first login (and a GTK 3 one importing `gtk-3.0.css`, which overrides Adwaita-dark's `theme_*` colours). Delete the import to opt out. The accent is also announced through the portal: `accent-color='teal'` in the gschema override |
| Firefox | `/usr/lib/firefox/defaults/pref/mindos.js`: `ui.systemUsesDarkTheme=1`, dark toolbar and content themes, `prefers-color-scheme: dark` for pages, the compositor's title bar instead of Firefox's own |
| Kitty | the palette in `/etc/xdg/kitty/kitty.conf` |

Users override any of these in the usual places (`~/.config/gtk-3.0/settings.ini`,
`gsettings set org.gnome.desktop.interface color-scheme default`, `about:config`).

### The pointer

`mindos-cursors` draws the MindOS cursor theme into
`/usr/share/icons/MindOS`. Its shapes follow the same rule as everything else
on the desktop, from the outside in: a cyan halo that breathes, an off-white
rim, a near-black glass body and a cyan sheen along the inner edge — dark
enough to read on a white document, bright enough to read on the void. The
pointer's glow pulses over a 1.4 s loop and `wait` / `progress` spin a
twelve-tick comet ring, so the cursor is alive without being loud. Nineteen
shapes are drawn at 24, 32, 48 and 64 px, with the usual pile of X11 aliases
(`left_ptr`, `sb_h_double_arrow`, the hashed drag-and-drop names …) symlinked
onto them; `index.theme` inherits Adwaita for anything not drawn.

Everything is generated by `packages/mindos-cursors/gen-cursors.py`, which
draws each frame with Pillow and writes the XCursor binaries itself (no
`xcursorgen`), from the same tokens as the rest of the theme. Run it with an
output directory to redraw the set, and `--preview FILE` for a contact sheet
of every shape at every size, half on the void and half on paper white:

```
python3 packages/mindos-cursors/gen-cursors.py ~/.icons/MindOS --preview /tmp/cursors.png
```

The theme and its size are one setting, in GSettings
(`org.gnome.desktop.interface cursor-theme` / `cursor-size`, defaulted to
`MindOS` and 24 by the gschema override). Settings › Desktop › Pointer writes
it; GTK applications follow through the settings portal at once, `mindwm`
is told over IPC and reloads its own cursor without a restart, and
`mindos-session` reads it at login into `XCURSOR_THEME` / `XCURSOR_SIZE` for
everything that only looks at the environment (SDL, Qt, XWayland). Programs
that read the size once at start pick a new one up the next time they run.

### Fonts

| Font | Role | Comes from |
| --- | --- | --- |
| Inter (Regular, Medium, SemiBold, Bold) | the system face: body text, UI labels, everything you read | `inter-font` |
| JetBrains Mono (Regular, Bold) | commands, key names, clocks, the terminal | `ttf-jetbrains-mono` |
| Orbitron (Regular, Medium, Bold, Black) | the wordmark and display text only | `mindos-theme`, in `/usr/share/fonts/mindos/` |
| Share Tech Mono | the Plymouth caption | `mindos-theme`, in `/usr/share/fonts/mindos/` |
| Noto Sans / Noto Color Emoji | fallback for scripts and emoji Inter does not cover | `noto-fonts`, `noto-fonts-emoji` |

Inter replaced Rajdhani as the interface face: Rajdhani is a condensed display
typeface, and at the 12–14 px the panel and the settings pages actually use,
its narrow counters and short x-height cost real legibility. Inter was drawn
for exactly that size range. Orbitron stays, but only as a wordmark.

The compositor embeds Latin/Greek/Cyrillic subsets of Inter, JetBrains Mono and
Orbitron Bold, plus the full DejaVu Sans as its glyph fallback
(`mindwm/resources/`, licences in `/usr/share/licenses/mindwm/`), so it never
depends on fontconfig; the shell ships the same subsets in its UI bundle
(`mindshell/ui/fonts/`).

#### Type scale

The shell uses five sizes, all Inter, defined once as tokens in
`mindshell/ui/src/app.css` and reused everywhere:

| Token | Size | Used for |
| --- | --- | --- |
| `--t-title` | 600 16 px | page and dialog titles |
| `--t-body` | 400 14 px | body text, help lines, the title bar |
| `--t-label` | 500 13 px | widget labels, buttons, navigation |
| `--t-caption` | 400 12 px | dates, secondary lines |
| `--t-eyebrow` | 600 11 px, `.08em` tracking | section labels, the one uppercase style |

Only section labels (the eyebrow) are uppercase. Widget names, menu items,
dates and status lines are written in sentence case so they read as words,
not badges.

### Text rendering

MindOS renders text the way macOS does rather than the way a stock Linux
desktop does. Three settings, applied consistently everywhere:

| Aspect | Setting | Why |
| --- | --- | --- |
| Antialiasing | grayscale, never subpixel/LCD | Subpixel antialiasing tints glyph edges red and blue. It assumes an RGB-stripe panel at ~96 dpi and fails on rotated displays, on OLED subpixel layouts, and in every screenshot and screen share. macOS dropped it in 10.14. |
| Hinting | slight (vertical only) | Glyphs snap vertically so horizontal stems stay crisp, and are left alone horizontally so letter shapes and spacing keep the proportions they were drawn with. |
| Compositing | gamma-corrected coverage | Blending coverage straight into sRGB makes light text on a dark ground look thin and washed out. The correction is what keeps stems at the weight the face was drawn with — the single biggest reason macOS text looks "fuller". |

Where each of those lives:

| Surface | Configured by |
| --- | --- |
| Everything using fontconfig (GTK, Qt, Electron, Kitty, …) | `/etc/fonts/conf.d/49-mindos-rendering.conf`, from `mindos-theme` |
| Generic family names (`sans-serif`, `system-ui`, `monospace`, `SF Mono`, …) | `/etc/fonts/conf.d/59-mindos-fonts.conf`, from `mindos-theme` |
| GTK, which does not read rendering out of fontconfig | `/etc/mindos/xdg/gtk-{3,4}.0/settings.ini`, from `mindos-session` |
| The Mind bar and window titles, drawn on the CPU by the compositor | `mindwm/src/text.rs` (`TEXT_GAMMA`, plus subpixel glyph placement) |
| The shell UI in WebKitGTK | `mindshell/ui/src/app.css` (`-webkit-font-smoothing: antialiased`) |

A user's own `~/.config/fontconfig/fonts.conf` still overrides all of it: both
MindOS files are numbered below `50-user.conf`.

`mindwm/examples/textproof.rs` renders a sample sheet through the compositor's
own rasteriser, which is the quickest way to see the effect of changing
`TEXT_GAMMA`:

```sh
cd mindwm && cargo run --example textproof -- /tmp/text.ppm
```

### Plymouth

The `mindos` theme (`packages/mindos-theme/mindos.script`) is the first thing
in the dark palette: a void gradient (`#05070a` → `#0a0f16`), the MINDOS
wordmark in Orbitron with a breathing cyan halo, a 3 px progress line with a
bright head, the caption `SYSTEM // BOOT` (`SYSTEM // HALT` on shutdown and
reboot), a hairline near the bottom with a cyan streak sweeping across it, and
faint HUD corner brackets. Status messages appear bottom-left; the disk
password prompt replaces the caption, with cyan bullets. All sprites are
generated by `packages/mindos-theme/gen-assets.py` from the fonts (rendered at
2× and scaled to the screen by the script, so 1440p and 4K stay crisp) —
rerun it after changing colours or fonts:

```sh
python3 packages/mindos-theme/gen-assets.py --preview /tmp/boot.png   # also renders a preview of the boot screen
```

The same script produces `wallpaper.png` (2560×1440, the aurora),
`splash.png` (the boot screen as a still) and `mindos.png` (the OS icon: a
rounded glass tile with a cyan M).

### Compositor and shell

`mindwm` clears to the void, carries the boot splash on as its startup screen
until the shell's desktop is up (the same wordmark, progress line, sweeping
hairline and HUD corners, captioned `STARTING THE DESKTOP`), renders the Mind
bar as a rounded translucent card with the
cyan accent glowing along its top edge and a soft shadow beneath it, and
gives every decorated window the same 36 px graphite title bar with rounded top
corners, a 1 px light ring and a drop shadow (40 px on floating windows,
12 px on tiles, none when maximised)
(`mindwm/src/mindbar.rs`, `mindwm/src/shell/ssd.rs`,
`mindwm/src/shell/frame.rs`, `docs/COMPOSITOR.md`).
`mindshell` draws the bottom bar, the desktop icons, the popups, the
Settings app and the desktop widgets from the same tokens
(`mindshell/ui/src/app.css`, `mindshell/ui/src/glass.ts`, `docs/SHELL.md`). The compositor colours can
be changed in `/etc/mindos/mindwm.toml` (`[theme] background`, `foreground`,
`accent`), but the defaults are the brand.

### Verified

Captured from the dev VM on 2026-09-06 (`scripts/vm/bootshots.sh`): the boot
menu is white on red (`docs/img/limine-red.png`, Limine; the ISO's GRUB in
`docs/img/grub-red.png`), then Plymouth shows the dark splash with
the glowing wordmark, the cyan progress line and the sweeping scan line
(`docs/img/plymouth-dark.png`); on shutdown the caption reads `SYSTEM // HALT`.
