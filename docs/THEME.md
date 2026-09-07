# The MindOS look

MindOS has two visual stages with a hard line between them:

* **Boot stage: white on MindOS red.** The boot loader and the kernel console
  paint white text (`#ffffff`) on MindOS red (`#8c1010`). Red means "the
  machine is still booting"; nothing after the kernel hands over to the splash
  uses it.
* **System stage: dark glass.** Plymouth, the compositor and the desktop
  shell share one dark palette with a single electric-cyan accent: a navy
  void with a cyan and violet aurora behind everything, and translucent,
  frosted surfaces with soft corners in front of it. No red anywhere.

## Boot stage (red)

| Stage | How | Where |
| --- | --- | --- |
| Limine (installed system) | `term_background: 008c1010`, white foreground and palette, the selected entry inverted to red on white; `mindos-boot config` folds the file into `/boot/limine.conf` | `packages/mindos-theme/limine-theme.conf` |
| GRUB (UEFI ISO) | `set color_normal=white/black` on a `background_color 140,16,16` menu (black is transparent in gfxterm), highlight `red/white` | `iso/grub/grub.cfg` |
| syslinux (BIOS ISO) | red menu with white text | `iso/syslinux/` |
| Kernel console | `linux-mindos` carries a patch that makes the VT default attribute white on red and sets the palette's red to `#8c1010`, so every message from the first kernel line onwards is white on red. On a stock kernel the same look comes from `vt.color=0x4f vt.default_red=... vt.default_grn=... vt.default_blu=...` | `packages/linux-mindos/` |
| Virtual consoles after boot | `mindos-console-theme.service` re-applies the colours to tty1–6, so the tty2 recovery shell stays red | `packages/mindos-theme/console-theme` |
| Login screen | dark glass like the desktop: the aurora, a frosted card, the cyan accent; rendered by the shell's own UI stack (`mindshell --app greeter`) under mindwm in kiosk mode | `mindshell/ui/src/greeter.ts`, `docs/img/greeter.png` |

## System stage (dark glass)

### Tokens

| Token | Value | Used for |
| --- | --- | --- |
| void | `#05070a` | screen clear colour, deepest background |
| bg-0 | `#0a0d12` | panels, the Mind bar |
| bg-1 | `#10151c` | raised surfaces, progress track |
| hairline | `#223041` | 1 px borders and separators |
| line-strong | `#2f4257` | emphasised borders |
| fg | `#e6edf3` | text |
| fg-dim | `#8b9bb0` | secondary text |
| fg-faint | `#55657a` | captions, hints |
| accent | `#19e3ff` | the one accent: progress, selection, focus, glow |
| accent-dim | `#0aa7c2` | accent on dark surfaces |
| mind | `#a78bfa` | anything the Mind (LLM) says or does |
| warn | `#ffb454` | tool calls, confirmations |
| danger | `#ff5d8f` | errors (deliberately not red) |
| ok | `#3ddc97` | success |

### Glass

Everything in front of the wallpaper is glass: a dark tint (`rgb(12 17 25)`
at 45–80 % alpha) over a blurred copy of what is behind it, a 1 px light
border (`white / 9 %`, `16 %` when raised), a lighter line catching the top
edge, a soft drop shadow, and rounded corners — 9 px on buttons, 12–16 px on
cards and popups, 22 px on the dock pill. Section labels stay uppercase with
wide tracking; body text is Inter at normal tracking.

| Where | How the glass is made |
| --- | --- |
| Desktop widgets | real `backdrop-filter: blur(28px) saturate(1.5)` — they live in the wallpaper's own window |
| Panels, the dock, popups, app sidebars | separate WebKit windows cannot see the wallpaper, so `mindshell/ui/src/glass.ts` puts a `.glass-bd` layer under the surface: the wallpaper blurred once on a small canvas (or the aurora gradient), sized to the output and shifted by the surface's position on it, so the crop under the window shows through. Updated when the wallpaper, the layout or the window moves. |
| Title bars and the Mind bar | drawn by the compositor as translucent rounded cards (`Canvas::fill_rounded_rect`); mindwm does not blur, the alpha alone reads as glass over the desktop |
| App windows (Settings) | opaque, over the same aurora; the sidebar is frosted with the wallpaper |
| Files, Image Viewer, Archive Manager, Text Editor (libadwaita) | opaque in the MindOS colours (`mindos-apps`, below); they draw their own header bars, which the compositor leaves alone |
| Terminals | `foot` runs at 92 % alpha with the MindOS palette (`packages/mindos-session/foot.ini`) |

The aurora is `--aurora` in `mindshell/ui/src/app.css` (radial cyan, violet,
blue and teal light over a navy-to-void diagonal); `wallpaper.png` from
`mindos-theme` is the same composition rendered by `gen-assets.py`, so the
built-in wallpaper and the file look alike.

### Dark mode for applications

Every toolkit is told the desktop is dark, from `mindos-session`:

| Toolkit | Mechanism |
| --- | --- |
| GTK 3 / GTK 4 | `/etc/mindos/xdg/gtk-{3,4}.0/settings.ini` (via `XDG_CONFIG_DIRS`): `gtk-theme-name=Adwaita` with `gtk-application-prefer-dark-theme=1` (the built-in dark variant; GTK 3 has no theme *named* Adwaita-dark and would fall back to light), the same name in the gschema override for GTK 3 on Wayland, which reads it from GSettings, `breeze-dark` icons |
| libadwaita, GTK 4, Firefox, Electron | the Settings portal: `/usr/share/xdg-desktop-portal/mindos-portals.conf` picks `xdg-desktop-portal-gtk`, which reports `org.gnome.desktop.interface color-scheme` — defaulted to `prefer-dark` by `/usr/share/glib-2.0/schemas/90_mindos.gschema.override` (also the Inter / JetBrains Mono font names) |
| libadwaita, GTK 4 | the MindOS colours: `/usr/share/mindos/gtk/gtk-4.0.css` (`mindos-apps`) sets libadwaita's named colours (`--accent-bg-color`, `--window-bg-color`, `--headerbar-bg-color`, … and the `@define-color` names for older apps) to the theme tokens: teal accent with dark text, bg-1 windows, bg-0 views, bg-2 header bars and popovers, hot pink destructive. GTK reads `gtk.css` only from `~/.config/gtk-4.0/`, so `/etc/xdg/mindos/autostart/10-mindos-gtk-css` writes a one-line `@import` there on first login (and a GTK 3 one importing `gtk-3.0.css`, which overrides Adwaita-dark's `theme_*` colours). Delete the import to opt out. The accent is also announced through the portal: `accent-color='teal'` in the gschema override |
| Firefox | `/usr/lib/firefox/defaults/pref/mindos.js`: `ui.systemUsesDarkTheme=1`, dark toolbar and content themes, `prefers-color-scheme: dark` for pages, the compositor's title bar instead of Firefox's own |
| foot | the palette in `foot.ini` |

Users override any of these in the usual places (`~/.config/gtk-3.0/settings.ini`,
`gsettings set org.gnome.desktop.interface color-scheme default`, `about:config`).

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
| Everything using fontconfig (GTK, Qt, Electron, foot, …) | `/etc/fonts/conf.d/49-mindos-rendering.conf`, from `mindos-theme` |
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

`mindwm` clears to the void, draws the MINDOS wordmark and key hints on an
empty desktop, renders the Mind bar as a rounded translucent card with the
cyan accent glowing along its top edge, and gives every decorated window the
same 30 px glass title bar with rounded top corners
(`mindwm/src/mindbar.rs`, `mindwm/src/shell/ssd.rs`, `docs/COMPOSITOR.md`).
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
