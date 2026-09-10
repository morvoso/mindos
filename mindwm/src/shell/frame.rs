//! The window frame: the soft shadow and the hairline border the compositor
//! draws around every window it decorates.
//!
//! The frame is not one big texture. A window's shadow is the same along the
//! whole of an edge, so it is drawn once, for a reference window just large
//! enough that its corners do not touch, and cut into nine pieces: four
//! corners, which are placed as they are, and four one-pixel-thin edge strips
//! that the GPU stretches to the window's real length. That keeps the frame
//! of a 4K window at a few hundred kilobytes of texture, uploaded once, and
//! makes resizing free (only the stretch changes). The reference is rendered
//! once per look (focused / unfocused, floating / tiled) and shared by every
//! window; each window only holds its own copies of the tiles, because the
//! damage tracker tells elements apart by their buffer's id.
//!
//! The window itself is punched out of the frame, so the tiles can go under
//! the window without darkening a translucent title bar or the client's own
//! transparency.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                Kind,
            },
            ImportMem, Renderer,
        },
    },
    utils::{Logical, Physical, Point, Scale, Size, Transform},
};

use crate::text::{alpha, hex, Canvas, TOP_LEFT, TOP_RIGHT};

/// How far a floating window's shadow reaches, in logical pixels.
pub const SHADOW: i32 = 40;
/// The shadow of a tile (they sit next to each other; less is more).
pub const TILE_SHADOW: i32 = 12;

const BLACK: [f32; 4] = hex(0x000000);
const WHITE: [f32; 4] = hex(0xffffff);

/// Sides of a tile that touch another tile, as bits of `FrameStyle::seams`,
/// in the order the corner and edge arrays use.
pub const SEAM_TOP: u8 = 1;
pub const SEAM_RIGHT: u8 = 2;
pub const SEAM_BOTTOM: u8 = 4;
pub const SEAM_LEFT: u8 = 8;

/// How solid a seam is. The ordinary ring is 0.09-0.17: barely there, which
/// is what a lone window wants and what makes two tiles side by side read as
/// one wide window. A seam is loud on purpose.
const SEAM_ALPHA: f32 = 0.55;

/// What the frame looks like. Every distinct value is one cached reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameStyle {
    pub focused: bool,
    /// Radius of the top corners (the title bar's).
    pub radius: i32,
    /// Reach of the shadow; 0 leaves only the border.
    pub shadow: i32,
    /// The accent a focused window is drawn in, packed 0xRRGGBB. It is part
    /// of the style so that picking a new theme misses the reference cache
    /// and the frames are drawn again in the new colour.
    pub accent: u32,
    /// Which sides touch another tile (`SEAM_*`). Only the tiling modes set
    /// it; a floating window and the outer edges of a tiling are always 0.
    pub seams: u8,
    /// The colour those sides are drawn in, packed 0xRRGGBB.
    pub seam: u32,
}

impl FrameStyle {
    /// The shadow's downward offset: light comes from above.
    fn offset(&self) -> i32 {
        self.shadow / 5
    }

    /// How much of the window's interior a corner tile covers: enough that
    /// the corner tiles, and the strip between them, see a shadow that is
    /// already uniform along the edge.
    fn inner(&self) -> i32 {
        self.shadow + self.offset() + self.radius + 2
    }

    fn shadow_alpha(&self) -> f32 {
        match (self.shadow >= SHADOW, self.focused) {
            (true, true) => 0.62,
            (true, false) => 0.38,
            (false, true) => 0.42,
            (false, false) => 0.26,
        }
    }

    fn ring_alpha(&self) -> f32 {
        if self.focused {
            0.17
        } else {
            0.09
        }
    }
}

/// The nine pieces, as pixels, at one scale.
struct Reference {
    /// Logical margins: left, top, right, bottom (the ring included).
    margins: [i32; 4],
    inner: i32,
    /// Top-left, top-right, bottom-right, bottom-left.
    corners: [Canvas; 4],
    /// Top, right, bottom, left; each a 4-pixel-thin strip.
    edges: [Canvas; 4],
}

