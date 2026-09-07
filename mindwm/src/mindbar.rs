//! The Mind bar: MindOS's launcher and conversation overlay (Super+Space).
//!
//! Typing filters installed applications; Enter launches the selection.
//! Anything that is not an application (or a query prefixed with `?`, or
//! Shift+Enter) is sent to Mind, whose streamed answer, tool calls and
//! confirmation prompts are rendered inline. `!cmd` runs a shell command.
//!
//! Everything is drawn on the CPU into a memory buffer, so the bar renders on
//! any backend and costs nothing while it is closed. The look is the MindOS
//! look: a rounded translucent dark card, light hairlines, one cyan accent, no red.

use serde_json::Value;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::{ImportMem, Renderer};
use smithay::input::keyboard::{Keysym, ModifiersState};
use smithay::utils::{Logical, Point, Size, Transform};

use crate::launcher::{self, AppEntry};
use crate::mind::{Event, MindEvent};
use crate::text::{alpha, hex, Canvas, Face, Rgba, TextRenderer, ALL_CORNERS};

// MindOS design tokens (see docs/SHELL.md); the accent and foreground come
// from the config and default to these.
pub const VOID: Rgba = hex(0x05070a);
pub const BG0: Rgba = hex(0x0a0d12);
pub const BG1: Rgba = hex(0x10151c);
pub const HAIRLINE: Rgba = hex(0x223041);
pub const LINE_STRONG: Rgba = hex(0x2f4257);
pub const FG_DIM: Rgba = hex(0x8b9bb0);
pub const FG_FAINT: Rgba = hex(0x55657a);
pub const MIND: Rgba = hex(0xa78bfa);
pub const WARN: Rgba = hex(0xffb454);
pub const DANGER: Rgba = hex(0xff5d8f);
pub const OK: Rgba = hex(0x3ddc97);

const PANEL_BG: Rgba = alpha(BG0, 0.88);
const INPUT_BG: Rgba = alpha(VOID, 0.55);
const WHITE: Rgba = hex(0xffffff);
const RADIUS: i32 = 18;

const PAD: i32 = 16;
const HEADER_H: i32 = 26;
const INPUT_H: i32 = 44;
const GAP: i32 = 10;
const ROW_H: i32 = 36;
const RESULT_ROWS: usize = 8;

/// Height of the whole panel for a body of `body` pixels.
fn panel_height(body: i32) -> i32 {
    PAD + HEADER_H + INPUT_H + GAP + body + PAD
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineKind {
    User,
    Mind,
    Thinking,
    Tool,
    Info,
    Error,
}

#[derive(Debug, Clone)]
struct Line {
    kind: LineKind,
    text: String,
}

#[derive(Debug, Clone)]
pub enum BarAction {
    None,
    Launch { exec: String, terminal: bool },
    Ask(String),
    RunShell(String),
    Confirm { id: String, approve: bool },
    Cancel,
    Close,
}

struct Cached {
    buffer: MemoryRenderBuffer,
    size: Size<i32, Logical>,
    scale: i32,
}

impl std::fmt::Debug for MindBar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MindBar")
            .field("open", &self.open)
            .field("input", &self.input)
            .field("apps", &self.apps.len())
            .field("busy", &self.busy)
            .field("connected", &self.connected)
            .finish()
    }
}

pub struct MindBar {
    pub open: bool,
    show_tools: bool,
    input: String,
    apps: Vec<AppEntry>,
    results: Vec<usize>,
    selected: usize,
    lines: Vec<Line>,
    streaming: String,
    thinking: String,
    busy: bool,
    session: Option<String>,
    pending: Option<(String, String)>,
    connected: bool,
    ready: bool,
    model: String,
    status: String,
    dirty: bool,
    panel: Option<Cached>,
    wordmark: Option<Cached>,
    text: TextRenderer,
    foreground: Rgba,
    accent: Rgba,
}

impl MindBar {
    pub fn new(text: TextRenderer, apps: Vec<AppEntry>, foreground: Rgba, accent: Rgba) -> Self {
        tracing::info!(apps = apps.len(), "application index loaded");
        MindBar {
            open: false,
            show_tools: false,
            input: String::new(),
            apps,
            results: Vec::new(),
            selected: 0,
            lines: Vec::new(),
            streaming: String::new(),
            thinking: String::new(),
            busy: false,
            session: None,
            pending: None,
            connected: false,
            ready: false,
            model: String::new(),
            status: "connecting".into(),
            dirty: true,
            panel: None,
            wordmark: None,
            text,
            foreground,
            accent,
        }
    }

