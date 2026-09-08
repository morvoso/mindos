//! Software text rendering with fontdue into premultiplied BGRA canvases.
//!
//! The Mind bar and the desktop wordmark are drawn on the CPU and uploaded as
//! memory buffers, so they work identically on GLES, Pixman and multi-GPU
//! renderers without any GPU text stack.
//!
//! Faces: Inter (the system face — body text and UI labels, in Regular,
//! Medium and SemiBold), Orbitron Bold (the wordmark), JetBrains Mono
//! (commands, keys, clocks) and DejaVu Sans as the per-character glyph
//! fallback for anything Inter does not cover.
//!
//! Glyphs are rasterised unhinted and composited through a gamma-corrected
//! coverage curve with subpixel horizontal placement, which is what gives
//! macOS text its even weight; see [`TEXT_GAMMA`].

use fontdue::layout::{CoordinateSystem, Layout, LayoutSettings, TextStyle, WrapStyle};
use fontdue::{Font, FontSettings};

pub static FONT_REGULAR: &[u8] = include_bytes!("../resources/Inter-Regular.ttf");
pub static FONT_MEDIUM: &[u8] = include_bytes!("../resources/Inter-Medium.ttf");
pub static FONT_SEMIBOLD: &[u8] = include_bytes!("../resources/Inter-SemiBold.ttf");
pub static FONT_DISPLAY: &[u8] = include_bytes!("../resources/Orbitron-Bold.ttf");
pub static FONT_MONO: &[u8] = include_bytes!("../resources/JetBrainsMono-Regular.ttf");
pub static FONT_FALLBACK: &[u8] = include_bytes!("../resources/DejaVuSans.ttf");
pub static FONT_FALLBACK_BOLD: &[u8] = include_bytes!("../resources/DejaVuSans-Bold.ttf");

/// Indices into [`TextRenderer::fonts`], in load order.
const F_REGULAR: usize = 0;
const F_MEDIUM: usize = 1;
const F_SEMIBOLD: usize = 2;
const F_DISPLAY: usize = 3;
const F_MONO: usize = 4;
const F_FALLBACK: usize = 5;
const F_FALLBACK_BOLD: usize = 6;

/// Straight (non-premultiplied) RGBA in 0..1.
pub type Rgba = [f32; 4];

/// `#rrggbb` as an opaque colour.
pub const fn hex(rgb: u32) -> Rgba {
    [
        ((rgb >> 16) & 0xff) as f32 / 255.0,
        ((rgb >> 8) & 0xff) as f32 / 255.0,
        (rgb & 0xff) as f32 / 255.0,
        1.0,
    ]
}

/// `color` with its alpha replaced.
pub const fn alpha(color: Rgba, a: f32) -> Rgba {
    [color[0], color[1], color[2], a]
}

/// Which typeface to draw with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Face {
    /// Inter Regular: chat text, prose, anything with arbitrary Unicode.
    Body,
    /// Inter SemiBold.
    BodyBold,
    /// Inter Medium: UI labels and hints, a touch heavier than body text so
    /// short uppercase strings hold up against the HUD background.
    Label,
    /// Inter SemiBold.
    LabelBold,
    /// Orbitron Bold: the wordmark.
    Display,
    /// JetBrains Mono: commands, key names and clocks.
    Mono,
}

impl Face {
    fn index(self) -> usize {
        match self {
            Face::Body => F_REGULAR,
            Face::Label => F_MEDIUM,
            Face::BodyBold | Face::LabelBold => F_SEMIBOLD,
            Face::Display => F_DISPLAY,
            Face::Mono => F_MONO,
        }
    }

    /// The DejaVu face used for glyphs this face does not have.
    fn fallback(self) -> usize {
        match self {
            Face::BodyBold | Face::LabelBold | Face::Display => F_FALLBACK_BOLD,
            _ => F_FALLBACK,
        }
    }
}

/// Corner selection for chamfered shapes.
pub const TOP_LEFT: u8 = 1;
pub const TOP_RIGHT: u8 = 2;
pub const BOTTOM_RIGHT: u8 = 4;
pub const BOTTOM_LEFT: u8 = 8;
pub const ALL_CORNERS: u8 = 15;
/// The HUD look: opposite corners cut.
pub const DIAGONAL: u8 = TOP_LEFT | BOTTOM_RIGHT;

