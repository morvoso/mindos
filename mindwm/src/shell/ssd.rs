//! Server-side decorations: the MindOS title bar.
//!
//! The compositor draws the bar for every toplevel that negotiates
//! server-side decorations (xdg-decoration, which is what Qt, GTK, foot,
//! Chromium and most toolkits ask for when the compositor prefers it) and
//! for X11 windows that do not ask to be undecorated. It uses the same
//! tokens and fonts as the shell: a translucent glass bar with a sheen along
//! its top edge, the title in Inter, a cyan line under the bar of the focused
//! window and minimise / maximise / close glyphs on the right. The bar is
//! rasterised on the CPU into a memory buffer that is only redrawn when
//! something about it changed (title, focus, hover, width, maximised state)
//! and composited by the GPU like everything else; the shadow and border
//! around the whole window are `frame.rs`.

use std::cell::{RefCell, RefMut};
use std::time::{Duration, Instant};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                AsRenderElements, Kind,
            },
            ImportMem, Renderer,
        },
    },
    desktop::WindowSurface,
    input::Seat,
    utils::{Logical, Physical, Point, Scale, Serial, Transform},
};

use crate::{
    state::Backend,
    text::{alpha, hex, Canvas, Face, Rgba, TextRenderer, TOP_LEFT, TOP_RIGHT},
    AnvilState,
};

use super::{frame::Frame, WindowElement};

/// Height of the bar in logical pixels.
pub const HEADER_BAR_HEIGHT: i32 = 32;
/// Radius of the window's top corners (the frame follows it).
pub const RADIUS: i32 = 12;
const BUTTON_WIDTH: i32 = 38;
const TITLE_PX: f32 = 13.5;
const DOUBLE_CLICK: Duration = Duration::from_millis(350);
const BTN_LEFT: u32 = 0x110;

const BG: Rgba = hex(0x0d1219);
const WHITE: Rgba = hex(0xffffff);
const FG: Rgba = hex(0xe6edf3);
const FG_DIM: Rgba = hex(0x8b9bb0);
const FG_FAINT: Rgba = hex(0x55657a);
const ACCENT: Rgba = hex(0x19e3ff);
const DANGER: Rgba = hex(0xff5d8f);

thread_local! {
    // One font stack for every bar; the compositor is single-threaded.
    static TEXT: RefCell<Option<TextRenderer>> = const { RefCell::new(None) };
}