impl Reference {
    fn draw(style: FrameStyle, s: i32) -> Reference {
        let s = s.max(1);
        let m = style.shadow;
        let dy = style.offset();
        let inner = style.inner();
        let r = style.radius;
        // margins around the window: the shadow, plus one pixel for the ring
        let margins = [m + 1, (m - dy).max(0) + 1, m + 1, m + dy + 1];
        let [ml, mt, mr, mb] = margins;
        let ref_w = 2 * inner + 4;
        let cw = (ml + ref_w + mr) * s;
        let ch = (mt + ref_w + mb) * s;
        let mut c = Canvas::new(cw, ch);
        let (wx, wy, ww, wh) = (ml * s, mt * s, ref_w * s, ref_w * s);
        let corners = TOP_LEFT | TOP_RIGHT;
        if m > 0 {
            // a wide, soft shadow and a tighter, darker one hugging the window
            let a = style.shadow_alpha();
            c.shadow_rounded_rect(wx, wy + dy * s, ww, wh, r * s, corners, m * s, alpha(BLACK, a));
            let tight = (m / 4).max(2);
            c.shadow_rounded_rect(wx, wy + (dy / 3) * s, ww, wh, r * s, corners, tight * s, alpha(BLACK, a * 0.8));
            if style.focused {
                c.shadow_rounded_rect(wx, wy, ww, wh, r * s, corners, (m / 2).max(4) * s, alpha(hex(style.accent), 0.28));
            }
        }
        // the ring: one pixel just outside the window
        for i in 0..s {
            c.stroke_rounded_rect(wx - s + i, wy - s + i, ww + 2 * s - 2 * i, wh + 2 * s - 2 * i, (r + 1) * s - i, corners, alpha(if style.focused { hex(style.accent) } else { WHITE }, style.ring_alpha()));
        }
        // The seam goes over the ring on the sides that touch another tile,
        // in the same band of `s` pixels just outside the window. Rounded
        // corners are left to the ring underneath: the straight run stops a
        // radius short of each end (with square corners it is the whole side).
        if style.seams != 0 {
            let color = alpha(hex(style.seam), SEAM_ALPHA);
            let k = r * s;
            if style.seams & SEAM_TOP != 0 {
                c.fill_rect(wx - s + k, wy - s, ww + 2 * s - 2 * k, s, color);
            }
            if style.seams & SEAM_BOTTOM != 0 {
                c.fill_rect(wx - s + k, wy + wh, ww + 2 * s - 2 * k, s, color);
            }
            if style.seams & SEAM_LEFT != 0 {
                c.fill_rect(wx - s, wy - s + k, s, wh + 2 * s - 2 * k, color);
            }
            if style.seams & SEAM_RIGHT != 0 {
                c.fill_rect(wx + ww, wy - s + k, s, wh + 2 * s - 2 * k, color);
            }
        }
        c.cut_rounded_rect(wx, wy, ww, wh, r * s, corners);

        let px = |v: i32| v * s;
        let tl = c.crop(0, 0, px(ml + inner), px(mt + inner));
        let tr = c.crop(px(ml + ref_w - inner), 0, px(inner + mr), px(mt + inner));
        let br = c.crop(px(ml + ref_w - inner), px(mt + ref_w - inner), px(inner + mr), px(inner + mb));
        let bl = c.crop(0, px(mt + ref_w - inner), px(ml + inner), px(inner + mb));
        let top = c.crop(px(ml + inner), 0, px(4), px(mt));
        let right = c.crop(px(ml + ref_w), px(mt + inner), px(mr), px(4));
        let bottom = c.crop(px(ml + inner), px(mt + ref_w), px(4), px(mb));
        let left = c.crop(0, px(mt + inner), px(ml), px(4));
        Reference {
            margins,
            inner,
            corners: [tl, tr, br, bl],
            edges: [top, right, bottom, left],
        }
    }
}

thread_local! {
    static REFERENCES: RefCell<HashMap<(FrameStyle, i32), Rc<Reference>>> = RefCell::new(HashMap::new());
}

fn reference(style: FrameStyle, scale: i32) -> Rc<Reference> {
    REFERENCES.with(|cell| {
        cell.borrow_mut()
            .entry((style, scale))
            .or_insert_with(|| Rc::new(Reference::draw(style, scale)))
            .clone()
    })
}

struct Tiles {
    style: FrameStyle,
    scale: i32,
    reference: Rc<Reference>,
    corners: [MemoryRenderBuffer; 4],
    edges: [MemoryRenderBuffer; 4],
}

fn buffer(canvas: &Canvas, scale: i32) -> MemoryRenderBuffer {
    MemoryRenderBuffer::from_slice(
        &canvas.data,
        Fourcc::Argb8888,
        (canvas.width, canvas.height),
        scale,
        Transform::Normal,
        None,
    )
}

/// One window's frame: its tiles, rebuilt when the look changes.
#[derive(Default)]
pub struct Frame {
    tiles: Option<Tiles>,
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Frame")
            .field("style", &self.tiles.as_ref().map(|t| t.style))
            .finish()
    }
}

