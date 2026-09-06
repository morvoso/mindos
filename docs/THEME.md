# The MindOS look

MindOS has two visual stages with a hard line between them:

* **Boot stage: white on MindOS red.** The boot loader and the kernel console
  paint white text (`#ffffff`) on MindOS red (`#8c1010`). Red means "the
  machine is still booting"; nothing after the kernel hands over to the splash
  uses it.
* **System stage: the dark HUD.** Plymouth, the compositor and the desktop
  shell share one dark, minimal, gamey palette with a single electric-cyan
  accent. No red anywhere.

## Boot stage (red)

| Stage | How | Where |
| --- | --- | --- |
| GRUB | `set color_normal=white/black` on a `background_color 140,16,16` menu (black is transparent in gfxterm), highlight `red/white` | `packages/mindos-theme/05_mindos`, `grub-default`, `iso/grub/grub.cfg` |
| syslinux (BIOS ISO) | red menu with white text | `iso/syslinux/` |
| Kernel console | `linux-mindos` carries a patch that makes the VT default attribute white on red and sets the palette's red to `#8c1010`, so every message from the first kernel line onwards is white on red. On a stock kernel the same look comes from `vt.color=0x4f vt.default_red=... vt.default_grn=... vt.default_blu=...` | `packages/linux-mindos/` |
| Virtual consoles after boot | `mindos-console-theme.service` re-applies the colours to tty1–6, so the tty2 recovery shell stays red | `packages/mindos-theme/console-theme` |

## System stage (dark HUD)

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

Shapes are chamfered (cut corners) rather than rounded; borders are hairlines;
glow is used sparingly on the accent. Labels are uppercase with wide tracking.

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

The same script produces `wallpaper.png` (2560×1440 void gradient, faint
grid, cyan glow), `splash.png` (the boot screen as a still) and `mindos.png`
(the OS icon: a chamfered tile with a cyan M).

### Compositor and shell

`mindwm` clears to the void, draws the MINDOS wordmark and key hints on an
empty desktop, and renders the Mind bar as a chamfered bg-0 panel with the
cyan accent, and gives every decorated window the same 30 px title bar
(`mindwm/src/mindbar.rs`, `mindwm/src/shell/ssd.rs`, `docs/COMPOSITOR.md`).
`mindshell` draws the top bar, the dock, the Settings and Files apps and the
desktop widgets from the same tokens (`mindshell/ui/src/theme.css`,
`docs/SHELL.md`). The compositor colours can
be changed in `/etc/mindos/mindwm.toml` (`[theme] background`, `foreground`,
`accent`), but the defaults are the brand.

### Verified

Captured from the dev VM on 2026-09-06 (`scripts/vm/bootshots.sh`): GRUB stays
white on red (`docs/img/grub-red.png`), then Plymouth shows the dark splash with
the glowing wordmark, the cyan progress line and the sweeping scan line
(`docs/img/plymouth-dark.png`); on shutdown the caption reads `SYSTEM // HALT`.
