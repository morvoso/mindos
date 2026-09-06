//! Software text rendering with fontdue into premultiplied BGRA canvases.
//!
//! The Mind bar and the desktop wordmark are drawn on the CPU and uploaded as
//! memory buffers, so they work identically on GLES, Pixman and multi-GPU
//! renderers without any GPU text stack.

use fontdue::layout::{CoordinateSystem, Layout, LayoutSettings, TextStyle, WrapStyle};
use fontdue::{Font, FontSettings};

pub static FONT_REGULAR: &[u8] = include_bytes!("../resources/DejaVuSans.ttf");
pub static FONT_BOLD: &[u8] = include_bytes!("../resources/DejaVuSans-Bold.ttf");

/// Straight (non-premultiplied) RGBA in 0..1.
pub type Rgba = [f32; 4];

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

    pub fn fill_rounded_rect(&mut self, x: i32, y: i32, w: i32, h: i32, radius: i32, color: Rgba) {
        let r = radius.min(w / 2).min(h / 2).max(0);
        for yy in y.max(0)..(y + h).min(self.height) {
            for xx in x.max(0)..(x + w).min(self.width) {
                let lx = xx - x;
                let ly = yy - y;
                let cx = if lx < r {
                    r - lx
                } else if lx >= w - r {
                    lx - (w - r - 1)
                } else {
                    0
                };
                let cy = if ly < r {
                    r - ly
                } else if ly >= h - r {
                    ly - (h - r - 1)
                } else {
                    0
                };
                if cx * cx + cy * cy <= r * r {
                    self.blend(xx, yy, color);
                }
            }
        }
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

pub struct TextRenderer {
    fonts: Vec<Font>,
    layout: Layout,
}

impl Default for TextRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl TextRenderer {
    pub fn new() -> Self {
        let regular = Font::from_bytes(FONT_REGULAR, FontSettings::default()).expect("embedded regular font");
        let bold = Font::from_bytes(FONT_BOLD, FontSettings::default()).expect("embedded bold font");
        TextRenderer {
            fonts: vec![regular, bold],
            layout: Layout::new(CoordinateSystem::PositiveYDown),
        }
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
        bold: bool,
    ) -> i32 {
        self.layout
            .reset(&Self::settings(x as f32, y as f32, max_width.map(|w| w as f32)));
        self.layout
            .append(&self.fonts, &TextStyle::new(text, px, usize::from(bold)));
        for glyph in self.layout.glyphs() {
            if glyph.width == 0 || glyph.height == 0 {
                continue;
            }
            let (metrics, bitmap) = self.fonts[glyph.font_index].rasterize_config(glyph.key);
            let gx = glyph.x.round() as i32;
            let gy = glyph.y.round() as i32;
            for row in 0..metrics.height {
                for col in 0..metrics.width {
                    let coverage = bitmap[row * metrics.width + col];
                    if coverage > 0 {
                        let a = color[3] * (coverage as f32 / 255.0);
                        canvas.blend(gx + col as i32, gy + row as i32, [color[0], color[1], color[2], a]);
                    }
                }
            }
        }
        self.layout.height().ceil() as i32
    }

    /// Width and height `text` would occupy.
    pub fn measure(&mut self, text: &str, px: f32, max_width: Option<i32>, bold: bool) -> (i32, i32) {
        self.layout
            .reset(&Self::settings(0.0, 0.0, max_width.map(|w| w as f32)));
        self.layout
            .append(&self.fonts, &TextStyle::new(text, px, usize::from(bold)));
        let width = self
            .layout
            .glyphs()
            .iter()
            .map(|g| g.x + g.width as f32)
            .fold(0.0_f32, f32::max);
        (width.ceil() as i32, self.layout.height().ceil() as i32)
    }

    pub fn line_height(&self, px: f32) -> i32 {
        self.fonts[0]
            .horizontal_line_metrics(px)
            .map(|m| m.new_line_size.ceil() as i32)
            .unwrap_or(px as i32 + 4)
    }
}