impl Frame {
    fn tiles(&mut self, style: FrameStyle, scale: i32) -> &Tiles {
        let stale = self
            .tiles
            .as_ref()
            .map(|t| t.style != style || t.scale != scale)
            .unwrap_or(true);
        if stale {
            let reference = reference(style, scale);
            let corners = std::array::from_fn(|i| buffer(&reference.corners[i], scale));
            let edges = std::array::from_fn(|i| buffer(&reference.edges[i], scale));
            self.tiles = Some(Tiles {
                style,
                scale,
                reference,
                corners,
                edges,
            });
        }
        self.tiles.as_ref().unwrap()
    }

    /// The frame's elements around a window of `size` logical pixels whose
    /// top-left corner is at `location`, bottom-most last. Small windows get
    /// a shorter shadow so the corners never overlap.
    pub fn render_elements<R>(
        &mut self,
        renderer: &mut R,
        location: Point<i32, Physical>,
        size: Size<i32, Logical>,
        scale: Scale<f64>,
        mut style: FrameStyle,
        alpha: f32,
    ) -> Vec<MemoryRenderBufferRenderElement<R>>
    where
        R: Renderer + ImportMem,
        R::TextureId: Clone + Send + 'static,
    {
        let (w, h) = (size.w, size.h);
        if w <= 0 || h <= 0 {
            return Vec::new();
        }
        while style.inner() * 2 + 2 > w.min(h) {
            if style.shadow == 0 {
                return Vec::new();
            }
            style.shadow = (style.shadow - 8).max(0);
        }
        let int_scale = scale.x.ceil().max(1.0) as i32;
        let tiles = self.tiles(style, int_scale);
        let [ml, mt, mr, mb] = tiles.reference.margins;
        let inner = tiles.reference.inner;
        let at = |dx: i32, dy: i32| -> Point<f64, Physical> {
            let p: Point<f64, Logical> = (dx as f64, dy as f64).into();
            location.to_f64() + p.to_physical(scale)
        };
        let mut out = Vec::with_capacity(8);
        let mut push = |buf: &MemoryRenderBuffer, loc: Point<f64, Physical>, size: Option<Size<i32, Logical>>| {
            if let Ok(e) = MemoryRenderBufferRenderElement::from_buffer(renderer, loc, buf, Some(alpha), None, size, Kind::Unspecified) {
                out.push(e);
            }
        };
        push(&tiles.corners[0], at(-ml, -mt), None);
        push(&tiles.corners[1], at(w - inner, -mt), None);
        push(&tiles.corners[2], at(w - inner, h - inner), None);
        push(&tiles.corners[3], at(-ml, h - inner), None);
        let run_w = w - 2 * inner;
        let run_h = h - 2 * inner;
        if run_w > 0 {
            push(&tiles.edges[0], at(inner, -mt), Some((run_w, mt).into()));
            push(&tiles.edges[2], at(inner, h), Some((run_w, mb).into()));
        }
        if run_h > 0 {
            push(&tiles.edges[1], at(w, inner), Some((mr, run_h).into()));
            push(&tiles.edges[3], at(-ml, inner), Some((ml, run_h).into()));
        }
        out
    }
}