#[derive(Clone)]
pub struct Canvas {
    pub width: i32,
    pub height: i32,
    /// Premultiplied BGRA (DRM_FORMAT_ARGB8888 on little endian).
    pub data: Vec<u8>,
}

impl Canvas {
    pub fn new(width: i32, height: i32) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        Canvas {
            width,
            height,
            data: vec![0; (width * height * 4) as usize],
        }
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: Rgba) {
        for yy in y.max(0)..(y + h).min(self.height) {
            for xx in x.max(0)..(x + w).min(self.width) {
                self.blend(xx, yy, color);
            }
        }
    }

    /// 1px outline just inside the rectangle.
    pub fn stroke_rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: Rgba) {
        if w <= 0 || h <= 0 {
            return;
        }
        self.fill_rect(x, y, w, 1, color);
        self.fill_rect(x, y + h - 1, w, 1, color);
        self.fill_rect(x, y + 1, 1, h - 2, color);
        self.fill_rect(x + w - 1, y + 1, 1, h - 2, color);
    }

    /// Is the pixel at (lx, ly) of a w×h rectangle inside the shape whose
    /// selected corners are cut diagonally by `cut` pixels?
    #[inline]
    fn chamfer_inside(lx: i32, ly: i32, w: i32, h: i32, cut: i32, corners: u8) -> bool {
        if lx < 0 || ly < 0 || lx >= w || ly >= h {
            return false;
        }
        let rx = w - 1 - lx;
        let ry = h - 1 - ly;
        !((corners & TOP_LEFT != 0 && lx + ly < cut)
            || (corners & TOP_RIGHT != 0 && rx + ly < cut)
            || (corners & BOTTOM_RIGHT != 0 && rx + ry < cut)
            || (corners & BOTTOM_LEFT != 0 && lx + ry < cut))
    }

    /// Filled rectangle with the selected corners cut diagonally (HUD style).
    pub fn fill_chamfered_rect(&mut self, x: i32, y: i32, w: i32, h: i32, cut: i32, corners: u8, color: Rgba) {
        let cut = cut.clamp(0, w.min(h) / 2);
        for yy in y.max(0)..(y + h).min(self.height) {
            for xx in x.max(0)..(x + w).min(self.width) {
                if Self::chamfer_inside(xx - x, yy - y, w, h, cut, corners) {
                    self.blend(xx, yy, color);
                }
            }
        }
    }

    /// 1px outline of a chamfered rectangle (inside the shape).
    pub fn stroke_chamfered_rect(&mut self, x: i32, y: i32, w: i32, h: i32, cut: i32, corners: u8, color: Rgba) {
        let cut = cut.clamp(0, w.min(h) / 2);
        for yy in y.max(0)..(y + h).min(self.height) {
            for xx in x.max(0)..(x + w).min(self.width) {
                let (lx, ly) = (xx - x, yy - y);
                if !Self::chamfer_inside(lx, ly, w, h, cut, corners) {
                    continue;
                }
                let edge = !Self::chamfer_inside(lx - 1, ly, w, h, cut, corners)
                    || !Self::chamfer_inside(lx + 1, ly, w, h, cut, corners)
                    || !Self::chamfer_inside(lx, ly - 1, w, h, cut, corners)
                    || !Self::chamfer_inside(lx, ly + 1, w, h, cut, corners);
                if edge {
                    self.blend(xx, yy, color);
                }
            }
        }
    }

    /// Coverage (0..1) of the pixel at (lx, ly) of a w×h rectangle whose
    /// selected corners are rounded with radius `r`: 1 inside, 0 outside, a
    /// fraction on the curve so the edge is anti-aliased.
    #[inline]
    pub(crate) fn rounded_coverage(lx: i32, ly: i32, w: i32, h: i32, r: i32, corners: u8) -> f32 {
        if lx < 0 || ly < 0 || lx >= w || ly >= h {
            return 0.0;
        }
        if r <= 0 {
            return 1.0;
        }
        // which corner circle (if any) governs this pixel
        let (cx, cy) = if lx < r && ly < r && corners & TOP_LEFT != 0 {
            (r, r)
        } else if lx >= w - r && ly < r && corners & TOP_RIGHT != 0 {
            (w - r, r)
        } else if lx >= w - r && ly >= h - r && corners & BOTTOM_RIGHT != 0 {
            (w - r, h - r)
        } else if lx < r && ly >= h - r && corners & BOTTOM_LEFT != 0 {
            (r, h - r)
        } else {
            return 1.0;
        };
        // distance from the pixel centre to the circle centre (pixel centres
        // sit at +0.5; the circle centre sits on a pixel corner)
        let dx = lx as f32 + 0.5 - cx as f32;
        let dy = ly as f32 + 0.5 - cy as f32;
        let d = (dx * dx + dy * dy).sqrt();
        (r as f32 - d + 0.5).clamp(0.0, 1.0)
    }

    /// Filled rectangle with the selected corners rounded (anti-aliased).
    pub fn fill_rounded_rect(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32, corners: u8, color: Rgba) {
        let r = r.clamp(0, w.min(h) / 2);
        for yy in y.max(0)..(y + h).min(self.height) {
            for xx in x.max(0)..(x + w).min(self.width) {
                let cov = Self::rounded_coverage(xx - x, yy - y, w, h, r, corners);
                if cov > 0.0 {
                    self.blend(xx, yy, [color[0], color[1], color[2], color[3] * cov]);
                }
            }
        }
    }

    /// 1px anti-aliased outline of a rounded rectangle, just inside the shape.
    pub fn stroke_rounded_rect(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32, corners: u8, color: Rgba) {
        let r = r.clamp(0, w.min(h) / 2);
        for yy in y.max(0)..(y + h).min(self.height) {
            for xx in x.max(0)..(x + w).min(self.width) {
                let (lx, ly) = (xx - x, yy - y);
                let outer = Self::rounded_coverage(lx, ly, w, h, r, corners);
                if outer <= 0.0 {
                    continue;
                }
                // the same shape shrunk by one pixel; the ring between is the stroke
                let inner = Self::rounded_coverage(lx - 1, ly - 1, w - 2, h - 2, (r - 1).max(0), corners);
                let cov = (outer - inner).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend(xx, yy, [color[0], color[1], color[2], color[3] * cov]);
                }
            }
        }
    }

    /// Horizontal line of `thickness` rows with a soft glow of `spread` rows
    /// fading out above and below.
    pub fn hline_glow(&mut self, x: i32, y: i32, w: i32, thickness: i32, spread: i32, color: Rgba) {
        let t = thickness.max(1);
        for i in 1..=spread {
            let a = color[3] * (1.0 - i as f32 / (spread as f32 + 1.0)) * 0.45;
            let c = [color[0], color[1], color[2], a];
            self.fill_rect(x, y - i, w, 1, c);
            self.fill_rect(x, y + t - 1 + i, w, 1, c);
        }
        self.fill_rect(x, y, w, t, color);
    }

    /// A straight line of `thickness` px between two points (small squares
    /// along the way; fine for glyphs, opaque colours look best).
    pub fn line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, thickness: i32, color: Rgba) {
        let t = thickness.max(1);
        let steps = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
        let mut last = None;
        for i in 0..=steps {
            let x = x0 + ((x1 - x0) as f32 * i as f32 / steps as f32).round() as i32;
            let y = y0 + ((y1 - y0) as f32 * i as f32 / steps as f32).round() as i32;
            if last == Some((x, y)) {
                continue;
            }
            last = Some((x, y));
            self.fill_rect(x - t / 2, y - t / 2, t, t, color);
        }
    }

    pub fn fill_circle(&mut self, cx: i32, cy: i32, r: i32, color: Rgba) {
        for yy in (cy - r).max(0)..=(cy + r).min(self.height - 1) {
            for xx in (cx - r).max(0)..=(cx + r).min(self.width - 1) {
                let dx = xx - cx;
                let dy = yy - cy;
                let d2 = (dx * dx + dy * dy) as f32;
                let r2 = (r as f32 + 0.5) * (r as f32 + 0.5);
                if d2 <= r2 {
                    // soft edge
                    let edge = (r as f32 + 0.5 - d2.sqrt()).clamp(0.0, 1.0);
                    self.blend(xx, yy, [color[0], color[1], color[2], color[3] * edge]);
                }
            }
        }
    }

    /// A soft drop shadow: `color` under a w×h rounded rectangle (selected
    /// corners rounded with `r`), blurred so it fades out over `blur` pixels
    /// around the shape. The mask is a gaussian blur of the shape's coverage,
    /// done as two separable passes; the cost is (area + blur²) × blur, so
    /// callers draw large shadows once and cache them.
    pub fn shadow_rounded_rect(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32, corners: u8, blur: i32, color: Rgba) {
        if w <= 0 || h <= 0 || color[3] <= 0.0 {
            return;
        }
        let blur = blur.max(0);
        let r = r.clamp(0, w.min(h) / 2);
        // the region the shadow can touch
        let bx = x - blur;
        let by = y - blur;
        let bw = w + 2 * blur;
        let bh = h + 2 * blur;
        let mut mask = vec![0f32; (bw * bh) as usize];
        for yy in 0..bh {
            for xx in 0..bw {
                mask[(yy * bw + xx) as usize] = Self::rounded_coverage(xx - blur, yy - blur, w, h, r, corners);
            }
        }
        if blur > 0 {
            let sigma = blur as f32 / 2.2;
            let kernel: Vec<f32> = (-blur..=blur)
                .map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp())
                .collect();
            let sum: f32 = kernel.iter().sum();
            let kernel: Vec<f32> = kernel.into_iter().map(|k| k / sum).collect();
            let mut tmp = vec![0f32; (bw * bh) as usize];
            for yy in 0..bh {
                for xx in 0..bw {
                    let mut acc = 0.0;
                    for (k, weight) in kernel.iter().enumerate() {
                        let sx = xx + k as i32 - blur;
                        if sx >= 0 && sx < bw {
                            acc += mask[(yy * bw + sx) as usize] * weight;
                        }
                    }
                    tmp[(yy * bw + xx) as usize] = acc;
                }
            }
            for yy in 0..bh {
                for xx in 0..bw {
                    let mut acc = 0.0;
                    for (k, weight) in kernel.iter().enumerate() {
                        let sy = yy + k as i32 - blur;
                        if sy >= 0 && sy < bh {
                            acc += tmp[(sy * bw + xx) as usize] * weight;
                        }
                    }
                    mask[(yy * bw + xx) as usize] = acc;
                }
            }
        }
        for yy in 0..bh {
            let py = by + yy;
            if py < 0 || py >= self.height {
                continue;
            }
            for xx in 0..bw {
                let px = bx + xx;
                if px < 0 || px >= self.width {
                    continue;
                }
                let a = mask[(yy * bw + xx) as usize];
                if a > 0.002 {
                    self.blend(px, py, [color[0], color[1], color[2], color[3] * a]);
                }
            }
        }
    }

    /// Multiply every pixel's alpha (and colour: the data is premultiplied)
    /// by `1 - coverage` of a rounded rectangle: cuts the shape out of the
    /// canvas, with anti-aliased edges. Used to punch the window out of its
    /// shadow so a translucent window does not darken itself.
    pub fn cut_rounded_rect(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32, corners: u8) {
        let r = r.clamp(0, w.min(h) / 2);
        for yy in y.max(0)..(y + h).min(self.height) {
            for xx in x.max(0)..(x + w).min(self.width) {
                let cov = Self::rounded_coverage(xx - x, yy - y, w, h, r, corners);
                if cov <= 0.0 {
                    continue;
                }
                let keep = 1.0 - cov;
                let i = ((yy * self.width + xx) * 4) as usize;
                for b in &mut self.data[i..i + 4] {
                    *b = (*b as f32 * keep).round() as u8;
                }
            }
        }
    }

    /// Composite another canvas onto this one at (x, y) (source over).
    pub fn draw_scaled_canvas(&mut self, x: i32, y: i32, w: i32, h: i32, src: &Canvas) {
        if w <= 0 || h <= 0 || src.width <= 0 || src.height <= 0 { return; }
        let mut scaled = Canvas::new(w, h);
        for yy in 0..h {
            for xx in 0..w {
                let from = (((yy * src.height / h) * src.width + xx * src.width / w) * 4) as usize;
                let to = ((yy * w + xx) * 4) as usize;
                scaled.data[to..to + 4].copy_from_slice(&src.data[from..from + 4]);
            }
        }
        self.draw_canvas(x, y, &scaled);
    }

    /// Analytic rounded-rectangle falloff: bounded work per pixel, independent
    /// of blur radius. Large HiDPI launcher shadows must not stall input.
    pub fn soft_shadow(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32, blur: i32, color: Rgba) {
        let r = r.max(0).min(w.min(h) / 2) as f32;
        let spread = blur.max(1) as f32;
        for yy in (y - blur).max(0)..(y + h + blur).min(self.height) {
            for xx in (x - blur).max(0)..(x + w + blur).min(self.width) {
                let qx = ((xx - x) as f32 - w as f32 * 0.5).abs() - (w as f32 * 0.5 - r);
                let qy = ((yy - y) as f32 - h as f32 * 0.5).abs() - (h as f32 * 0.5 - r);
                let distance = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r;
                let a = (1.0 - distance.max(0.0) / spread).clamp(0.0, 1.0);
                if a > 0.0 { self.blend(xx, yy, [color[0], color[1], color[2], color[3] * a * a]); }
            }
        }
    }

    pub fn draw_canvas(&mut self, x: i32, y: i32, src: &Canvas) {
        for sy in 0..src.height {
            let dy = y + sy;
            if dy < 0 || dy >= self.height {
                continue;
            }
            for sx in 0..src.width {
                let dx = x + sx;
                if dx < 0 || dx >= self.width {
                    continue;
                }
                let si = ((sy * src.width + sx) * 4) as usize;
                let sa = src.data[si + 3];
                if sa == 0 {
                    continue;
                }
                let di = ((dy * self.width + dx) * 4) as usize;
                let inv = 1.0 - sa as f32 / 255.0;
                for c in 0..4 {
                    let v = src.data[si + c] as f32 + self.data[di + c] as f32 * inv;
                    self.data[di + c] = v.round().min(255.0) as u8;
                }
            }
        }
    }

    /// A copy of the w×h pixels at (x, y); pixels outside stay transparent.
    pub fn crop(&self, x: i32, y: i32, w: i32, h: i32) -> Canvas {
        let mut out = Canvas::new(w, h);
        for yy in 0..out.height {
            let sy = y + yy;
            if sy < 0 || sy >= self.height {
                continue;
            }
            for xx in 0..out.width {
                let sx = x + xx;
                if sx < 0 || sx >= self.width {
                    continue;
                }
                let si = ((sy * self.width + sx) * 4) as usize;
                let di = ((yy * out.width + xx) * 4) as usize;
                out.data[di..di + 4].copy_from_slice(&self.data[si..si + 4]);
            }
        }
        out
    }

    #[inline]
    pub fn blend(&mut self, x: i32, y: i32, color: Rgba) {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return;
        }
        let a = color[3].clamp(0.0, 1.0);
        if a <= 0.0 {
            return;
        }
        let i = ((y * self.width + x) * 4) as usize;
        let inv = 1.0 - a;
        let px = &mut self.data[i..i + 4];
        px[0] = (color[2] * a * 255.0 + px[0] as f32 * inv).round().min(255.0) as u8;
        px[1] = (color[1] * a * 255.0 + px[1] as f32 * inv).round().min(255.0) as u8;
        px[2] = (color[0] * a * 255.0 + px[2] as f32 * inv).round().min(255.0) as u8;
        px[3] = (a * 255.0 + px[3] as f32 * inv).round().min(255.0) as u8;
    }
}