fn with_text<T>(f: impl FnOnce(&mut TextRenderer) -> T) -> T {
    TEXT.with(|cell| {
        let mut slot = cell.borrow_mut();
        let text = slot.get_or_insert_with(TextRenderer::new);
        f(text)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Minimize,
    Maximize,
    Close,
}

pub struct WindowState {
    /// The window negotiated (or was assigned) server-side decorations.
    pub is_ssd: bool,
    pub header_bar: HeaderBar,
    /// The shadow and border around the window.
    pub frame: Frame,
}

#[derive(Debug)]
struct Cached {
    buffer: MemoryRenderBuffer,
    width: i32,
    scale: i32,
}

#[derive(Debug)]
pub struct HeaderBar {
    pointer_loc: Option<Point<f64, Logical>>,
    width: i32,
    title: String,
    focused: bool,
    maximized: bool,
    /// A tile of the tiling modes: no minimise or maximise button, no
    /// double-click to maximise (the layout owns the size).
    tiled: bool,
    hover: Option<Button>,
    pressed: Option<Button>,
    last_press: Option<Instant>,
    cache: Option<Cached>,
    dirty: bool,
}

impl Default for HeaderBar {
    fn default() -> Self {
        HeaderBar {
            pointer_loc: None,
            width: 0,
            title: String::new(),
            focused: false,
            maximized: false,
            tiled: false,
            hover: None,
            pressed: None,
            last_press: None,
            cache: None,
            dirty: true,
        }
    }
}

impl HeaderBar {
    pub fn set_title(&mut self, title: &str) {
        if self.title != title {
            self.title = title.to_string();
            self.dirty = true;
        }
    }

    pub fn set_focused(&mut self, focused: bool) {
        if self.focused != focused {
            self.focused = focused;
            self.dirty = true;
        }
    }

    pub fn set_maximized(&mut self, maximized: bool) {
        if self.maximized != maximized {
            self.maximized = maximized;
            self.dirty = true;
        }
    }

    pub fn set_tiled(&mut self, tiled: bool) {
        if self.tiled != tiled {
            self.tiled = tiled;
            self.hover = None;
            self.pressed = None;
            self.dirty = true;
        }
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    pub fn is_maximized(&self) -> bool {
        self.maximized
    }

    pub fn is_tiled(&self) -> bool {
        self.tiled
    }

    /// The buttons of the bar, left to right.
    fn buttons(&self) -> &'static [Button] {
        if self.tiled {
            &[Button::Close]
        } else {
            &[Button::Minimize, Button::Maximize, Button::Close]
        }
    }

    fn button_at(&self, x: f64) -> Option<Button> {
        if x < 0.0 || x > self.width as f64 {
            return None;
        }
        let from_right = self.width as f64 - x;
        let bw = BUTTON_WIDTH as f64;
        let buttons = self.buttons();
        let slot = (from_right / bw).ceil().max(1.0) as usize;
        if slot <= buttons.len() {
            Some(buttons[buttons.len() - slot])
        } else {
            None
        }
    }

    pub fn pointer_enter(&mut self, loc: Point<f64, Logical>) {
        self.pointer_loc = Some(loc);
        let hover = self.button_at(loc.x);
        if hover != self.hover {
            self.hover = hover;
            self.dirty = true;
        }
    }

    pub fn pointer_leave(&mut self) {
        self.pointer_loc = None;
        if self.hover.is_some() || self.pressed.is_some() {
            self.hover = None;
            self.pressed = None;
            self.dirty = true;
        }
    }

    /// A pointer button on the bar. Buttons act on release (so a slip off the
    /// button cancels), a press anywhere else starts a move and a double
    /// click on the title toggles maximise.
    pub fn button<BackendData: Backend>(
        &mut self,
        seat: &Seat<AnvilState<BackendData>>,
        state: &mut AnvilState<BackendData>,
        window: &WindowElement,
        serial: Serial,
        button: u32,
        pressed: bool,
    ) {
        if button != BTN_LEFT {
            return;
        }
        let hit = self.pointer_loc.and_then(|loc| self.button_at(loc.x));
        if pressed {
            self.pressed = hit;
            if hit.is_some() {
                self.dirty = true;
                return;
            }
            let now = Instant::now();
            let double = !self.tiled
                && self
                    .last_press
                    .map(|t| now.duration_since(t) < DOUBLE_CLICK)
                    .unwrap_or(false);
            if double {
                self.last_press = None;
                let window = window.clone();
                state
                    .handle
                    .insert_idle(move |data| data.toggle_maximize_window(&window));
            } else {
                self.last_press = Some(now);
                Self::start_move(seat, state, window, serial);
            }
        } else {
            let was = self.pressed.take();
            if was.is_some() {
                self.dirty = true;
            }
            if let (Some(b), Some(h)) = (was, hit) {
                if b == h {
                    Self::activate(state, window, b);
                }
            }
        }
    }

    pub fn touch_down<BackendData: Backend>(
        &mut self,
        seat: &Seat<AnvilState<BackendData>>,
        state: &mut AnvilState<BackendData>,
        window: &WindowElement,
        serial: Serial,
    ) {
        let hit = self.pointer_loc.and_then(|loc| self.button_at(loc.x));
        self.pressed = hit;
        if hit.is_some() {
            self.dirty = true;
        } else {
            Self::start_move(seat, state, window, serial);
        }
    }

    pub fn touch_up<BackendData: Backend>(
        &mut self,
        _seat: &Seat<AnvilState<BackendData>>,
        state: &mut AnvilState<BackendData>,
        window: &WindowElement,
        _serial: Serial,
    ) {
        let hit = self.pointer_loc.and_then(|loc| self.button_at(loc.x));
        let was = self.pressed.take();
        if was.is_some() {
            self.dirty = true;
        }
        if let (Some(b), Some(h)) = (was, hit) {
            if b == h {
                Self::activate(state, window, b);
            }
        }
    }

    // Window actions run from the event loop's idle callback: the caller
    // holds the window's decoration state borrowed and the actions reach
    // back into it (focus changes, relayout).
    fn activate<BackendData: Backend>(state: &mut AnvilState<BackendData>, window: &WindowElement, button: Button) {
        let window = window.clone();
        match button {
            Button::Close => {
                state.handle.insert_idle(move |_| window.close());
            }
            Button::Minimize => {
                state.handle.insert_idle(move |data| data.minimize_window(&window));
            }
            Button::Maximize => {
                state
                    .handle
                    .insert_idle(move |data| data.toggle_maximize_window(&window));
            }
        }
    }

    fn start_move<BackendData: Backend>(
        seat: &Seat<AnvilState<BackendData>>,
        state: &mut AnvilState<BackendData>,
        window: &WindowElement,
        serial: Serial,
    ) {
        match window.0.underlying_surface() {
            WindowSurface::Wayland(w) => {
                let seat = seat.clone();
                let toplevel = w.clone();
                state
                    .handle
                    .insert_idle(move |data| data.move_request_xdg(&toplevel, &seat, serial));
            }
            #[cfg(feature = "xwayland")]
            WindowSurface::X11(w) => {
                let window = w.clone();
                state.handle.insert_idle(move |data| data.move_request_x11(&window));
            }
        }
    }

    /// Make sure the cached bar matches `width` logical pixels at `scale`.
    pub fn redraw(&mut self, width: i32, scale: i32) {
        if width <= 0 {
            return;
        }
        if self.width != width {
            self.width = width;
            self.dirty = true;
        }
        let stale = self
            .cache
            .as_ref()
            .map(|c| c.width != width || c.scale != scale)
            .unwrap_or(true);
        if !self.dirty && !stale {
            return;
        }
        let canvas = self.draw(scale);
        let buffer = MemoryRenderBuffer::from_slice(
            &canvas.data,
            Fourcc::Argb8888,
            (canvas.width, canvas.height),
            scale,
            Transform::Normal,
            None,
        );
        self.cache = Some(Cached { buffer, width, scale });
        self.dirty = false;
    }

    /// Paint the bar. Also used by the tests, which is why it is pure.
    pub fn draw(&self, s: i32) -> Canvas {
        let s = s.max(1);
        let w = self.width * s;
        let h = HEADER_BAR_HEIGHT * s;
        let r = if self.maximized { 0 } else { RADIUS * s };
        let corners = if self.maximized { 0 } else { TOP_LEFT | TOP_RIGHT };
        let mut c = Canvas::new(w, h);
        // Glass: a translucent dark tint (the compositor blends it over what
        // is behind the window) that the light catches from above: a sheen
        // fading down the bar and a bright hairline along the top edge.
        c.fill_rounded_rect(0, 0, w, h, r, corners, alpha(BG, if self.focused { 0.82 } else { 0.72 }));
        let sheen = if self.focused { 0.07 } else { 0.04 };
        for row in 0..h / 2 {
            let t = 1.0 - row as f32 / (h / 2) as f32;
            let a = sheen * t * t;
            for col in 0..w {
                let cov = Canvas::rounded_coverage(col, row, w, h, r, corners);
                if cov > 0.0 {
                    c.blend(col, row, alpha(WHITE, a * cov));
                }
            }
        }
        let mut edge = Canvas::new(w, r + s);
        edge.stroke_rounded_rect(0, 0, w, 2 * (r + s) + 4 * s, r, corners, alpha(WHITE, if self.focused { 0.14 } else { 0.09 }));
        c.draw_canvas(0, 0, &edge);
        // Bottom line between the bar and the window: the accent, with a
        // soft glow into the bar, when focused; a hairline otherwise.
        if self.focused {
            for (row, a) in [(1, 0.16), (2, 0.08), (3, 0.03)] {
                c.fill_rect(0, h - s - s * row, w, s, alpha(ACCENT, a));
            }
            c.fill_rect(0, h - s, w, s, alpha(ACCENT, 0.85));
        } else {
            c.fill_rect(0, h - s, w, s, alpha(WHITE, 0.07));
        }

        // Title.
        let buttons = self.buttons();
        let buttons_w = buttons.len() as i32 * BUTTON_WIDTH * s;
        let pad = 14 * s;
        let max_w = w - buttons_w - pad - 8 * s;
        if max_w > 20 * s {
            let px = TITLE_PX * s as f32;
            let color = if self.focused { FG } else { FG_DIM };
            with_text(|t| {
                let title = fit_text(t, &self.title, px, Face::LabelBold, max_w);
                let lh = t.line_height(px, Face::LabelBold);
                let y = (h - lh) / 2;
                t.draw(&mut c, pad, y, None, &title, px, color, Face::LabelBold);
            });
        }

        // Buttons.
        let glyph_base = if self.focused { FG_DIM } else { FG_FAINT };
        for (i, button) in buttons.iter().enumerate() {
            let bx = w - (buttons.len() - i) as i32 * BUTTON_WIDTH * s;
            let bw = BUTTON_WIDTH * s;
            let hovered = self.hover == Some(*button);
            let pressed = self.pressed == Some(*button);
            let danger = *button == Button::Close;
            if hovered || pressed {
                let bg = match (danger, pressed) {
                    (true, true) => alpha(DANGER, 0.30),
                    (true, false) => alpha(DANGER, 0.18),
                    (false, true) => alpha(FG, 0.14),
                    (false, false) => alpha(FG, 0.08),
                };
                c.fill_circle(bx + bw / 2, h / 2, 12 * s, bg);
            }
            let glyph = if hovered || pressed {
                if danger {
                    DANGER
                } else {
                    ACCENT
                }
            } else {
                glyph_base
            };
            let cx = bx + bw / 2;
            let cy = h / 2;
            let r = 5 * s;
            match button {
                Button::Minimize => c.fill_rect(cx - r, cy, 2 * r, s, glyph),
                Button::Maximize => {
                    if self.maximized {
                        // Two overlapping squares: restore.
                        stroke_square(&mut c, cx - r + 3 * s, cy - r - s, 2 * r - 2 * s, s, glyph);
                        c.fill_rect(cx - r - s, cy - r + 2 * s, 2 * r - s, 2 * r - s, alpha(BG, 0.9));
                        stroke_square(&mut c, cx - r - s, cy - r + 2 * s, 2 * r - 2 * s, s, glyph);
                    } else {
                        stroke_square(&mut c, cx - r, cy - r, 2 * r, s, glyph);
                    }
                }
                Button::Close => {
                    c.line(cx - r, cy - r, cx + r, cy + r, s, glyph);
                    c.line(cx + r, cy - r, cx - r, cy + r, s, glyph);
                }
            }
        }
        c
    }
}

fn stroke_square(c: &mut Canvas, x: i32, y: i32, size: i32, thickness: i32, color: Rgba) {
    for i in 0..thickness.max(1) {
        c.stroke_rect(x + i, y + i, size - 2 * i, size - 2 * i, color);
    }
}

/// `text` cut with an ellipsis so that it fits `max_w` pixels.
fn fit_text(t: &mut TextRenderer, text: &str, px: f32, face: Face, max_w: i32) -> String {
    let (w, _) = t.measure(text, px, None, face);
    if w <= max_w {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut lo = 0usize;
    let mut hi = chars.len();
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let candidate: String = chars[..mid].iter().collect::<String>() + "…";
        let (cw, _) = t.measure(&candidate, px, None, face);
        if cw <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let cut: String = chars[..lo].iter().collect::<String>().trim_end().to_string();
    if cut.is_empty() {
        "…".to_string()
    } else {
        cut + "…"
    }
}

impl<R> AsRenderElements<R> for HeaderBar
where
    R: Renderer + ImportMem,
    R::TextureId: Clone + Send + 'static,
{
    type RenderElement = MemoryRenderBufferRenderElement<R>;

    fn render_elements<C: From<Self::RenderElement>>(
        &self,
        renderer: &mut R,
        location: Point<i32, Physical>,
        _scale: Scale<f64>,
        alpha: f32,
    ) -> Vec<C> {
        let Some(cached) = self.cache.as_ref() else {
            return Vec::new();
        };
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            location.to_f64(),
            &cached.buffer,
            Some(alpha),
            None,
            None,
            Kind::Unspecified,
        )
        .ok()
        .map(C::from)
        .into_iter()
        .collect()
    }
}

impl WindowElement {
    pub fn decoration_state(&self) -> RefMut<'_, WindowState> {
        self.user_data().insert_if_missing(|| {
            RefCell::new(WindowState {
                is_ssd: false,
                header_bar: HeaderBar::default(),
                frame: Frame::default(),
            })
        });

        self.user_data()
            .get::<RefCell<WindowState>>()
            .unwrap()
            .borrow_mut()
    }

    pub fn set_ssd(&self, ssd: bool) {
        self.decoration_state().is_ssd = ssd;
    }

    /// Does the compositor draw a title bar above this window right now?
    /// (Server-side decorations, and not fullscreen.)
    pub fn has_header(&self) -> bool {
        self.decoration_state().is_ssd && !self.is_fullscreen()
    }

    /// Height of the bar above the client area, 0 when there is none.
    pub fn header_height(&self) -> i32 {
        if self.has_header() {
            HEADER_BAR_HEIGHT
        } else {
            0
        }
    }

    /// The bar height the window will have once its pending state is
    /// applied (what to subtract from a tile or the maximised area when
    /// sizing the client).
    pub fn pending_header_height(&self) -> i32 {
        if let Some(toplevel) = self.0.toplevel() {
            use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
            let (mode, fullscreen) = toplevel.with_pending_state(|state| {
                (
                    state.decoration_mode,
                    state.states.contains(xdg_toplevel::State::Fullscreen),
                )
            });
            let ssd = wants_ssd(mode, &self.app_id());
            return if ssd && !fullscreen { HEADER_BAR_HEIGHT } else { 0 };
        }
        self.header_height()
    }
}

/// Does the compositor draw the title bar for a toplevel? Whatever the
/// client negotiated through xdg-decoration wins. A client that never asks
/// (GTK only ever draws its own) gets no bar, like KWin and Mutter do, except
/// MindOS's own app windows: GTK cannot request server-side decorations, so
/// the shell opens Settings and Files undecorated and the compositor gives
/// them the same bar as every other window.
pub fn wants_ssd(
    mode: Option<smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode>,
    app_id: &str,
) -> bool {
    use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
    match mode {
        Some(Mode::ServerSide) => true,
        Some(Mode::ClientSide) => false,
        _ => app_id.starts_with("mindos-"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoration_rule() {
        use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
        assert!(wants_ssd(Some(Mode::ServerSide), "firefox"));
        assert!(!wants_ssd(Some(Mode::ClientSide), "mindos-settings"));
        assert!(!wants_ssd(None, "org.gnome.Nautilus"));
        assert!(wants_ssd(None, "mindos-files"));
    }

    #[test]
    fn draws_focused_and_unfocused_bars() {
        let mut bar = HeaderBar::default();
        bar.set_title("Terminal — ~/src/mindos");
        bar.set_focused(true);
        bar.redraw(640, 1);
        let c = bar.draw(1);
        assert_eq!((c.width, c.height), (640, HEADER_BAR_HEIGHT));
        // top-left corner is rounded away (transparent); the glass itself is
        // translucent but clearly there
        assert_eq!(c.data[3], 0);
        let mid = ((HEADER_BAR_HEIGHT / 2) * 640 + 320) as usize * 4;
        assert!(c.data[mid + 3] > 200);
        bar.set_focused(false);
        bar.set_maximized(true);
        let c = bar.draw(2);
        assert_eq!((c.width, c.height), (1280, HEADER_BAR_HEIGHT * 2));
        // maximised bars keep square corners
        assert!(c.data[3] > 150);
    }

    #[test]
    fn hit_tests_buttons_from_the_right() {
        let mut bar = HeaderBar::default();
        bar.redraw(400, 1);
        assert_eq!(bar.button_at(399.0), Some(Button::Close));
        assert_eq!(bar.button_at(400.0 - 45.0), Some(Button::Maximize));
        assert_eq!(bar.button_at(400.0 - 85.0), Some(Button::Minimize));
        assert_eq!(bar.button_at(100.0), None);
        assert_eq!(bar.button_at(-1.0), None);
    }

    #[test]
    fn tiles_only_have_a_close_button() {
        let mut bar = HeaderBar::default();
        bar.redraw(400, 1);
        bar.set_tiled(true);
        assert_eq!(bar.button_at(399.0), Some(Button::Close));
        assert_eq!(bar.button_at(400.0 - 45.0), None);
        assert_eq!(bar.button_at(400.0 - 85.0), None);
        let c = bar.draw(1);
        assert_eq!((c.width, c.height), (400, HEADER_BAR_HEIGHT));
        bar.set_tiled(false);
        assert_eq!(bar.button_at(400.0 - 45.0), Some(Button::Maximize));
    }

    #[test]
    fn long_titles_get_an_ellipsis() {
        let long = "x".repeat(400);
        let fitted = with_text(|t| fit_text(t, &long, 14.0, Face::LabelBold, 200));
        assert!(fitted.ends_with('…'));
        assert!(fitted.chars().count() < 60);
        let short = with_text(|t| fit_text(t, "hi", 14.0, Face::LabelBold, 200));
        assert_eq!(short, "hi");
    }
}
