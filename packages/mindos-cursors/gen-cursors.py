#!/usr/bin/env python3
"""Generate the MindOS cursor theme: an animated, dark-glass pointer set.

    python3 packages/mindos-cursors/gen-cursors.py OUTDIR [--preview out.png]

OUTDIR gets `index.theme` and `cursors/`, ready to be installed as
/usr/share/icons/MindOS. Every shape is drawn with Pillow at four nominal
sizes (24, 32, 48, 64) and written straight into the XCursor binary format —
no xcursorgen — so the theme is reproducible from this one file.

The look follows the MindOS tokens: a near-black glass body, an off-white
edge and one electric-cyan accent that breathes (the animation: a slow glow
pulse on the pointer, a spinning ring while something is busy).
"""
import math
import os
import struct
import sys

from PIL import Image, ImageChops, ImageDraw, ImageFilter

# ----- design tokens (shared with mindwm, mindshell and mindos-theme) --------
INK = (0x07, 0x0A, 0x0F)
EDGE = (0xE6, 0xED, 0xF3)
ACCENT = (0x19, 0xE3, 0xFF)
DANGER = (0xFF, 0x5D, 0x8F)

SIZES = (24, 32, 48, 64)
SS = 4               # supersampling factor: everything is drawn 4x and shrunk
FRAMES = 16          # frames in the pointer's glow pulse
DELAY = 90           # ms per frame (a 1.44 s loop)
SPIN_FRAMES = 24
SPIN_DELAY = 60      # a 1.44 s revolution

# ----- the XCursor container -------------------------------------------------
IMAGE_TYPE = 0xFFFD0002


def xcursor(images):
    """Serialise [(nominal size, xhot, yhot, delay, PIL RGBA image)] as XCursor."""
    ntoc = len(images)
    header = struct.pack("<4sIII", b"Xcur", 16, 0x10000, ntoc)
    toc = b""
    chunks = b""
    offset = 16 + 12 * ntoc
    for size, xhot, yhot, delay, img in images:
        w, h = img.size
        raw = img.tobytes()
        px = bytearray(len(raw))
        # XCursor stores premultiplied BGRA, little-endian
        for i in range(0, len(raw), 4):
            r, g, b, a = raw[i], raw[i + 1], raw[i + 2], raw[i + 3]
            px[i] = b * a // 255
            px[i + 1] = g * a // 255
            px[i + 2] = r * a // 255
            px[i + 3] = a
        px = bytes(px)
        chunk = struct.pack("<IIIIIIIII", 36, IMAGE_TYPE, size, 1, w, h, xhot, yhot, delay) + px
        toc += struct.pack("<III", IMAGE_TYPE, size, offset)
        chunks += chunk
        offset += len(chunk)
    return header + toc + chunks


# ----- drawing helpers -------------------------------------------------------
def canvas(px):
    return Image.new("RGBA", (px * SS, px * SS), (0, 0, 0, 0))


def finish(img, px):
    return img.resize((px, px), Image.LANCZOS)


def glow(shape_mask, colour, blur, alpha):
    """A blurred halo of `shape_mask` in `colour`."""
    halo = shape_mask.filter(ImageFilter.GaussianBlur(blur))
    layer = Image.new("RGBA", shape_mask.size, colour + (0,))
    layer.putalpha(halo.point(lambda v: int(v * alpha)))
    return layer


def tint(mask, colour, alpha=1.0):
    layer = Image.new("RGBA", mask.size, colour + (0,))
    layer.putalpha(mask if alpha >= 1.0 else mask.point(lambda v: int(v * alpha)))
    return layer


def grow(mask, r):
    """Dilate `mask` by roughly `r` pixels, keeping the edge soft."""
    return mask.filter(ImageFilter.GaussianBlur(r)).point(lambda v: min(255, v * 5))


def shrink(mask, r):
    return ImageChops.invert(grow(ImageChops.invert(mask), r))


def over(base, layer):
    return Image.alpha_composite(base, layer)