    pub fn apps(&self) -> &[AppEntry] {
        &self.apps
    }

    pub fn session(&self) -> Option<String> {
        self.session.clone()
    }

    /// Mind connection state for the shell: (connected, ready, model).
    pub fn mind_status(&self) -> (bool, bool, String) {
        (self.connected, self.ready, self.model.clone())
    }

    pub fn toggle(&mut self) {
        if self.open {
            self.close();
        } else {
            self.open();
        }
    }

    pub fn open(&mut self) {
        self.open = true;
        self.dirty = true;
        self.refresh_results();
    }

    pub fn close(&mut self) {
        self.open = false;
        self.dirty = true;
    }

    fn refresh_results(&mut self) {
        self.results = if self.input.starts_with('?') || self.input.starts_with('!') {
            Vec::new()
        } else {
            launcher::search(&self.apps, &self.input, RESULT_ROWS)
        };
        self.selected = 0;
        self.dirty = true;
    }

    fn push(&mut self, kind: LineKind, text: impl Into<String>) {
        self.lines.push(Line {
            kind,
            text: text.into(),
        });
        if self.lines.len() > 200 {
            self.lines.drain(..self.lines.len() - 200);
        }
        self.dirty = true;
    }

    fn flush_streaming(&mut self) {
        if !self.streaming.trim().is_empty() {
            let text = std::mem::take(&mut self.streaming);
            self.push(LineKind::Mind, text.trim().to_string());
        } else {
            self.streaming.clear();
        }
        self.thinking.clear();
    }

    /// Handle a key press while the bar is open.
    pub fn handle_key(&mut self, keysym: Keysym, utf8: &str, mods: ModifiersState) -> BarAction {
        self.dirty = true;
        if let Some((id, _)) = self.pending.clone() {
            return match keysym {
                Keysym::y | Keysym::Y | Keysym::Return | Keysym::KP_Enter => {
                    self.pending = None;
                    self.push(LineKind::Info, "allowed");
                    BarAction::Confirm { id, approve: true }
                }
                Keysym::n | Keysym::N | Keysym::Escape => {
                    self.pending = None;
                    self.push(LineKind::Info, "denied");
                    BarAction::Confirm { id, approve: false }
                }
                _ => BarAction::None,
            };
        }
        match keysym {
            Keysym::Escape => {
                if self.busy {
                    self.busy = false;
                    self.flush_streaming();
                    self.push(LineKind::Info, "cancelled");
                    BarAction::Cancel
                } else {
                    self.close();
                    BarAction::Close
                }
            }
            Keysym::Return | Keysym::KP_Enter => {
                let text = self.input.trim().to_string();
                if text.is_empty() {
                    return BarAction::None;
                }
                if let Some(cmd) = text.strip_prefix('!') {
                    self.input.clear();
                    self.refresh_results();
                    self.push(LineKind::Info, format!("$ {}", cmd.trim()));
                    return BarAction::RunShell(cmd.trim().to_string());
                }
                let force_ask = mods.shift || text.starts_with('?');
                if !force_ask {
                    if let Some(&idx) = self.results.get(self.selected) {
                        let app = &self.apps[idx];
                        let action = BarAction::Launch {
                            exec: app.exec.clone(),
                            terminal: app.terminal,
                        };
                        self.input.clear();
                        self.refresh_results();
                        self.close();
                        return action;
                    }
                }
                let question = text.trim_start_matches('?').trim().to_string();
                if question.is_empty() {
                    return BarAction::None;
                }
                self.push(LineKind::User, question.clone());
                self.busy = true;
                self.streaming.clear();
                self.thinking.clear();
                self.input.clear();
                self.results.clear();
                BarAction::Ask(question)
            }
            Keysym::BackSpace => {
                if mods.ctrl {
                    let trimmed = self.input.trim_end().to_string();
                    let cut = trimmed.rfind(' ').map(|i| i + 1).unwrap_or(0);
                    self.input.truncate(cut);
                } else {
                    self.input.pop();
                }
                self.refresh_results();
                BarAction::None
            }
            Keysym::Up | Keysym::ISO_Left_Tab => {
                if !self.results.is_empty() {
                    self.selected = (self.selected + self.results.len() - 1) % self.results.len();
                }
                BarAction::None
            }
            Keysym::Down | Keysym::Tab => {
                if !self.results.is_empty() {
                    self.selected = (self.selected + 1) % self.results.len();
                }
                BarAction::None
            }
            Keysym::u if mods.ctrl => {
                self.input.clear();
                self.refresh_results();
                BarAction::None
            }
            Keysym::l if mods.ctrl => {
                self.lines.clear();
                self.streaming.clear();
                self.thinking.clear();
                BarAction::None
            }
            _ => {
                if !mods.ctrl && !mods.alt && !mods.logo {
                    let s = utf8.trim_end_matches('\0');
                    if !s.is_empty() && !s.chars().any(char::is_control) {
                        self.input.push_str(s);
                        self.refresh_results();
                    }
                }
                BarAction::None
            }
        }
    }

