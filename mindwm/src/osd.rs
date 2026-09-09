//! Small cached feedback card. Reuses the Mind bar's fonts and theme colors.
use std::time::{Duration, Instant};
use smithay::{backend::{allocator::Fourcc, renderer::{ImportMem, Renderer,
    element::{Kind, memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement}}}},
    utils::{Logical, Point, Size, Transform}};
use crate::{media::Feedback, text::{alpha, hex, Canvas, Face, Rgba, TextRenderer, ALL_CORNERS}};

#[derive(Default)]
pub struct Osd {
    message: Option<(Feedback, Instant)>,
    buffer: Option<(MemoryRenderBuffer, i32, i32)>,
}

impl Osd {
    pub fn show(&mut self, message: Feedback) {
        self.message = Some((message, Instant::now()));
        self.buffer = None;
    }
    pub fn clear(&mut self) { self.message = None; self.buffer = None; }
    /// A card is showing (or fading): the frame after this one is wanted.
    pub fn active(&self) -> bool {
        self.message.as_ref().is_some_and(|(_, since)| since.elapsed() < Duration::from_millis(1800))
    }

    pub fn render<R>(&mut self, renderer: &mut R, text: &mut TextRenderer,
                     foreground: Rgba, accent: Rgba, output: &str,
                     size: Size<i32, Logical>, scale: f64) -> Option<MemoryRenderBufferRenderElement<R>>
    where R: Renderer + ImportMem, R::TextureId: Clone + Send + 'static {
        let (message, since) = self.message.as_ref()?;
        let elapsed = since.elapsed();
        if elapsed >= Duration::from_millis(1800) { self.clear(); return None; }
        if message.output.as_ref().is_some_and(|name| name != output) || size.w < 200 || size.h < 180 { return None; }
        let width = 320.min(size.w - 32);
        let height = 92;
        let density = scale.ceil().max(1.0) as i32;
        if self.buffer.as_ref().is_none_or(|(_, w, s)| *w != width || *s != density) {
            let mut canvas = Canvas::new(width * density, height * density);
            canvas.fill_rounded_rect(0, 0, width * density, height * density, 18 * density,
                                     ALL_CORNERS, alpha(hex(0x0a111c), 0.94));
            canvas.stroke_rounded_rect(0, 0, width * density, height * density, 18 * density,
                                       ALL_CORNERS, alpha(accent, 0.35));
            text.draw(&mut canvas, 22*density, 17*density, Some((width-44)*density),
                      message.label, 13.0*density as f32, alpha(foreground, 0.7), Face::Label);
            text.draw(&mut canvas, 22*density, 36*density, Some((width-44)*density),
                      &message.value, 21.0*density as f32, foreground, Face::LabelBold);
            if let Some(level) = message.level {
                let bar_width = (width - 44) * density;
                canvas.fill_rounded_rect(22*density, 71*density, bar_width, 5*density, 2*density,
                                         ALL_CORNERS, alpha(foreground, 0.12));
                canvas.fill_rounded_rect(22*density, 71*density,
                    (bar_width as f64 * level.clamp(0.0, 1.0)).round() as i32, 5*density,
                    2*density, ALL_CORNERS, if message.muted { alpha(foreground, 0.4) } else { accent });
            }
            self.buffer = Some((MemoryRenderBuffer::from_slice(&canvas.data, Fourcc::Argb8888,
                (canvas.width, canvas.height), density, Transform::Normal, None), width, density));
        }
        let opacity = ((1800.0 - elapsed.as_millis() as f32) / 180.0).clamp(0.0, 1.0);
        let location: Point<i32, Logical> = ((size.w-width)/2, (size.h-height-88).max(16)).into();
        MemoryRenderBufferRenderElement::from_buffer(renderer, location.to_f64().to_physical(scale),
            &self.buffer.as_ref()?.0, Some(opacity), None, None, Kind::Unspecified).ok()
    }
}