def mask_of(size, draw_fn):
    m = Image.new("L", size, 0)
    draw_fn(ImageDraw.Draw(m))
    return m


def body(px, draw_fn, pulse, rim=1.4):
    """The MindOS treatment, from the outside in: a breathing cyan halo, an
    off-white rim, a near-black body and a cyan sheen along the inner edge.

    The dark body is what makes the cursor readable on a bright window; the
    off-white rim is what makes it readable on the void. `draw_fn(draw, s)`
    fills the shape on a mask, where `s` is the scale of a 32-unit grid.
    """
    s = px * SS / 32.0
    size = (px * SS, px * SS)
    shape = mask_of(size, lambda d: draw_fn(d, s))
    outer = grow(shape, rim * s)
    contour = ImageChops.subtract(outer, shape)
    sheen = ImageChops.subtract(shape, shrink(shape, 1.1 * s))

    img = canvas(px)
    img = over(img, glow(outer, ACCENT, 2.4 * s, 0.26 + 0.40 * pulse))
    img = over(img, tint(contour, EDGE, 0.96))
    img = over(img, tint(shape, INK, 0.97))
    img = over(img, tint(sheen, ACCENT, 0.28 + 0.34 * pulse))
    return finish(img, px)


def pulse_at(i, n):
    """0 → 1 → 0, smoothly, over the frame loop."""
    return (1 - math.cos(2 * math.pi * i / n)) / 2


# ----- the shapes (a 32-unit grid; y grows downwards) ------------------------
ARROW = [(1, 1), (1, 22.5), (6.6, 17.4), (10.1, 25.6), (13.9, 24.0), (10.6, 16.1), (17.6, 15.6)]


def poly(pts, s):
    return [(x * s, y * s) for x, y in pts]


def shape_arrow(d, s, extra=()):
    d.polygon(poly(ARROW, s), fill=255)
    for pts in extra:
        d.polygon(poly(pts, s), fill=255)


def shape_ibeam(d, s):
    d.rectangle(poly([(14.6, 5), (17.4, 27)], s), fill=255)
    for y in (5, 25.4):
        d.rectangle(poly([(11, y), (21, y + 1.6)], s), fill=255)


def shape_hand(d, s):
    # A pointing hand: the index finger, the fist, the thumb.
    d.rounded_rectangle(poly([(11.4, 4), (15.4, 17)], s), radius=2 * s, fill=255)
    d.rounded_rectangle(poly([(9, 13.5), (22.5, 27.5)], s), radius=3.2 * s, fill=255)
    for x in (15.6, 18.2, 20.8):
        d.rounded_rectangle(poly([(x, 11.5), (x + 2.4, 16)], s), radius=1.2 * s, fill=255)
    d.rounded_rectangle(poly([(7.4, 16.5), (11.5, 22)], s), radius=1.8 * s, fill=255)


def shape_cross(d, s, gap=3.0):
    d.rectangle(poly([(14.6, 2), (17.4, 16 - gap)], s), fill=255)
    d.rectangle(poly([(14.6, 16 + gap), (17.4, 30)], s), fill=255)
    d.rectangle(poly([(2, 14.6), (16 - gap, 17.4)], s), fill=255)
    d.rectangle(poly([(16 + gap, 14.6), (30, 17.4)], s), fill=255)


def arrow_head(d, s, cx, cy, angle, length=6.0, width=8.4, depth=5.4):
    """A triangle pointing along `angle` (radians), tip `length` from centre."""
    ax, ay = math.cos(angle), math.sin(angle)
    px_, py_ = -ay, ax
    tip = (cx + ax * length, cy + ay * length)
    base = (cx + ax * (length - depth), cy + ay * (length - depth))
    d.polygon(
        poly([tip, (base[0] + px_ * width / 2, base[1] + py_ * width / 2), (base[0] - px_ * width / 2, base[1] - py_ * width / 2)], s),
        fill=255,
    )