    pub fn on_mind_event(&mut self, event: MindEvent) {
        self.dirty = true;
        match event {
            MindEvent::Connected => {
                self.connected = true;
                self.status = "connected".into();
            }
            MindEvent::Disconnected => {
                self.connected = false;
                self.ready = false;
                self.busy = false;
                self.status = "offline (mindd stopped)".into();
            }
            MindEvent::Unavailable(err) => {
                self.connected = false;
                self.ready = false;
                self.busy = false;
                self.status = format!("offline ({err})");
            }
            MindEvent::Event(ev) => match ev {
                Event::Welcome { model, ready, .. } => {
                    self.model = model;
                    self.ready = ready;
                    self.status = if ready {
                        "ready".into()
                    } else {
                        "model loading…".into()
                    };
                }
                Event::Status { model, ready, .. } => {
                    if !model.is_empty() {
                        self.model = model;
                    }
                    self.ready = ready;
                    self.status = if ready { "ready".into() } else { "model loading…".into() };
                }
                Event::Delta { text, kind } => {
                    if kind == "thinking" {
                        self.thinking.push_str(&text);
                    } else {
                        self.streaming.push_str(&text);
                    }
                }
                Event::ToolCall {
                    id,
                    name,
                    args,
                    needs_confirmation,
                    ..
                } => {
                    let desc = describe_tool(&name, &args);
                    self.flush_streaming();
                    self.push(LineKind::Tool, format!("⚙ {desc}"));
                    if needs_confirmation {
                        self.pending = Some((id, desc));
                    }
                }
                Event::ToolResult { name, ok, summary, .. } => {
                    let first = summary.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
                    let mut first = first.chars().take(160).collect::<String>();
                    if first.is_empty() {
                        first = if ok { "done".into() } else { "failed".into() };
                    }
                    self.push(LineKind::Tool, format!("{} {name}: {first}", if ok { "✓" } else { "✗" }));
                }
                Event::ClientTool { name, .. } => {
                    self.push(LineKind::Tool, format!("⚙ {name} (in session)"));
                }
                Event::Done { session, text } => {
                    if !session.is_empty() {
                        self.session = Some(session);
                    }
                    if self.streaming.trim().is_empty() && !text.trim().is_empty() {
                        self.streaming = text;
                    }
                    self.flush_streaming();
                    self.busy = false;
                }
                Event::Error { message } => {
                    self.flush_streaming();
                    self.push(LineKind::Error, message);
                    self.busy = false;
                }
                Event::History { .. } => {}
            },
        }
    }

    /// Whether tool activity lines (commands Mind runs) are shown.
    pub fn show_tools(&self) -> bool {
        self.show_tools
    }

    pub fn set_show_tools(&mut self, on: bool) {
        if self.show_tools != on {
            self.show_tools = on;
            self.dirty = true;
        }
    }

