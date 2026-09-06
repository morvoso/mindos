#!/usr/bin/env python3
"""Generate the MindOS theme assets (Plymouth sprites, wallpaper, splash, icon).

Run from anywhere: python3 packages/mindos-theme/gen-assets.py [--preview out.png]
Everything is rendered from the fonts in ./fonts with Pillow, so the PNGs in
this directory are reproducible. Plymouth sprites are drawn at 2x and scaled
down by mindos.script (crisp on 1440p/4K); "--preview" composes the boot
screen exactly like the script does, for checking the look without booting.
"""
import os
import sys

from PIL import Image, ImageDraw, ImageFilter, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
FONTS = os.path.join(HERE, "fonts")

# MindOS design tokens (shared with mindwm and mindshell)
VOID = (0x05, 0x07, 0x0A)
BG0 = (0x0A, 0x0D, 0x12)
BG1 = (0x10, 0x15, 0x1C)
LINE = (0x22, 0x30, 0x41)
LINE_STRONG = (0x2F, 0x42, 0x57)
FG = (0xE6, 0xED, 0xF3)
FG_DIM = (0x8B, 0x9B, 0xB0)
FG_FAINT = (0x55, 0x65, 0x7A)
ACCENT = (0x19, 0xE3, 0xFF)
ACCENT_DIM = (0x0A, 0xA7, 0xC2)


def font(name, px):
    return ImageFont.truetype(os.path.join(FONTS, name), int(px))


def text_size(fnt, text, spacing):
    w = 0
    for i, ch in enumerate(text):
        w += fnt.getlength(ch)
        if i < len(text) - 1:
            w += spacing
    asc, desc = fnt.getmetrics()
    return int(round(w)), asc + desc


def draw_spaced(draw, xy, text, fnt, spacing, fill):
    x, y = xy
    for ch in text:
        draw.text((x, y), ch, font=fnt, fill=fill)
        x += fnt.getlength(ch) + spacing


def text_mask(text, fnt, spacing, pad):
    w, h = text_size(fnt, text, spacing)
    mask = Image.new("L", (w + 2 * pad, h + 2 * pad), 0)
    draw_spaced(ImageDraw.Draw(mask), (pad, pad), text, fnt, spacing, 255)
    return mask


def tint(mask, color, alpha=1.0):
    """An RGBA image of `color` whose alpha is `mask` scaled by `alpha`."""
    img = Image.new("RGBA", mask.size, color + (0,))
    a = mask.point(lambda v: int(v * alpha)) if alpha != 1.0 else mask
    img.putalpha(a)
    return img


def glow_text(text, fnt, spacing, glow_radius, glow_alpha, text_color, glow_color, pad=None):
    """Crisp text over a soft coloured glow, on transparency."""
    pad = pad or glow_radius * 3
    mask = text_mask(text, fnt, spacing, pad)
    glow = tint(mask.filter(ImageFilter.GaussianBlur(glow_radius)), glow_color, glow_alpha)
    crisp = tint(mask, text_color)
    return Image.alpha_composite(glow, crisp)


def save(img, name, **kw):
    path = os.path.join(HERE, name)
    img.save(path, optimize=True, **kw)
    print(f"{name:20s} {img.size[0]}x{img.size[1]}  {os.path.getsize(path) // 1024} KB")