def shape_double_arrow(angle):
    def draw(d, s):
        cx, cy = 16, 16
        ax, ay = math.cos(angle), math.sin(angle)
        d.line(poly([(cx - ax * 9, cy - ay * 9), (cx + ax * 9, cy + ay * 9)], s), fill=255, width=int(2.2 * s))
        arrow_head(d, s, cx, cy, angle, 13.6)
        arrow_head(d, s, cx, cy, angle + math.pi, 13.6)
    return draw


def shape_move(d, s):
    d.line(poly([(16, 7), (16, 25)], s), fill=255, width=int(2.2 * s))
    d.line(poly([(7, 16), (25, 16)], s), fill=255, width=int(2.2 * s))
    for a in (0, math.pi / 2, math.pi, 3 * math.pi / 2):
        arrow_head(d, s, 16, 16, a, 14.0, width=7.4, depth=4.8)


def shape_forbidden(d, s):
    d.ellipse(poly([(4, 4), (28, 28)], s), outline=255, width=int(3.2 * s))
    d.line(poly([(9.5, 22.5), (22.5, 9.5)], s), fill=255, width=int(3.2 * s))


def shape_magnifier(minus=False):
    def draw(d, s):
        d.ellipse(poly([(4, 4), (22, 22)], s), outline=255, width=int(2.6 * s))
        d.line(poly([(20, 20), (28, 28)], s), fill=255, width=int(3.0 * s))
        d.rectangle(poly([(9, 12.2), (17, 13.8)], s), fill=255)
        if not minus:
            d.rectangle(poly([(12.2, 9), (13.8, 17)], s), fill=255)
    return draw


def badge(pts):
    """A small mark drawn beside the arrow (copy, link, help, menu)."""
    def draw(d, s):
        shape_arrow(d, s)
        for p in pts:
            kind, args = p[0], p[1:]
            if kind == "rect":
                d.rectangle(poly([(args[0], args[1]), (args[2], args[3])], s), fill=255)
            elif kind == "ellipse":
                d.ellipse(poly([(args[0], args[1]), (args[2], args[3])], s), outline=255, width=int(2.2 * s))
    return draw


PLUS_BADGE = [("rect", 17, 21, 27, 23.4), ("rect", 20.8, 17.2, 23.2, 27.2)]
MENU_BADGE = [("rect", 17, 18, 28, 20), ("rect", 17, 21.5, 28, 23.5), ("rect", 17, 25, 28, 27)]
LINK_BADGE = [("ellipse", 17, 18, 24, 25), ("ellipse", 21, 22, 28, 29)]


# ----- animated frames -------------------------------------------------------
def spinner_frames(px, i, n):
    """A ring of twelve ticks with a comet head, for wait and progress."""
    s = px * SS / 32.0
    size = (px * SS, px * SS)
    ticks = []
    for k in range(12):
        a = k / 12 * 2 * math.pi - math.pi / 2
        # brightness trails the head around the ring
        d = ((a + math.pi / 2 - i / n * 2 * math.pi) % (2 * math.pi)) / (2 * math.pi)
        ticks.append((mask_of(size, lambda dr, a=a: dr.line(
            poly([(16 + math.cos(a) * 7.5, 16 + math.sin(a) * 7.5),
                  (16 + math.cos(a) * 12.5, 16 + math.sin(a) * 12.5)], s),
            fill=255, width=int(2.4 * s))), 0.16 + 0.84 * (1 - d) ** 2.2))

    whole = ticks[0][0].copy()
    for m, _ in ticks[1:]:
        whole = ImageChops.lighter(whole, m)
    img = canvas(px)
    img = over(img, glow(whole, ACCENT, 2.2 * s, 0.28))
    img = over(img, tint(ImageChops.subtract(grow(whole, 1.0 * s), whole), INK, 0.85))
    for m, strength in ticks:
        img = over(img, glow(m, ACCENT, 1.6 * s, 0.45 * strength))
        img = over(img, tint(m, ACCENT, strength))
    return finish(img, px)