    fn body_height(&mut self, width: i32, output_h: i32) -> i32 {
        if !self.results.is_empty() {
            return self.results.len() as i32 * ROW_H + 4;
        }
        if self.lines.is_empty() && self.streaming.is_empty() && self.thinking.is_empty() && self.pending.is_none() {
            return 0;
        }
        let max = (output_h as f32 * 0.55) as i32;
        let text_w = Some(width - 2 * PAD - 16);
        let mut needed = 4;
        if !self.streaming.trim().is_empty() {
            needed += self.text.measure(&self.streaming, 17.0, text_w, Face::Body).1 + 8;
        } else if !self.thinking.trim().is_empty() || self.busy {
            needed += self.text.measure("…", 15.0, text_w, Face::Body).1 + 8;
        }
        if let Some((_, desc)) = &self.pending {
            needed += self.text.measure(desc, 16.0, text_w, Face::Body).1 + 22;
        }
        let show_tools = self.show_tools;
        for line in self
            .lines
            .iter()
            .filter(|l| show_tools || l.kind != LineKind::Tool)
            .rev()
            .take(40)
        {
            needed += self.text.measure(&line.text, 17.0, text_w, Face::Body).1 + 8;
            if needed >= max {
                break;
            }
        }
        needed += 8;
        needed.clamp(64, max)
    }

    fn status_color(&self) -> Rgba {
        if !self.connected {
            DANGER
        } else if self.ready {
            OK
        } else {
            WARN
        }
    }