/// The whole frame of a w×h window assembled on the CPU, the window's
/// top-left corner at (`margin`, `margin`): what the GPU composites, for
/// tests and the `frameproof` example.
pub fn preview(style: FrameStyle, scale: i32, w: i32, h: i32) -> (Canvas, i32) {
    let r = Reference::draw(style, scale);
    let [ml, mt, mr, mb] = r.margins;
    let inner = r.inner;
    let s = scale.max(1);
    let m = ml.max(mt).max(mr).max(mb);
    let mut c = Canvas::new((w + 2 * m) * s, (h + 2 * m) * s);
    let (x0, y0) = (m * s, m * s);
    let px = |v: i32| v * s;
    c.draw_canvas(x0 - px(ml), y0 - px(mt), &r.corners[0]);
    c.draw_canvas(x0 + px(w - inner), y0 - px(mt), &r.corners[1]);
    c.draw_canvas(x0 + px(w - inner), y0 + px(h - inner), &r.corners[2]);
    c.draw_canvas(x0 - px(ml), y0 + px(h - inner), &r.corners[3]);
    // stretch the strips the way the GPU does (they are uniform lengthwise)
    let stretch = |strip: &Canvas, out_w: i32, out_h: i32| {
        let mut o = Canvas::new(out_w, out_h);
        for y in 0..out_h {
            for x in 0..out_w {
                let sx = (x * strip.width / out_w).min(strip.width - 1);
                let sy = (y * strip.height / out_h).min(strip.height - 1);
                let si = ((sy * strip.width + sx) * 4) as usize;
                let di = ((y * out_w + x) * 4) as usize;
                o.data[di..di + 4].copy_from_slice(&strip.data[si..si + 4]);
            }
        }
        o
    };
    let run_w = px(w - 2 * inner);
    let run_h = px(h - 2 * inner);
    if run_w > 0 {
        c.draw_canvas(x0 + px(inner), y0 - px(mt), &stretch(&r.edges[0], run_w, px(mt)));
        c.draw_canvas(x0 + px(inner), y0 + px(h), &stretch(&r.edges[2], run_w, px(mb)));
    }
    if run_h > 0 {
        c.draw_canvas(x0 + px(w), y0 + px(inner), &stretch(&r.edges[1], px(mr), run_h));
        c.draw_canvas(x0 - px(ml), y0 + px(inner), &stretch(&r.edges[3], px(ml), run_h));
    }
    (c, m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(c: &Canvas, x: i32, y: i32) -> [u8; 4] {
        let i = ((y * c.width + x) * 4) as usize;
        [c.data[i], c.data[i + 1], c.data[i + 2], c.data[i + 3]]
    }

    #[test]
    fn reference_has_a_hole_a_ring_and_a_shadow() {
        let style = FrameStyle { focused: true, radius: 12, shadow: SHADOW, accent: 0x3ddc97, seams: 0, seam: 0xedf2f8 };
        let r = Reference::draw(style, 1);
        let [ml, mt, _, _] = r.margins;
        let tl = &r.corners[0];
        // inside the window: nothing
        assert_eq!(px(tl, ml + 20, mt + 20)[3], 0);
        // the ring hugs the window
        assert!(px(tl, ml - 1, mt + 30)[3] > 20);
        assert!(px(tl, ml + 30, mt - 1)[3] > 20);
        // the shadow fades away from it
        let near = px(tl, ml - 3, mt + 30)[3];
        let far = px(tl, ml - 30, mt + 30)[3];
        assert!(near > far, "near {near} far {far}");
        assert!(far > 0);
        // the corner is rounded off: what shows through the bitten-out square
        // corner of the window is only shadow, far fainter than the ring
        let corner = px(tl, ml + 1, mt + 1)[3];
        let ring = px(tl, ml - 1, mt + 30)[3];
        assert!(corner * 2 < ring, "corner {corner} ring {ring}");
        // the edge strips are uniform along their length
        let top = &r.edges[0];
        assert_eq!(px(top, 0, 5), px(top, 3, 5));
        assert_eq!(top.width, 4);
    }

    #[test]
    fn a_border_only_frame_is_one_pixel() {
        let style = FrameStyle { focused: false, radius: 12, shadow: 0, accent: 0x3ddc97, seams: 0, seam: 0xedf2f8 };
        let r = Reference::draw(style, 1);
        assert_eq!(r.margins, [1, 1, 1, 1]);
        let left = &r.edges[3];
        assert_eq!(left.width, 1);
        assert!(px(left, 0, 1)[3] > 10);
    }

    #[test]
    fn a_seam_brightens_only_the_side_that_touches() {
        let plain = FrameStyle { focused: false, radius: 0, shadow: TILE_SHADOW, accent: 0x3ddc97, seams: 0, seam: 0xedf2f8 };
        let seamed = FrameStyle { seams: SEAM_RIGHT, ..plain };
        let (a, b) = (Reference::draw(plain, 1), Reference::draw(seamed, 1));
        // The right edge is the one that touches: it gains the seam. Both
        // pixels carry the tile's shadow as well, so what the seam is worth
        // is the difference, not the ratio.
        let (dull, bright) = (px(&a.edges[1], 0, 1)[3], px(&b.edges[1], 0, 1)[3]);
        assert!(bright > dull + 90, "dull {dull} bright {bright}");
        // Its three other sides are the hairline they were.
        for i in [0, 2, 3] {
            assert_eq!(px(&a.edges[i], 0, 0), px(&b.edges[i], 0, 0), "edge {i} changed");
        }
        // A seam is loud enough to see against the desktop between two tiles.
        assert!(bright > 150, "seam alpha {bright}");
        // Both neighbours draw their own, and the seam colour is the same
        // whether or not a window has the focus, so the two lines either side
        // of a gap are a pair. The focused one keeps a little of its accent
        // glow underneath, which is a difference you have to look for.
        let focused = Reference::draw(FrameStyle { focused: true, ..seamed }, 1);
        let lit = px(&focused.edges[1], 0, 1)[3];
        assert!(lit.abs_diff(bright) < 40, "unfocused {bright} focused {lit}");
    }

    #[test]
    fn scaled_reference_scales_the_tiles() {
        let style = FrameStyle { focused: true, radius: 12, shadow: TILE_SHADOW, accent: 0x3ddc97, seams: 0, seam: 0xedf2f8 };
        let one = Reference::draw(style, 1);
        let two = Reference::draw(style, 2);
        assert_eq!(two.corners[0].width, one.corners[0].width * 2);
        assert_eq!(two.edges[0].height, one.edges[0].height * 2);
    }
}
