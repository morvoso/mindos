# Boot theme: white on MindOS red

MindOS red is `#8c1010`; text is `#ffffff`. Every stage of the boot paints
those two colours.

| Stage | How | Where |
| --- | --- | --- |
| GRUB | `set color_normal=white/red`, `set color_highlight=red/white`, plus a red background for the menu | `packages/mindos-theme/05_mindos`, `iso/grub/grub.cfg` |
| syslinux (BIOS ISO) | red menu with white text | `iso/syslinux/` |
| Kernel console | `linux-mindos` carries a patch that makes the VT default attribute white on red and sets the palette's red to `#8c1010`, so every message from the first kernel line onwards is white on red. On a stock kernel the same look comes from `vt.color=0x4f vt.default_red=... vt.default_grn=... vt.default_blu=...` on the command line | `packages/linux-mindos/` |
| Plymouth | the `mindos` theme: red background, white wordmark and progress bar | `packages/mindos-theme/mindos.plymouth`, `mindos.script` |
| systemd / getty | `mindos-console-theme.service` re-applies the colours to every VT after the splash, so the tty2 recovery shell is red too | `packages/mindos-theme/console-theme` |
| Compositor | mindwm clears the screen to MindOS red, draws the "MindOS" wordmark on the empty desktop and renders the Mind bar in white on translucent red | `mindwm/src/drawing.rs`, `mindwm/src/mindbar.rs` |

The kernel stage has been verified in QEMU with a screenshot of the console
(`build/logs/` keeps the boot log). The compositor colours are configurable in
`/etc/mindos/mindwm.toml` (`[theme] background`, `foreground`) if a user wants
something else, but the defaults are the brand.