    fn draw_panel(&mut self, size: Size<i32, Logical>, scale: i32) -> Canvas {
        let s = scale.max(1);
        let w = size.w * s;
        let h = size.h * s;
        let pad = PAD * s;
        let r = RADIUS * s;
        let fg = self.foreground;
        let accent = self.accent;
        let font = |px: f32| px * s as f32;
        let mut canvas = Canvas::new(w, h);

        // The card: glass (translucent dark, blended over the desktop by the
        // compositor), a light hairline, an accent glow along the top edge.
        canvas.fill_rounded_rect(0, 0, w, h, r, ALL_CORNERS, PANEL_BG);
        canvas.stroke_rounded_rect(0, 0, w, h, r, ALL_CORNERS, alpha(WHITE, 0.14));
        canvas.hline_glow(r, 0, w - 2 * r, s, 5 * s, alpha(accent, 0.85));

        // Header: ◈ MIND · status dot · status · model            hints
        let hy = pad;
        let mut x = pad;
        self.text
            .draw(&mut canvas, x, hy - 2 * s, None, "◈", font(14.0), accent, Face::Body);
        x += 20 * s;
        x += self
            .text
            .draw_spaced(&mut canvas, x, hy, "MIND", font(14.0), accent, Face::LabelBold, 3 * s);
        x += 16 * s;
        canvas.fill_circle(x + 3 * s, hy + 8 * s, 3 * s, self.status_color());
        x += 12 * s;
        let status = self.status.to_uppercase();
        x += self
            .text
            .draw_spaced(&mut canvas, x, hy, &status, font(13.0), FG_DIM, Face::Label, s);
        if self.connected && !self.model.is_empty() {
            x += 10 * s;
            let model = format!("· {}", self.model);
            self.text
                .draw(&mut canvas, x, hy, Some(w / 2), &model, font(13.0), FG_FAINT, Face::Mono);
        }
        let hint = if self.pending.is_some() {
            "Y allow   N deny"
        } else if self.busy {
            "Esc cancel"
        } else if !self.results.is_empty() {
            "Enter launch   Shift+Enter ask Mind   ↑↓ select   Esc close"
        } else {
            "Enter ask Mind   !cmd run   Esc close"
        };
        let (hw, _) = self.text.measure(hint, font(14.0), None, Face::Label);
        self.text
            .draw(&mut canvas, w - pad - hw, hy, None, hint, font(14.0), FG_FAINT, Face::Label);

        // Input row: inset box with an accent bar and a caret.
        let iy = pad + HEADER_H * s;
        let ih = INPUT_H * s;
        canvas.fill_rounded_rect(pad, iy, w - 2 * pad, ih, 10 * s, ALL_CORNERS, INPUT_BG);
        canvas.stroke_rounded_rect(pad, iy, w - 2 * pad, ih, 10 * s, ALL_CORNERS, alpha(WHITE, 0.10));
        canvas.fill_rounded_rect(pad, iy + 10 * s, 3 * s, ih - 20 * s, s, ALL_CORNERS, accent);
        let prompt_x = pad + 16 * s;
        self.text
            .draw(&mut canvas, prompt_x, iy + 7 * s, None, "›", font(24.0), accent, Face::BodyBold);
        let text_x = prompt_x + 20 * s;
        if self.input.is_empty() {
            self.text.draw(
                &mut canvas,
                text_x,
                iy + 11 * s,
                Some(w - 2 * pad - 48 * s),
                "Type an app, or ask Mind…",
                font(18.0),
                FG_FAINT,
                Face::Body,
            );
            canvas.fill_rect(text_x - 4 * s, iy + 11 * s, 2 * s, 22 * s, accent);
        } else {
            let (tw, _) = self.text.measure(&self.input, font(20.0), None, Face::Body);
            self.text.draw(
                &mut canvas,
                text_x,
                iy + 10 * s,
                None,
                &self.input,
                font(20.0),
                fg,
                Face::Body,
            );
            canvas.fill_rect(text_x + tw + 3 * s, iy + 11 * s, 2 * s, 22 * s, accent);
        }

        let body_y = iy + ih + GAP * s;
        if !self.results.is_empty() {
            let row_h = ROW_H * s;
            for (i, &idx) in self.results.iter().enumerate() {
                let y = body_y + i as i32 * row_h;
                let selected = i == self.selected;
                let name_color = if selected {
                    canvas.fill_rounded_rect(pad, y, w - 2 * pad, row_h - 2 * s, 8 * s, ALL_CORNERS, alpha(accent, 0.12));
                    canvas.stroke_rounded_rect(pad, y, w - 2 * pad, row_h - 2 * s, 8 * s, ALL_CORNERS, alpha(accent, 0.35));
                    accent
                } else {
                    fg
                };
                let app = &self.apps[idx];
                let name = app.name.clone();
                let exec = app.exec.clone();
                self.text.draw(
                    &mut canvas,
                    pad + 16 * s,
                    y + 6 * s,
                    Some(w / 2),
                    &name,
                    font(19.0),
                    name_color,
                    if selected { Face::LabelBold } else { Face::Label },
                );
                let (ew, _) = self.text.measure(&exec, font(12.0), None, Face::Mono);
                let ex = (w - pad - 12 * s - ew).max(w / 2 + pad);
                self.text.draw(
                    &mut canvas,
                    ex,
                    y + 11 * s,
                    Some(w / 2 - 2 * pad),
                    &exec,
                    font(12.0),
                    if selected { alpha(accent, 0.8) } else { FG_DIM },
                    Face::Mono,
                );
            }
            return canvas;
        }

        // Conversation, newest at the bottom.
        let body_bottom = h - pad;
        let mut entries: Vec<(LineKind, String)> = self
            .lines
            .iter()
            .filter(|l| self.show_tools || l.kind != LineKind::Tool)
            .map(|l| (l.kind, l.text.clone()))
            .collect();
        if !self.thinking.trim().is_empty() && self.streaming.trim().is_empty() {
            let t: String = self.thinking.trim().chars().rev().take(240).collect::<Vec<_>>().into_iter().rev().collect();
            entries.push((LineKind::Thinking, format!("thinking… {t}")));
        }
        if !self.streaming.trim().is_empty() {
            entries.push((LineKind::Mind, format!("{}▍", self.streaming.trim_end())));
        } else if self.busy && self.thinking.trim().is_empty() {
            entries.push((LineKind::Thinking, "…".into()));
        }
        if let Some((_, desc)) = &self.pending {
            entries.push((LineKind::Info, format!("Mind wants to: {desc}   —   Y allow · N deny")));
        }
        let max_w = w - 2 * pad - 16 * s;
        let px_of = |kind: LineKind| match kind {
            LineKind::Thinking => font(15.0),
            LineKind::Tool => font(15.0),
            LineKind::Info => font(16.0),
            _ => font(17.0),
        };
        let face_of = |kind: LineKind| match kind {
            LineKind::Tool => Face::Mono,
            _ => Face::Body,
        };
        // Lay out from the bottom up so the newest entries are always visible.
        let mut placed: Vec<(LineKind, String, i32, i32)> = Vec::new();
        let mut y = body_bottom;
        for (kind, text) in entries.into_iter().rev() {
            let (_, th) = self.text.measure(&text, px_of(kind), Some(max_w), face_of(kind));
            let confirm = kind == LineKind::Info && self.pending.is_some() && text.starts_with("Mind wants to");
            let block = th + if confirm { 22 * s } else { 8 * s };
            if y - block < body_y {
                break;
            }
            y -= block;
            placed.push((kind, text, y, th));
        }
        for (kind, text, y, th) in placed.into_iter().rev() {
            let confirm = kind == LineKind::Info && self.pending.is_some() && text.starts_with("Mind wants to");
            let color = match kind {
                LineKind::User => fg,
                LineKind::Mind => fg,
                LineKind::Thinking => FG_FAINT,
                LineKind::Tool => WARN,
                LineKind::Info => {
                    if confirm {
                        WARN
                    } else {
                        FG_DIM
                    }
                }
                LineKind::Error => DANGER,
            };
            let text_x = pad + 14 * s;
            if confirm {
                canvas.fill_rect(pad, y, w - 2 * pad, th + 14 * s, alpha(WARN, 0.10));
                canvas.stroke_rect(pad, y, w - 2 * pad, th + 14 * s, alpha(WARN, 0.55));
                canvas.fill_rect(pad, y, 3 * s, th + 14 * s, WARN);
                self.text
                    .draw(&mut canvas, text_x, y + 7 * s, Some(max_w), &text, px_of(kind), color, face_of(kind));
                continue;
            }
            match kind {
                LineKind::User => canvas.fill_rect(pad, y, 3 * s, th, accent),
                LineKind::Mind => canvas.fill_rect(pad, y, 3 * s, th, MIND),
                LineKind::Error => canvas.fill_rect(pad, y, 3 * s, th, DANGER),
                _ => {}
            }
            self.text
                .draw(&mut canvas, text_x, y, Some(max_w), &text, px_of(kind), color, face_of(kind));
        }
        canvas
    }