/// Gamma applied to glyph coverage before it is composited.
///
/// CoreText composites text in a gamma-corrected space rather than blending
/// coverage straight into sRGB the way most Linux stacks do. The difference
/// shows on the partially covered pixels that make up a stem's edges: linear
/// blending renders them too close to the background, so light text on a dark
/// ground looks thin and washed out, and dark text on a light ground looks
/// smeared. Feeding coverage through `c^(1/GAMMA)` for light text (and
/// `c^GAMMA` for dark text) restores the weight the face was drawn with.
///
/// 1.45 matches the correction CoreText applies at typical UI sizes; raising
/// it makes text heavier, 1.0 disables the curve.
const TEXT_GAMMA: f32 = 1.45;

/// Luminance above which text counts as light-on-dark for [`TEXT_GAMMA`].
const LIGHT_TEXT_LUMA: f32 = 0.5;

pub struct TextRenderer {
    fonts: Vec<Font>,
    layout: Layout,
    /// Coverage → alpha curves, indexed by [`Self::curve_for`]: 0 is
    /// light-on-dark, 1 is dark-on-light.
    coverage: [[f32; 256]; 2],
}

impl Default for TextRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl TextRenderer {
    pub fn new() -> Self {
        let load = |bytes: &'static [u8], name: &str| {
            Font::from_bytes(bytes, FontSettings::default()).unwrap_or_else(|err| panic!("embedded font {name}: {err}"))
        };
        let curve = |exp: f32| {
            let mut lut = [0.0f32; 256];
            for (i, v) in lut.iter_mut().enumerate() {
                *v = (i as f32 / 255.0).powf(exp);
            }
            lut
        };
        TextRenderer {
            fonts: vec![
                load(FONT_REGULAR, "Inter"),
                load(FONT_MEDIUM, "Inter Medium"),
                load(FONT_SEMIBOLD, "Inter SemiBold"),
                load(FONT_DISPLAY, "Orbitron Bold"),
                load(FONT_MONO, "JetBrains Mono"),
                load(FONT_FALLBACK, "DejaVu Sans"),
                load(FONT_FALLBACK_BOLD, "DejaVu Sans Bold"),
            ],
            layout: Layout::new(CoordinateSystem::PositiveYDown),
            coverage: [curve(1.0 / TEXT_GAMMA), curve(TEXT_GAMMA)],
        }
    }

    /// Which [`Self::coverage`] curve `color` wants. Perceptual luminance of
    /// the *text*: the HUD is dark, so light text is the common case and gets
    /// the thickening curve.
    fn curve_for(color: Rgba) -> usize {
        let luma = 0.2126 * color[0] + 0.7152 * color[1] + 0.0722 * color[2];
        usize::from(luma <= LIGHT_TEXT_LUMA)
    }

    fn settings(x: f32, y: f32, max_width: Option<f32>) -> LayoutSettings {
        LayoutSettings {
            x,
            y,
            max_width,
            wrap_style: WrapStyle::Word,
            wrap_hard_breaks: true,
            ..LayoutSettings::default()
        }
    }

    /// Append `text` to the layout, switching to the fallback face for every
    /// character the chosen face cannot draw.
    fn append_runs(&mut self, text: &str, px: f32, face: Face) {
        let primary = face.index();
        let fallback = face.fallback();
        let mut run = String::new();
        let mut run_font = primary;
        for c in text.chars() {
            let font = if c.is_whitespace() || self.fonts[primary].lookup_glyph_index(c) != 0 {
                primary
            } else {
                fallback
            };
            if font != run_font && !run.is_empty() {
                self.layout
                    .append(&self.fonts, &TextStyle::new(&run, px, run_font));
                run.clear();
            }
            run_font = font;
            run.push(c);
        }
        if !run.is_empty() {
            self.layout
                .append(&self.fonts, &TextStyle::new(&run, px, run_font));
        }
    }

    /// Composite the laid-out glyphs into `canvas`.
    ///
    /// Two things here are deliberate and are what make the result look like
    /// CoreText rather than a stock FreeType surface:
    ///
    /// * **Subpixel horizontal placement.** fontdue rasterises on integer
    ///   origins, so snapping `glyph.x` would quantise every advance to a whole
    ///   pixel and make letter spacing visibly uneven. Instead the coverage
    ///   mask is resampled by the fractional part with a two-tap filter, which
    ///   places the glyph where the layout actually put it. Baselines stay
    ///   snapped vertically — that keeps horizontal stems crisp, the one thing
    ///   light hinting is still good for.
    /// * **Gamma-corrected coverage**, via [`TEXT_GAMMA`].
    fn rasterize(&mut self, canvas: &mut Canvas, color: Rgba) {
        self.rasterize_runs(canvas, color, None)
    }

    /// [`Self::rasterize`], with `code` (when given) used for every glyph that
    /// came from the mono face — the one thing a rich run needs a second
    /// colour for.
    fn rasterize_runs(&mut self, canvas: &mut Canvas, color: Rgba, code: Option<Rgba>) {
        let curves = [
            &self.coverage[Self::curve_for(color)],
            &self.coverage[Self::curve_for(code.unwrap_or(color))],
        ];
        for glyph in self.layout.glyphs() {
            let mono = code.is_some() && glyph.font_index == F_MONO;
            let color = if mono { code.unwrap_or(color) } else { color };
            let curve = curves[usize::from(mono)];
            if glyph.width == 0 || glyph.height == 0 {
                continue;
            }
            let (metrics, bitmap) = self.fonts[glyph.font_index].rasterize_config(glyph.key);
            let gx = glyph.x.floor() as i32;
            let gy = glyph.y.round() as i32;
            // How far right of `gx` the glyph really sits, in pixels.
            let shift = glyph.x - glyph.x.floor();
            for row in 0..metrics.height {
                let line = &bitmap[row * metrics.width..(row + 1) * metrics.width];
                // One column wider than the mask: the shift spills ink right.
                for col in 0..=metrics.width {
                    let left = if col > 0 { line[col - 1] as f32 } else { 0.0 };
                    let right = if col < metrics.width { line[col] as f32 } else { 0.0 };
                    let coverage = right * (1.0 - shift) + left * shift;
                    if coverage <= 0.0 {
                        continue;
                    }
                    let a = color[3] * curve[coverage.round().clamp(0.0, 255.0) as usize];
                    canvas.blend(gx + col as i32, gy + row as i32, [color[0], color[1], color[2], a]);
                }
            }
        }
    }

    /// Draw `text` with its top-left corner at (x, y), wrapping at `max_width`.
    /// Returns the height used in pixels.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        canvas: &mut Canvas,
        x: i32,
        y: i32,
        max_width: Option<i32>,
        text: &str,
        px: f32,
        color: Rgba,
        face: Face,
    ) -> i32 {
        self.layout
            .reset(&Self::settings(x as f32, y as f32, max_width.map(|w| w as f32)));
        self.append_runs(text, px, face);
        self.rasterize(canvas, color);
        self.layout.height().ceil() as i32
    }

    /// Width and height `text` would occupy.
    pub fn measure(&mut self, text: &str, px: f32, max_width: Option<i32>, face: Face) -> (i32, i32) {
        self.layout
            .reset(&Self::settings(0.0, 0.0, max_width.map(|w| w as f32)));
        self.append_runs(text, px, face);
        let width = self
            .layout
            .glyphs()
            .iter()
            .map(|g| g.x + g.width as f32)
            .fold(0.0_f32, f32::max);
        (width.ceil() as i32, self.layout.height().ceil() as i32)
    }

    /// Lay out `runs` — pieces of one paragraph, each with its own face — into
    /// the current layout. Mono runs are set a little smaller so JetBrains
    /// Mono sits on the same line as Inter without looking oversized.
    fn append_rich(&mut self, runs: &[(&str, Face)], px: f32) {
        for (text, face) in runs {
            let px = if *face == Face::Mono { px * 0.94 } else { px };
            self.append_runs(text, px, *face);
        }
    }

    /// [`Self::draw`] for text whose face changes part-way through (the Mind
    /// bar's Markdown). `code` colours the mono runs when given.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_rich(
        &mut self,
        canvas: &mut Canvas,
        x: i32,
        y: i32,
        max_width: Option<i32>,
        runs: &[(&str, Face)],
        px: f32,
        color: Rgba,
        code: Option<Rgba>,
    ) -> i32 {
        self.layout
            .reset(&Self::settings(x as f32, y as f32, max_width.map(|w| w as f32)));
        self.append_rich(runs, px);
        self.rasterize_runs(canvas, color, code);
        self.layout.height().ceil() as i32
    }

    /// Width and height [`Self::draw_rich`] would occupy.
    pub fn measure_rich(&mut self, runs: &[(&str, Face)], px: f32, max_width: Option<i32>) -> (i32, i32) {
        self.layout
            .reset(&Self::settings(0.0, 0.0, max_width.map(|w| w as f32)));
        self.append_rich(runs, px);
        let width = self
            .layout
            .glyphs()
            .iter()
            .map(|g| g.x + g.width as f32)
            .fold(0.0_f32, f32::max);
        (width.ceil() as i32, self.layout.height().ceil() as i32)
    }

    /// Draw a single line with extra `tracking` pixels between characters
    /// (letter-spaced labels). Returns the width used.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_spaced(
        &mut self,
        canvas: &mut Canvas,
        x: i32,
        y: i32,
        text: &str,
        px: f32,
        color: Rgba,
        face: Face,
        tracking: i32,
    ) -> i32 {
        let mut cx = x;
        let mut buf = [0u8; 4];
        for c in text.chars() {
            let s: &str = c.encode_utf8(&mut buf);
            let (w, _) = self.measure(s, px, None, face);
            let (adv, _) = self.advance(s, px, face);
            self.draw(canvas, cx, y, None, s, px, color, face);
            cx += adv.max(w) + tracking;
        }
        cx - x - tracking
    }

    pub fn measure_spaced(&mut self, text: &str, px: f32, face: Face, tracking: i32) -> i32 {
        let mut total = 0;
        let mut buf = [0u8; 4];
        for c in text.chars() {
            let s: &str = c.encode_utf8(&mut buf);
            let (w, _) = self.measure(s, px, None, face);
            let (adv, _) = self.advance(s, px, face);
            total += adv.max(w) + tracking;
        }
        (total - tracking).max(0)
    }

    /// Horizontal advance (not ink width) of a short string.
    fn advance(&mut self, text: &str, px: f32, face: Face) -> (i32, i32) {
        let font = if text
            .chars()
            .all(|c| c.is_whitespace() || self.fonts[face.index()].lookup_glyph_index(c) != 0)
        {
            face.index()
        } else {
            face.fallback()
        };
        let adv: f32 = text
            .chars()
            .map(|c| self.fonts[font].metrics(c, px).advance_width)
            .sum();
        (adv.round() as i32, 0)
    }

    /// `draw` with a soft glow: the text is stamped around its position in
    /// `glow` (low alpha) before the real text is drawn on top.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_glow(
        &mut self,
        canvas: &mut Canvas,
        x: i32,
        y: i32,
        text: &str,
        px: f32,
        color: Rgba,
        glow: Rgba,
        radius: i32,
        face: Face,
    ) -> i32 {
        let r = radius.max(1);
        for dy in -r..=r {
            for dx in -r..=r {
                let d = ((dx * dx + dy * dy) as f32).sqrt();
                if d > r as f32 + 0.5 || (dx == 0 && dy == 0) {
                    continue;
                }
                let falloff = 1.0 - d / (r as f32 + 1.0);
                let a = glow[3] * falloff * falloff;
                self.draw(canvas, x + dx, y + dy, None, text, px, [glow[0], glow[1], glow[2], a], face);
            }
        }
        self.draw(canvas, x, y, None, text, px, color, face)
    }

    /// `draw_spaced` with a soft glow behind it: the wordmark of the startup
    /// screen, which has to match `measure_spaced` exactly to stay centred, so
    /// the glow is stamped from the same routine rather than a plain `draw`.
    /// Wide glows are stamped every other pixel; the result is as soft and
    /// costs a quarter as much, which matters on the first frame after boot.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_spaced_glow(
        &mut self,
        canvas: &mut Canvas,
        x: i32,
        y: i32,
        text: &str,
        px: f32,
        color: Rgba,
        glow: Rgba,
        radius: i32,
        face: Face,
        tracking: i32,
    ) -> i32 {
        let r = radius.max(1);
        let step = if r > 4 { 2 } else { 1 };
        let mut dy = -r;
        while dy <= r {
            let mut dx = -r;
            while dx <= r {
                let d = ((dx * dx + dy * dy) as f32).sqrt();
                if d <= r as f32 + 0.5 && !(dx == 0 && dy == 0) {
                    let falloff = 1.0 - d / (r as f32 + 1.0);
                    let a = glow[3] * falloff * falloff * step as f32;
                    self.draw_spaced(canvas, x + dx, y + dy, text, px, [glow[0], glow[1], glow[2], a], face, tracking);
                }
                dx += step;
            }
            dy += step;
        }
        self.draw_spaced(canvas, x, y, text, px, color, face, tracking)
    }

    pub fn line_height(&self, px: f32, face: Face) -> i32 {
        self.fonts[face.index()]
            .horizontal_line_metrics(px)
            .map(|m| m.new_line_size.ceil() as i32)
            .unwrap_or(px as i32 + 4)
    }
}
