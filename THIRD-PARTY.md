# Third-party software in MindOS

MindOS is a Linux distribution. Most of what is on the ISO was written by other
people and is licensed by them, not by Black Arrow Software, LLC. This file says
what those components are, under what terms they are here, and where to get
their source code.

The `LICENSE` file covers only the parts Black Arrow wrote. It adds no
restriction to anything listed below. Where a component's own licence gives you
rights that `LICENSE` does not — to modify it, to redistribute it, to sell
copies of it — that component's licence wins for that component.

An ISO is an *aggregate*: separate works, each under its own terms, collected
onto one image. Putting them on the same disc does not put them under the same
licence.

---

## The kernel — GPL-2.0-only

`packages/linux-mindos/` builds a modified Linux kernel: kernel.org 7.2.y plus
the BORE scheduler patch (Masahito Suzuki, via Piotr Gorski) and two MindOS
patches (the console theme, and a Clang fix for DKMS). The Linux kernel is
GPL-2.0-only and so is `linux-mindos`.

**Source offer.** The complete corresponding source for `linux-mindos` is the
upstream tarball named in `packages/linux-mindos/PKGBUILD`, plus the three
patch files and the kernel config in that same directory, all of which are in
this repository. Anyone who receives a MindOS binary kernel may take that
source, modify it, rebuild it and redistribute it under the GPL. That right is
not affected by `LICENSE`.

The BORE patch keeps its authorship headers in `0001-bore.patch`.

## Arch Linux packages — mixed licences

The ISO and the installed system pull packages from the Arch Linux repositories:
`base`, systemd, glibc, GTK 4, WebKitGTK, Mesa, PipeWire, NetworkManager,
Firefox, Nautilus, Loupe, File Roller, GNOME Text Editor, foot, Steam,
gamescope, GameMode, Lutris, Wine, llama.cpp, and the rest of the lists in
`iso/packages.x86_64` and the `depends=()` lines of the `packages/*/PKGBUILD`
files.

MindOS does not fork or patch any of them. It depends on them, ships them
unmodified as Arch built them, and configures them from the outside (a
stylesheet, a `mimeapps.list`, a sysctl file). Each carries its own licence and
its own source, both available from Arch Linux at
<https://archlinux.org/packages/> and <https://gitlab.archlinux.org/archlinux/packaging/packages>.

Notable ones to be aware of:

| Component | Licence | Note |
| --- | --- | --- |
| Linux userland (glibc, coreutils, systemd, GTK, WebKitGTK, Mesa, …) | GPL-2.0/3.0, LGPL-2.1/3.0, MIT, and others | Redistributable; source from Arch. `mindshell` links GTK 4 and WebKitGTK **dynamically**, as the LGPL requires. |
| `paru` (`packages/paru/`) | GPL-3.0-or-later | Upstream PKGBUILD, kept as-is with its maintainer line. |
| Firefox | MPL-2.0 + Mozilla trademark policy | Shipped exactly as Arch builds it, unbranded changes not made. Themed only through the desktop's own dark-mode preference, which is a user setting, not a modification of Firefox. |
| Steam | Valve Steam Subscriber Agreement | The Arch `steam` package is a bootstrapper; the client itself is downloaded from Valve by the user on first run. Users accept Valve's terms directly. |
| NVIDIA drivers (`nvidia-open-dkms`, `nvidia-utils`) | NVIDIA Software License Agreement (`nvidia-open-dkms` kernel modules: MIT/GPL-2.0 dual) | Redistributed unmodified under NVIDIA's licence, which permits distribution as part of an operating system. |
| `linux-firmware` | Mixed; many blobs are redistributable-only | The package's own `WHENCE` file carries every blob's terms and ships with it. |

## The language model — Apache-2.0

The default model is **Qwen3.5** (Alibaba Cloud), Apache License 2.0. The
licence text is in `models/LICENSE-Qwen3.5.txt` and ships on the ISO. The
weights are not tracked in this repository; `make model` fetches them.

`mindd` can load any GGUF file. Models from the catalogue in
`packages/mindos-mind/model-catalog.json`, and any file a user supplies, carry
their own licences, which the user is responsible for.

## The compositor — derived from Smithay's `anvil`

`mindwm/` began as **`anvil`**, the reference compositor of the
[Smithay](https://github.com/Smithay/smithay) project, and still contains code
derived from it (`state.rs`, `udev.rs`, `winit.rs`, `drawing.rs`, `cursor.rs`,
`focus.rs`, `input_handler.rs`, `render.rs`, `shell/`). Smithay is MIT licensed:

> Copyright (c) 2017 Victor Berger and Victoria Brekenfeld

The full text is in `mindwm/LICENSE-smithay.txt` and is installed to
`/usr/share/licenses/mindwm/` with the binary. The MIT licence permits this
derivative work to be distributed under other terms so long as that notice
travels with it, which it does.

## Rust crates and Node tooling — MIT / Apache-2.0

`mindwm`, `mindshell` and `mindd` link a number of Rust crates (Smithay, the
gtk-rs and webkit6 bindings, tokio, serde, zbus, clap, reqwest, and their
dependencies). The shell UI is bundled with esbuild and TypeScript at build
time; no Node package ends up in the shipped bundle. Nearly all are MIT or
Apache-2.0, both of which require only that their notices be preserved.

The exact set and versions for any given build are pinned in the `Cargo.lock`
files in this repository. To regenerate the full notice list for a build:

```sh
cargo install cargo-about        # or cargo-deny
cargo about generate about.hbs   # in mindwm/, mindshell/, mindd/
```

## Fonts

| Font | Licence | Where |
| --- | --- | --- |
| Inter | SIL OFL 1.1 | `mindshell/ui/fonts/OFL-Inter.txt`, `mindwm/resources/OFL-Inter.txt` |
| JetBrains Mono | SIL OFL 1.1 | `mindshell/ui/fonts/OFL-JetBrainsMono.txt`, `mindwm/resources/OFL-JetBrainsMono.txt` |
| Orbitron | SIL OFL 1.1 | `packages/mindos-theme/fonts/OFL-Orbitron.txt` and the two above |
| Share Tech Mono | SIL OFL 1.1 | `packages/mindos-theme/fonts/OFL-ShareTechMono.txt` |
| DejaVu Sans | Bitstream Vera / Public domain | `mindwm/resources/LICENSE-DejaVu.txt` |

All are shipped unmodified, under their original names, with their licences
installed to `/usr/share/licenses/`. The OFL forbids selling the fonts on their
own; MindOS does not sell anything.

## Upscaler libraries — not redistributed

`mindos-dlss` (Settings › Games) swaps a game's DLSS, FSR and XeSS DLLs. MindOS
ships **none** of those files. The tool reads the public DLSS Swapper manifest
and downloads a chosen version from the vendor at the user's request, verifying
it against the manifest's MD5s, or imports a copy the user already has. The
vendors' licences apply to those files and the user accepts them by fetching
them.

Anything Wine installs into a prefix (Mono, Gecko, or a `winetricks`
redistributable) is likewise fetched at the user's request and is not on the
ISO.

## Wiki content

The Mind's web tools read the Arch Wiki and Wikipedia over their public APIs at
the user's request and quote from them in an answer. That content is CC BY-SA;
MindOS neither caches nor redistributes it. See `docs/WEB.md`.

---

## Reporting a problem with this file

If you believe something is attributed wrongly, missing, or shipped without the
licence it needs, open an issue:

    https://github.com/morvoso/mindos/issues

Black Arrow Software, LLC will correct it.