    /// The bar's render element for one output, or `None` while closed.
    pub fn render_element<R>(
        &mut self,
        renderer: &mut R,
        output_size: Size<i32, Logical>,
        scale: f64,
    ) -> Option<MemoryRenderBufferRenderElement<R>>
    where
        R: Renderer + ImportMem,
        R::TextureId: Clone + Send + 'static,
    {
        if !self.open || output_size.w <= 0 || output_size.h <= 0 {
            return None;
        }
        let int_scale = scale.ceil().max(1.0) as i32;
        let width = (output_size.w - 80).clamp(320, 980);
        let body = self.body_height(width, output_size.h);
        let height = panel_height(body);
        let size = Size::from((width, height.min(output_size.h - 40)));

        let stale = match &self.panel {
            Some(c) => c.size != size || c.scale != int_scale,
            None => true,
        };
        if self.dirty || stale {
            let canvas = self.draw_panel(size, int_scale);
            let buffer = MemoryRenderBuffer::from_slice(
                &canvas.data,
                Fourcc::Argb8888,
                (canvas.width, canvas.height),
                int_scale,
                Transform::Normal,
                None,
            );
            self.panel = Some(Cached {
                buffer,
                size,
                scale: int_scale,
            });
            self.dirty = false;
        }
        let cached = self.panel.as_ref()?;
        let loc: Point<i32, Logical> = (
            (output_size.w - size.w) / 2,
            (output_size.h as f64 * 0.10) as i32,
        )
            .into();
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            loc.to_f64().to_physical(scale),
            &cached.buffer,
            None,
            None,
            None,
            Kind::Unspecified,
        )
        .ok()
    }

    /// Draw the wordmark and key hints into a fresh canvas of `size` at `s`.
    fn draw_backdrop(&mut self, size: Size<i32, Logical>, s: i32) -> Canvas {
        let mut canvas = Canvas::new(size.w * s, size.h * s);
        let fg = self.foreground;
        let accent = self.accent;
        let font = |px: f32| px * s as f32;
        let cx = canvas.width / 2;

        let title = "MINDOS";
        let (tw, th) = self.text.measure(title, font(84.0), None, Face::Display);
        let tx = cx - tw / 2;
        self.text.draw_glow(
            &mut canvas,
            tx,
            8 * s,
            title,
            font(84.0),
            alpha(fg, 0.96),
            alpha(accent, 0.11),
            6 * s,
            Face::Display,
        );
        let mut y = 8 * s + th + 12 * s;
        canvas.hline_glow(cx - 120 * s, y, 240 * s, 2 * s, 3 * s, accent);
        y += 14 * s;
        let tag = "GAME MODE";
        let tagw = self.text.measure_spaced(tag, font(13.0), Face::LabelBold, 4 * s);
        self.text
            .draw_spaced(&mut canvas, cx - tagw / 2, y, tag, font(13.0), alpha(accent, 0.9), Face::LabelBold, 4 * s);
        y += 40 * s;

        let hints = [
            ("Super+Space", "ask Mind or launch an app"),
            ("Super+Enter", "terminal"),
            ("Super+Q", "close window"),
            ("Super+F", "fullscreen"),
            ("Super+Tab", "next window"),
        ];
        for (key, action) in hints {
            let (kw, _) = self.text.measure(key, font(15.0), None, Face::Mono);
            self.text
                .draw(&mut canvas, cx - 12 * s - kw, y + s, None, key, font(15.0), alpha(accent, 0.85), Face::Mono);
            self.text
                .draw(&mut canvas, cx + 12 * s, y, None, action, font(17.0), FG_DIM, Face::Label);
            y += 27 * s;
        }
        canvas
    }

    /// The "MINDOS" wordmark with key hints, drawn behind windows when the
    /// desktop is empty.
    pub fn backdrop_element<R>(
        &mut self,
        renderer: &mut R,
        output_size: Size<i32, Logical>,
        scale: f64,
        has_windows: bool,
        enabled: bool,
    ) -> Option<MemoryRenderBufferRenderElement<R>>
    where
        R: Renderer + ImportMem,
        R::TextureId: Clone + Send + 'static,
    {
        if !enabled || has_windows || output_size.w < 200 || output_size.h < 120 {
            return None;
        }
        let int_scale = scale.ceil().max(1.0) as i32;
        let size = Size::from((output_size.w.min(1200), 340));
        let stale = match &self.wordmark {
            Some(c) => c.size != size || c.scale != int_scale,
            None => true,
        };
        if stale {
            let canvas = self.draw_backdrop(size, int_scale);
            let buffer = MemoryRenderBuffer::from_slice(
                &canvas.data,
                Fourcc::Argb8888,
                (canvas.width, canvas.height),
                int_scale,
                Transform::Normal,
                None,
            );
            self.wordmark = Some(Cached {
                buffer,
                size,
                scale: int_scale,
            });
        }
        let cached = self.wordmark.as_ref()?;
        let loc: Point<i32, Logical> = ((output_size.w - size.w) / 2, (output_size.h - size.h) / 2).into();
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            loc.to_f64().to_physical(scale),
            &cached.buffer,
            None,
            None,
            None,
            Kind::Unspecified,
        )
        .ok()
    }
}