def build(shape, hotspot, frames=FRAMES, delay=DELAY, animate=True, colour=None):
    """One cursor: every size, every frame."""
    out = []
    for px in SIZES:
        for i in range(frames if animate else 1):
            p = pulse_at(i, frames) if animate else 0.5
            img = shape(px, p) if callable(shape) and shape.__name__ == "custom" else body(px, shape, p)
            hx, hy = hotspot
            out.append((px, round(hx * px / 32), round(hy * px / 32), delay if animate else 0, img))
    return out


def wait_cursor():
    out = []
    for px in SIZES:
        for i in range(SPIN_FRAMES):
            out.append((px, px // 2, px // 2, SPIN_DELAY, spinner_frames(px, i, SPIN_FRAMES)))
    return out


def progress_cursor():
    out = []
    for px in SIZES:
        for i in range(SPIN_FRAMES):
            arrow = body(px, shape_arrow, pulse_at(i, SPIN_FRAMES))
            ring = spinner_frames(px, i, SPIN_FRAMES).resize((px * 5 // 8, px * 5 // 8), Image.LANCZOS)
            arrow.alpha_composite(ring, (px - px * 5 // 8, px - px * 5 // 8))
            out.append((px, round(px / 32), round(px / 32), SPIN_DELAY, arrow))
    return out


# ----- the theme -------------------------------------------------------------
# name: (shape, hotspot in the 32-unit grid)
SHAPES = {
    "default": (shape_arrow, (1, 1)),
    "text": (shape_ibeam, (16, 16)),
    "pointer": (shape_hand, (12, 4)),
    "crosshair": (shape_cross, (16, 16)),
    "cell": (lambda d, s: shape_cross(d, s, gap=0), (16, 16)),
    "move": (shape_move, (16, 16)),
    "not-allowed": (shape_forbidden, (16, 16)),
    "zoom-in": (shape_magnifier(), (13, 13)),
    "zoom-out": (shape_magnifier(minus=True), (13, 13)),
    "copy": (badge(PLUS_BADGE), (1, 1)),
    "alias": (badge(LINK_BADGE), (1, 1)),
    "context-menu": (badge(MENU_BADGE), (1, 1)),
    "help": (badge([("ellipse", 18, 18, 27, 27)]), (1, 1)),
    "ns-resize": (shape_double_arrow(math.pi / 2), (16, 16)),
    "ew-resize": (shape_double_arrow(0), (16, 16)),
    "nesw-resize": (shape_double_arrow(-math.pi / 4), (16, 16)),
    "nwse-resize": (shape_double_arrow(math.pi / 4), (16, 16)),
}

# every other name a toolkit may ask for, pointed at one of the above
ALIASES = {
    "default": ["left_ptr", "arrow", "top_left_arrow", "default", "dnd-none", "pointer-move"],
    "text": ["xterm", "ibeam", "vertical-text"],
    "pointer": ["hand", "hand1", "hand2", "pointing_hand", "grab", "grabbing", "openhand",
                "closedhand", "dnd-move", "e29285e634086352946a0e7090d73106",
                "9d800788f1b08800ae810202380a0822", "5aca4d189052212118709018842178c0",
                "fcf21c00b30f7e3f83fe0dfd12e71cff"],
    "crosshair": ["cross", "cross_reverse", "diamond_cross", "tcross"],
    "cell": ["plus"],
    "move": ["fleur", "all-scroll", "size_all", "grabbing2", "4498f0e0c1937ffe01fd06f973665830",
             "9081237383d90e509aa00f00170e968f"],
    "not-allowed": ["no-drop", "forbidden", "crossed_circle", "circle", "dnd-no-drop",
                    "03b6e0fcb3499374a867c041f52298f0"],
    "zoom-in": ["zoom_in", "f41c0e382c94c0958e07017e42b00462"],
    "zoom-out": ["zoom_out"],
    "copy": ["dnd-copy", "1081e37283d90000800003c07f3ef6bf", "6407b0e94181790501fd1e167b474872"],
    "alias": ["dnd-link", "link", "0876e1c15ff2fc01f906f1c363074c0f", "3085a0e285430894940527032f8b26df",
              "640fb0e74195791501fd1ed57b41487f", "a2a266d0498c3104214a47bd64ab0fc8"],
    "context-menu": ["menu"],
    "help": ["question_arrow", "whats_this", "left_ptr_help", "dnd-ask",
             "d9ce0ab605698f320427677b458ad60b", "5c6cd98b3f3ebcb1f9c7f1c204630408"],
    "ns-resize": ["n-resize", "s-resize", "top_side", "bottom_side", "sb_v_double_arrow",
                  "row-resize", "size_ver", "split_v", "double_arrow", "v_double_arrow",
                  "00008160000006810000408080010102"],
    "ew-resize": ["e-resize", "w-resize", "left_side", "right_side", "sb_h_double_arrow",
                  "col-resize", "size_hor", "split_h", "h_double_arrow",
                  "028006030e0e7ebffc7f7070c0600140"],
    "nesw-resize": ["ne-resize", "sw-resize", "top_right_corner", "bottom_left_corner",
                    "size_bdiag", "fd_double_arrow"],
    "nwse-resize": ["nw-resize", "se-resize", "top_left_corner", "bottom_right_corner",
                    "size_fdiag", "bd_double_arrow"],
    "wait": ["watch", "0034003300310030003800340032003800", "clock",
             "08e8e1c95fe2fc01f976f1e063a24ccd"],
    "progress": ["left_ptr_watch", "half-busy", "00000000000000020006000e7e9ffc3f",
                 "3ecb610c1bf2410f44200f48c40d3599"],
}


def write_theme(outdir):
    cursors = os.path.join(outdir, "cursors")
    os.makedirs(cursors, exist_ok=True)
    with open(os.path.join(outdir, "index.theme"), "w") as f:
        f.write("[Icon Theme]\nName=MindOS\nComment=MindOS: dark glass, one electric-cyan accent, alive\nInherits=Adwaita\n")
    made = {}
    for name, (shape, hotspot) in SHAPES.items():
        made[name] = build(shape, hotspot)
    made["wait"] = wait_cursor()
    made["progress"] = progress_cursor()
    for name, images in made.items():
        with open(os.path.join(cursors, name), "wb") as f:
            f.write(xcursor(images))
        for alias in ALIASES.get(name, []):
            if alias == name:
                continue
            link = os.path.join(cursors, alias)
            if os.path.lexists(link):
                os.remove(link)
            os.symlink(name, link)
    return made


def preview(made, path, sizes=SIZES):
    """A contact sheet: one row per nominal size, on the MindOS void."""
    names = sorted(made)
    cell = max(sizes) + 16
    sheet = Image.new("RGBA", (len(names) * cell, len(sizes) * cell), (0x05, 0x07, 0x0A, 255))
    band = ImageDraw.Draw(sheet)
    # half the sheet on paper-white, to check the edge on a bright background
    band.rectangle([len(names) * cell // 2, 0, len(names) * cell, len(sizes) * cell], fill=(0xF4, 0xF6, 0xF8, 255))
    for row, size in enumerate(sizes):
        for col, name in enumerate(names):
            img = next(im for s_, _, _, _, im in made[name] if s_ == size)
            sheet.alpha_composite(img, (col * cell + 8, row * cell + 8))
    sheet.save(path)


if __name__ == "__main__":
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    outdir = args[0] if args else os.path.join(os.path.dirname(os.path.abspath(__file__)), "MindOS")
    made = write_theme(outdir)
    if "--preview" in sys.argv:
        preview(made, sys.argv[sys.argv.index("--preview") + 1])
    total = sum(len(v) for v in made.values())
    print(f"MindOS cursors: {len(made)} shapes, {total} images → {outdir}")