# ----------------------------------------------------------------------------- Plymouth sprites
def plymouth_assets():
    # wordmark at 2x: Orbitron Bold, wide tracking, cyan glow
    fnt = font("Orbitron-Bold.ttf", 2 * 112)
    spacing = 2 * 20
    logo = glow_text("MINDOS", fnt, spacing, glow_radius=2 * 10, glow_alpha=0.75,
                     text_color=FG, glow_color=ACCENT, pad=2 * 32)
    save(logo, "logo.png")

    # the pulsing halo behind it, at 1x (it is blurry anyway; the script scales it)
    fnt1 = font("Orbitron-Bold.ttf", 112)
    mask = text_mask("MINDOS", fnt1, 20, 96)
    halo = tint(mask.filter(ImageFilter.GaussianBlur(34)), ACCENT, 0.9)
    save(halo, "logo-glow.png")

    # caption at 2x: Share Tech Mono, tracked, faint
    cf = font("ShareTechMono-Regular.ttf", 2 * 15)
    for name, text in (("caption-boot.png", "SYSTEM // BOOT"), ("caption-halt.png", "SYSTEM // HALT")):
        m = text_mask(text, cf, 2 * 4, 2 * 2)
        save(tint(m, FG_DIM), name)

    # 1x1 pixels the script scales into bars and lines
    save(Image.new("RGBA", (1, 1), (255, 255, 255, 255)), "bar.png")
    save(Image.new("RGBA", (1, 1), (0x1A, 0x23, 0x31, 255)), "px-track.png")
    save(Image.new("RGBA", (1, 1), ACCENT + (255,)), "px-fill.png")
    save(Image.new("RGBA", (1, 1), LINE + (255,)), "px-line.png")

    # bright head at the end of the progress fill (2x: 96x24)
    w, h = 96, 24
    head = Image.new("RGBA", (w, h), ACCENT + (0,))
    core = Image.new("L", (w, h), 0)
    ImageDraw.Draw(core).ellipse((w / 2 - 14, h / 2 - 3, w / 2 + 14, h / 2 + 3), fill=255)
    soft = core.filter(ImageFilter.GaussianBlur(7))
    head = Image.alpha_composite(tint(soft, ACCENT, 0.9), tint(core.filter(ImageFilter.GaussianBlur(1)), (0xF2, 0xFD, 0xFF)))
    save(head, "bar-head.png")

    # the streak that scans along the hairline (2x: 520x4)
    w, h = 520, 4
    streak = Image.new("L", (w, h), 0)
    px = streak.load()
    for x in range(w):
        t = abs(x - w / 2) / (w / 2)
        v = int(255 * max(0.0, 1 - t) ** 1.3)
        for y in range(h):
            px[x, y] = v if y in (1, 2) else v // 2
    save(tint(streak, ACCENT), "scan.png")

    # password bullet (2x: 20x20 accent dot)
    b = Image.new("L", (20, 20), 0)
    ImageDraw.Draw(b).ellipse((4, 4, 15, 15), fill=255)
    save(tint(b.filter(ImageFilter.GaussianBlur(0.6)), ACCENT), "bullet.png")

    # HUD corner brackets (2x: 64x64, 3px stroke) in the four orientations
    w = 64
    base = Image.new("RGBA", (w, w), (0, 0, 0, 0))
    d = ImageDraw.Draw(base)
    d.rectangle((0, 0, w - 1, 3), fill=FG_FAINT + (200,))
    d.rectangle((0, 0, 3, w - 1), fill=FG_FAINT + (200,))
    d.rectangle((0, 0, 9, 9), fill=ACCENT + (230,))
    save(base, "corner-tl.png")
    save(base.transpose(Image.Transpose.FLIP_LEFT_RIGHT), "corner-tr.png")
    save(base.transpose(Image.Transpose.FLIP_TOP_BOTTOM), "corner-bl.png")
    save(base.transpose(Image.Transpose.ROTATE_180), "corner-br.png")