fn describe_tool(name: &str, args: &Value) -> String {
    let s = |k: &str| args.get(k).and_then(Value::as_str).map(str::to_string);
    let list = |k: &str| {
        args.get(k).and_then(Value::as_array).map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" ")
        })
    };
    match name {
        "run_command" => s("command").map(|c| format!("run: {c}")),
        "install_packages" => list("packages").map(|p| format!("install {p}")),
        "remove_packages" => list("packages").map(|p| format!("remove {p}")),
        "apply_updates" => Some("apply system updates".into()),
        "check_updates" => Some("check for updates".into()),
        "service_control" => match (s("action"), s("unit")) {
            (Some(a), Some(u)) => Some(format!("{a} {u}")),
            _ => None,
        },
        "write_file" => s("path").map(|p| format!("write {p}")),
        "set_kernel_parameter" => s("parameter").map(|p| format!("set kernel parameter {p}")),
        "reboot" => Some("reboot".into()),
        "launch_app" => s("name").map(|n| format!("launch {n}")),
        "run_in_terminal" => s("command").map(|c| format!("terminal: {c}")),
        _ => None,
    }
    .unwrap_or_else(|| {
        let compact = args.to_string();
        let compact: String = compact.chars().take(120).collect();
        format!("{name} {compact}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_apps() -> Vec<AppEntry> {
        ["Steam", "Firefox", "Discord", "Lutris", "Kitty", "Files"]
            .iter()
            .map(|n| AppEntry {
                id: format!("{}.desktop", n.to_lowercase()),
                name: n.to_string(),
                exec: n.to_lowercase(),
                terminal: false,
                haystack: n.to_lowercase(),
            })
            .collect()
    }

    fn write_ppm(path: &std::path::Path, canvas: &Canvas) {
        use std::io::Write;
        let mut out = std::fs::File::create(path).unwrap();
        write!(out, "P6\n{} {}\n255\n", canvas.width, canvas.height).unwrap();
        // composite over the void so the preview shows what the user sees
        let bg = [5u8, 7, 10];
        let mut buf = Vec::with_capacity((canvas.width * canvas.height * 3) as usize);
        for px in canvas.data.chunks_exact(4) {
            let a = px[3] as f32 / 255.0;
            let b = px[0] as f32 + bg[2] as f32 * (1.0 - a);
            let g = px[1] as f32 + bg[1] as f32 * (1.0 - a);
            let r = px[2] as f32 + bg[0] as f32 * (1.0 - a);
            buf.extend_from_slice(&[r.min(255.0) as u8, g.min(255.0) as u8, b.min(255.0) as u8]);
        }
        out.write_all(&buf).unwrap();
    }

    #[test]
    fn renders_preview() {
        let Ok(dir) = std::env::var("MINDWM_PREVIEW_DIR") else {
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        let mut bar = MindBar::new(
            TextRenderer::new(),
            fake_apps(),
            crate::config::FOREGROUND,
            crate::config::ACCENT,
        );
        bar.open();
        bar.on_mind_event(MindEvent::Connected);
        bar.on_mind_event(MindEvent::Event(Event::Welcome {
            version: "0.1".into(),
            model: "qwen2.5-7b-instruct".into(),
            ready: true,
        }));
        // launcher view
        for c in "st".chars() {
            bar.handle_key(Keysym::a, &c.to_string(), ModifiersState::default());
        }
        let size = Size::from((900, panel_height(bar.body_height(900, 1080))));
        write_ppm(&dir.join("bar-launcher.ppm"), &bar.draw_panel(size, 1));
        // conversation view
        bar.input.clear();
        bar.refresh_results();
        bar.push(LineKind::User, "why is my GPU running hot?");
        bar.push(LineKind::Tool, "⚙ run: nvidia-smi");
        bar.push(LineKind::Tool, "✓ run_command: 72C, 98% utilisation, 380W");
        bar.streaming = "Your RTX 4090 is at 72°C under full load, which is normal for this card. Fan curve looks fine; if you want it quieter I can cap the power limit to 320 W.".into();
        bar.pending = Some(("t1".into(), "run: nvidia-smi -pl 320".into()));
        let size = Size::from((900, panel_height(bar.body_height(900, 1080))));
        write_ppm(&dir.join("bar-chat.ppm"), &bar.draw_panel(size, 1));
        // empty conversation (just the input)
        bar.lines.clear();
        bar.streaming.clear();
        bar.pending = None;
        let size = Size::from((900, panel_height(bar.body_height(900, 1080))));
        write_ppm(&dir.join("bar-empty.ppm"), &bar.draw_panel(size, 1));
        // wordmark backdrop, composited onto a full 1920x1080 desktop
        let backdrop = bar.draw_backdrop(Size::from((1200, 340)), 1);
        let mut desktop = Canvas::new(1920, 1080);
        let (ox, oy) = ((1920 - backdrop.width) / 2, (1080 - backdrop.height) / 2);
        for y in 0..backdrop.height {
            for x in 0..backdrop.width {
                let i = ((y * backdrop.width + x) * 4) as usize;
                let px = &backdrop.data[i..i + 4];
                if px[3] == 0 {
                    continue;
                }
                let a = px[3] as f32 / 255.0;
                // un-premultiply for blend()
                desktop.blend(
                    ox + x,
                    oy + y,
                    [px[2] as f32 / 255.0 / a, px[1] as f32 / 255.0 / a, px[0] as f32 / 255.0 / a, a],
                );
            }
        }
        write_ppm(&dir.join("desktop.ppm"), &desktop);
    }
}