# ----------------------------------------------------------------------------- composition
def gradient(w, h, top, bottom):
    """Vertical gradient, ordered-dithered (8x8 Bayer) so dark tones do not band
    while the PNG still compresses (the pattern repeats along x)."""
    import numpy as np
    bayer = np.array([[0, 32, 8, 40, 2, 34, 10, 42], [48, 16, 56, 24, 50, 18, 58, 26],
                      [12, 44, 4, 36, 14, 46, 6, 38], [60, 28, 52, 20, 62, 30, 54, 22],
                      [3, 35, 11, 43, 1, 33, 9, 41], [51, 19, 59, 27, 49, 17, 57, 25],
                      [15, 47, 7, 39, 13, 45, 5, 37], [63, 31, 55, 23, 61, 29, 53, 21]])
    thresh = (np.tile(bayer, (h // 8 + 1, w // 8 + 1))[:h, :w] + 0.5) / 64.0
    t = (np.arange(h) / max(1, h - 1))[:, None]
    out = np.empty((h, w, 3), dtype="uint8")
    for i in range(3):
        channel = top[i] + (bottom[i] - top[i]) * t          # (h, 1) float
        out[:, :, i] = np.clip(np.floor(channel + thresh), 0, 255)
    return Image.fromarray(out, "RGB")


def compose_boot(w, h, progress=0.6, frame=40):
    """The boot screen as mindos.script lays it out (same maths, same assets)."""
    s = max(0.5, min(2.5, h / 1080))
    img = gradient(w, h, VOID, (0x0A, 0x0F, 0x16)).convert("RGBA")

    def asset(name):
        return Image.open(os.path.join(HERE, name)).convert("RGBA")

    def scaled(im, k):
        return im.resize((max(1, int(im.width * k)), max(1, int(im.height * k))), Image.Resampling.LANCZOS)

    def paste(im, x, y, opacity=1.0):
        if opacity < 1.0:
            im = im.copy()
            im.putalpha(im.getchannel("A").point(lambda v: int(v * opacity)))
        img.alpha_composite(im, (int(x), int(y)))

    cx, cy = w / 2, h / 2
    logo = scaled(asset("logo.png"), s / 2)
    lx, ly = cx - logo.width / 2, cy - 30 * s - logo.height / 2
    halo = scaled(asset("logo-glow.png"), s)
    pulse = 0.25 + 0.5 * (0.5 + 0.5 * __import__("math").sin(frame / 150 * 6.2832))
    paste(halo, cx - halo.width / 2, ly + logo.height / 2 - halo.height / 2, pulse)
    paste(logo, lx, ly)

    bar_w, bar_h = int(360 * s), max(2, int(3 * s))
    bx, by = int(cx - bar_w / 2), int(ly + logo.height + 34 * s)
    paste(asset("px-track.png").resize((bar_w, bar_h)), bx, by)
    fill_w = int(bar_w * progress)
    if fill_w > 0:
        paste(asset("px-fill.png").resize((fill_w, bar_h)), bx, by)
        head = scaled(asset("bar-head.png"), s / 2)
        paste(head, bx + fill_w - head.width / 2, by + bar_h / 2 - head.height / 2)

    cap = scaled(asset("caption-boot.png"), s / 2)
    paste(cap, cx - cap.width / 2, by + bar_h + 24 * s)

    line_w = int(w * 0.56)
    lx0, ly0 = int(cx - line_w / 2), int(h * 0.84)
    paste(asset("px-line.png").resize((line_w, max(1, int(s)))), lx0, ly0, 0.9)
    streak = scaled(asset("scan.png"), s / 2)
    phase = (frame % 130) / 130
    paste(streak, lx0 - streak.width + (line_w + streak.width) * phase, ly0 - streak.height / 2 + s / 2, 0.85)

    inset = 32 * s
    for name, x, y in (("corner-tl.png", inset, inset), ("corner-tr.png", w - inset, inset),
                       ("corner-bl.png", inset, h - inset), ("corner-br.png", w - inset, h - inset)):
        c = scaled(asset(name), s / 2)
        paste(c, x - (c.width if "r" in name[-6:-4] else 0), y - (c.height if "b" in name[-6:-4] else 0), 0.7)
    return img.convert("RGB")


def wallpaper(w=2560, h=1440):
    img = gradient(w, h, VOID, (0x0B, 0x11, 0x19)).convert("RGBA")
    # faint 48 px grid, strongest at the bottom, gone at the top
    grid = Image.new("L", (w, h), 0)
    d = ImageDraw.Draw(grid)
    for x in range(0, w, 48):
        d.line((x, 0, x, h), fill=255)
    for y in range(h % 48, h, 48):
        d.line((0, y, w, y), fill=255)
    fade = Image.linear_gradient("L").resize((w, h))            # black top → white bottom
    fade = fade.point(lambda v: int(v * 0.30))
    from PIL import ImageChops
    grid = ImageChops.multiply(grid, fade)
    img.alpha_composite(tint(grid, LINE))
    # soft cyan glow rising from the bottom centre (exact elliptical falloff)
    import numpy as np
    gw, gh = int(w * 0.9), int(h * 0.55)
    ys, xs = np.mgrid[0:gh, 0:gw]
    d2 = ((xs - gw / 2) / (gw / 2)) ** 2 + ((ys - gh / 2) / (gh / 2)) ** 2
    fall = np.clip(1.0 - d2, 0.0, 1.0) ** 2
    glow = Image.fromarray((fall * 255 * 0.16).astype("uint8"), "L")
    img.alpha_composite(tint(glow, ACCENT), (int(w / 2 - gw / 2), int(h - gh * 0.45)))
    # wordmark, bottom right, quiet
    fnt = font("Orbitron-Bold.ttf", 26)
    m = text_mask("MINDOS", fnt, 6, 4)
    img.alpha_composite(tint(m, FG, 0.22), (w - 72 - m.width, h - 72 - m.height))
    return img.convert("RGB")


def icon(size=256):
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    c = size * 0.14  # chamfer
    poly = [(c, 0), (size - 1, 0), (size - 1, size - 1 - c), (size - 1 - c, size - 1), (0, size - 1), (0, c)]
    d.polygon(poly, fill=BG1 + (255,), outline=LINE_STRONG + (255,), width=4)
    # inner cyan glow at the bottom edge
    glow = Image.new("L", (size, size), 0)
    ImageDraw.Draw(glow).rectangle((0, size * 0.78, size, size), fill=255)
    glow = glow.filter(ImageFilter.GaussianBlur(size * 0.12)).point(lambda v: int(v * 0.35))
    shape = Image.new("L", (size, size), 0)
    ImageDraw.Draw(shape).polygon(poly, fill=255)
    from PIL import ImageChops
    img.alpha_composite(tint(ImageChops.multiply(glow, shape), ACCENT))
    # the M
    fnt = font("Orbitron-Black.ttf", int(size * 0.62))
    m = glow_text("M", fnt, 0, glow_radius=int(size * 0.05), glow_alpha=0.8, text_color=ACCENT, glow_color=ACCENT, pad=int(size * 0.2))
    img.alpha_composite(m, (int(size / 2 - m.width / 2), int(size / 2 - m.height / 2 - size * 0.02)))
    return img


if __name__ == "__main__":
    os.chdir(HERE)
    plymouth_assets()
    save(wallpaper(), "wallpaper.png")
    save(compose_boot(1920, 1080), "splash.png")
    save(icon(), "mindos.png")
    if len(sys.argv) > 2 and sys.argv[1] == "--preview":
        compose_boot(1920, 1080, progress=0.6, frame=40).save(sys.argv[2])
        print("preview", sys.argv[2])
