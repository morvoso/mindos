//! The Mind bar: MindOS's launcher and conversation overlay (Super+Space).
//!
//! Typing filters installed applications; Enter launches the selection.
//! Anything that is not an application (or a query prefixed with `?`, or
//! Shift+Enter) is sent to Mind, whose streamed answer, tool calls and
//! confirmation prompts are rendered inline. `!cmd` runs a shell command.
//!
//! Everything is drawn on the CPU into a memory buffer, so the bar renders on
//! any backend and costs nothing while it is closed. The look is the MindOS
//! look: a square charcoal card, light hairlines and the shared green accent.

use std::time::Instant;

use serde_json::Value;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::{ImportMem, Renderer};
use smithay::input::keyboard::{Keysym, ModifiersState};
use smithay::utils::{Logical, Point, Size, Transform};

use crate::launcher::{self, AppEntry};
use crate::markdown::{self, Style};
use crate::mind::{Event, MindEvent};
use crate::text::{alpha, hex, Canvas, Face, Rgba, TextRenderer, ALL_CORNERS};

// MindOS design tokens (see docs/SHELL.md); the accent and foreground come
// from the config and default to these.
pub const VOID: Rgba = hex(0x18191b);
pub const BG0: Rgba = hex(0x242527);
pub const BG1: Rgba = hex(0x2d2e30);
pub const HAIRLINE: Rgba = hex(0x484a4e);
pub const LINE_STRONG: Rgba = hex(0x64666a);
pub const FG_DIM: Rgba = hex(0xc1c3c6);
pub const FG_FAINT: Rgba = hex(0xa0a3a7);
pub const MIND: Rgba = hex(0x3ddc97);
pub const WARN: Rgba = hex(0xffb454);
pub const DANGER: Rgba = hex(0xff5d8f);
pub const OK: Rgba = hex(0x3ddc97);

// The startup screen (`backdrop_elements`): the boot splash carried on by the
// compositor until the shell puts the desktop up.
const TITLE: &str = "MINDOS";
const CAPTION: &str = "STARTING THE DESKTOP";
/// How long the screen waits before it also shows the key hints.
const HINTS_AFTER: f32 = 6.0;
const STARTUP_HINTS: [(&str, &str); 5] = [
    ("Super+Space", "ask Mind or launch an app"),
    ("Super+Enter", "terminal"),
    ("Super+Q", "close window"),
    ("Super+F", "fullscreen"),
    ("Super+Tab", "next window"),
];

const PANEL_BG: Rgba = alpha(BG0, 0.92);
const INPUT_BG: Rgba = alpha(VOID, 0.55);
const WHITE: Rgba = hex(0xffffff);
const RADIUS: i32 = 0;
/// Reach of the panel's drop shadow (logical px), around the card.
const SHADOW: i32 = 36;

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

/// The Mind answers in Markdown, so its lines are drawn as styled runs
/// (`**bold**` heavier, `` `code` `` in the mono face, markers gone). Every
/// other kind of line is drawn exactly as it was written.
fn rich_runs(kind: LineKind, text: &str) -> Option<Vec<(String, Face)>> {
    if kind != LineKind::Mind {
        return None;
    }
    Some(
        markdown::spans(text)
            .into_iter()
            .map(|span| {
                let face = match span.style {
                    Style::Normal => Face::Body,
                    Style::Strong => Face::BodyBold,
                    Style::Emphasis => Face::Label,
                    Style::Code => Face::Mono,
                };
                (span.text, face)
            })
            .collect(),
    )
}

/// `rich_runs` output as the borrowed pairs the text renderer takes.
fn runs(owned: &[(String, Face)]) -> Vec<(&str, Face)> {
    owned.iter().map(|(t, f)| (t.as_str(), *f)).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineKind {
    User,
    Mind,
    Thinking,
    /// The animated "Thinking" line while a request is in flight.
    Busy,
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
    pub output: Option<String>,
    swallowed_buttons: std::collections::HashSet<u32>,
    show_tools: bool,
    input: String,
    apps: Vec<AppEntry>,
    results: Vec<usize>,
    selected: usize,
    lines: Vec<Line>,
    streaming: String,
    thinking: String,
    busy: bool,
    /// When the request in flight started (the thinking animation's clock).
    busy_since: Option<Instant>,
    /// The animation frame drawn last (a new one marks the panel dirty).
    phase: u32,
    session: Option<String>,
    pending: Option<(String, String)>,
    connected: bool,
    ready: bool,
    model: String,
    status: String,
    dirty: bool,
    panel: Option<Cached>,
    layout_output_size: Option<Size<i32, Logical>>,
    /// The panel's shadow, drawn once per size (it is the slow part).
    shadow: Option<(Size<i32, Logical>, i32, Canvas)>,
    /// The startup screen's card (wordmark, track, caption), its hints and its
    /// HUD corners, with the card's measurements: width, height, the wordmark's
    /// middle and the track's row, all in canvas pixels.
    wordmark: Option<Cached>,
    card_geometry: (i32, i32, i32, i32),
    hints: Option<Cached>,
    corners: Option<(i32, i32, Vec<MemoryRenderBuffer>)>,
    /// When the startup screen went up (the sweep's clock).
    backdrop_since: Option<Instant>,
    text: TextRenderer,
    osd: crate::osd::Osd,
    foreground: Rgba,
    accent: Rgba,
}

impl MindBar {
    pub fn new(text: TextRenderer, apps: Vec<AppEntry>, foreground: Rgba, accent: Rgba) -> Self {
        tracing::info!(apps = apps.len(), "application index loaded");
        MindBar {
            open: false,
            output: None,
            swallowed_buttons: Default::default(),
            show_tools: false,
            input: String::new(),
            apps,
            results: Vec::new(),
            selected: 0,
            lines: Vec::new(),
            streaming: String::new(),
            thinking: String::new(),
            busy: false,
            busy_since: None,
            phase: 0,
            session: None,
            pending: None,
            connected: false,
            ready: false,
            model: String::new(),
            status: "connecting".into(),
            dirty: true,
            panel: None,
            layout_output_size: None,
            shadow: None,
            wordmark: None,
            card_geometry: (0, 0, 0, 0),
            hints: None,
            corners: None,
            backdrop_since: None,
            text,
            osd: Default::default(),
            foreground,
            accent,
        }
    }

    pub fn apps(&self) -> &[AppEntry] {
        &self.apps
    }

    pub fn set_apps(&mut self, apps: Vec<AppEntry>) {
        self.apps = apps;
        self.refresh_results();
    }

    /// GPU-backed copies must be uploaded again after the display device resumes.
    pub fn invalidate_graphics(&mut self) {
        self.panel = None;
        self.wordmark = None;
        self.hints = None;
        self.corners = None;
        self.osd.clear();
        self.dirty = true;
    }

    pub fn swallow_button(&mut self, button: u32) { self.swallowed_buttons.insert(button); }
    pub fn release_button(&mut self, button: u32) -> bool { self.swallowed_buttons.remove(&button) }

    fn launch_selected(&mut self) -> BarAction {
        let Some(&idx) = self.results.get(self.selected) else { return BarAction::None };
        let app = &self.apps[idx];
        let action = BarAction::Launch { exec: app.launch_command(), terminal: app.terminal };
        self.input.clear();
        self.close();
        action
    }

    pub fn click(&mut self, output: &str, position: Point<f64, Logical>, size: Size<i32, Logical>) -> BarAction {
        if self.output.as_deref() == Some(output) {
            if let Some(panel) = &self.panel {
                let left = (size.w - panel.size.w) / 2;
                let top = (size.h as f64 * 0.10) as i32;
                let x = position.x - left as f64;
                let y = position.y - (top + PAD + HEADER_H + INPUT_H + GAP) as f64;
                if x >= PAD as f64 && x < (panel.size.w - PAD) as f64 && y >= 0.0 {
                    let row = (y / ROW_H as f64) as usize;
                    if row < self.results.len() {
                        self.selected = row;
                        return self.launch_selected();
                    }
                }
                if x >= 0.0 && x < panel.size.w as f64 && position.y >= top as f64
                    && position.y < (top + panel.size.h) as f64 {
                    return BarAction::None;
                }
            }
        }
        self.close();
        BarAction::Close
    }

    pub fn show_osd(&mut self, message: crate::media::Feedback) { self.osd.show(message); }
    pub fn clear_osd(&mut self) { self.osd.clear(); }
    pub fn render_osd<R>(&mut self, renderer: &mut R, output: &str,
        size: Size<i32, Logical>, scale: f64) -> Option<MemoryRenderBufferRenderElement<R>>
    where R: Renderer + ImportMem, R::TextureId: Clone + Send + 'static {
        self.osd.render(renderer, &mut self.text, self.foreground, self.accent, output, size, scale)
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

    /// Open the bar with `text` in the input; with `ask` the question is
    /// sent to the Mind right away (a notice's "Explain" button).
    pub fn open_with(&mut self, text: &str, ask: bool) -> BarAction {
        self.open();
        if self.busy || self.pending.is_some() {
            return BarAction::None;
        }
        let text = text.trim();
        if text.is_empty() {
            return BarAction::None;
        }
        if ask {
            self.input.clear();
            self.results.clear();
            self.push(LineKind::User, text.to_string());
            self.busy = true;
            self.streaming.clear();
            self.thinking.clear();
            self.dirty = true;
            BarAction::Ask(text.to_string())
        } else {
            self.input = format!("?{}", text);
            self.refresh_results();
            BarAction::None
        }
    }

    fn refresh_results(&mut self) {
        self.results = if self.input.starts_with('?') || self.input.starts_with('!') {
            Vec::new()
        } else if self.input.trim().is_empty() && self.lines.is_empty() && !self.busy {
            launcher::quick_launches(&self.apps, RESULT_ROWS)
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
                if !mods.shift && !text.starts_with(['?', '!']) && !self.results.is_empty() {
                    return self.launch_selected();
                }
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
                            exec: app.launch_command(),
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
                self.refresh_results();
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
            let owned = rich_runs(LineKind::Mind, &self.streaming).unwrap_or_default();
            needed += self.text.measure_rich(&runs(&owned), 17.0, text_w).1 + 8;
        } else if self.busy {
            needed += self.text.measure("Thinking", 15.0, text_w, Face::Body).1 + 8;
            if !self.thinking.trim().is_empty() {
                needed += self.text.measure("…", 15.0, text_w, Face::Body).1 * 2 + 8;
            }
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
            needed += match rich_runs(line.kind, &line.text) {
                Some(owned) => self.text.measure_rich(&runs(&owned), 17.0, text_w).1,
                None => self.text.measure(&line.text, 17.0, text_w, Face::Body).1,
            } + 8;
            if needed >= max {
                break;
            }
        }
        needed += 8;
        needed.clamp(64, max)
    }

    /// 0..1, breathing at about one cycle per second while a request runs.
    fn pulse(&self) -> f32 {
        match self.busy_since {
            Some(since) if self.busy => 0.5 + 0.5 * (since.elapsed().as_secs_f32() * 4.5).sin(),
            _ => 1.0,
        }
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

    /// The panel on its drop shadow: a canvas `SHADOW` logical pixels larger
    /// on every side. The shadow only depends on the size, so it is kept.
    fn with_shadow(&mut self, panel: Canvas, size: Size<i32, Logical>, scale: i32) -> Canvas {
        let s = scale.max(1);
        let m = SHADOW * s;
        let stale = !matches!(&self.shadow, Some((sz, sc, _)) if *sz == size && *sc == scale);
        if stale {
            let mut c = Canvas::new(panel.width + 2 * m, panel.height + 2 * m);
            let r = RADIUS * s;
            c.soft_shadow(m, m + 6 * s, panel.width, panel.height, r, m, alpha(VOID, 0.6));
            c.soft_shadow(m, m, panel.width, panel.height, r, (m / 2).max(1), alpha(self.accent, 0.22));
            c.cut_rounded_rect(m, m, panel.width, panel.height, r, ALL_CORNERS);
            self.shadow = Some((size, scale, c));
        }
        let mut c = self.shadow.as_ref().map(|(_, _, c)| c.clone()).unwrap();
        c.draw_canvas(m, m, &panel);
        c
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
        let pulse = self.pulse();
        if self.busy {
            // The dot breathes in the accent while the Mind works.
            canvas.fill_circle(x + 3 * s, hy + 8 * s, 5 * s, alpha(accent, 0.25 * pulse));
            canvas.fill_circle(x + 3 * s, hy + 8 * s, 3 * s, alpha(accent, 0.45 + 0.55 * pulse));
        } else {
            canvas.fill_circle(x + 3 * s, hy + 8 * s, 3 * s, self.status_color());
        }
        x += 12 * s;
        let status = if self.busy { "THINKING".to_string() } else { self.status.to_uppercase() };
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
            "Enter ask Mind   Esc close"
        };
        let (hw, _) = self.text.measure(hint, font(14.0), None, Face::Label);
        self.text
            .draw(&mut canvas, w - pad - hw, hy, None, hint, font(14.0), FG_FAINT, Face::Label);

        // Input row: inset box with an accent bar and a caret.
        let iy = pad + HEADER_H * s;
        let ih = INPUT_H * s;
        canvas.fill_rounded_rect(pad, iy, w - 2 * pad, ih, 0, ALL_CORNERS, INPUT_BG);
        canvas.stroke_rounded_rect(pad, iy, w - 2 * pad, ih, 0, ALL_CORNERS, alpha(WHITE, 0.10));
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
                    canvas.fill_rounded_rect(pad, y, w - 2 * pad, row_h - 2 * s, 0, ALL_CORNERS, alpha(accent, 0.12));
                    canvas.stroke_rounded_rect(pad, y, w - 2 * pad, row_h - 2 * s, 0, ALL_CORNERS, alpha(accent, 0.35));
                    accent
                } else {
                    fg
                };
                let app = &self.apps[idx];
                let name = app.name.clone();
                if let Some(icon) = &app.icon_pixels {
                    canvas.draw_scaled_canvas(pad + 10 * s, y + 4 * s, 26 * s, 26 * s, icon);
                } else {
                    canvas.stroke_rounded_rect(pad + 12 * s, y + 6 * s, 22 * s, 22 * s, 5 * s, ALL_CORNERS, FG_DIM);
                }
                let name_face = if selected { Face::LabelBold } else { Face::Label };
                self.text.draw(
                    &mut canvas,
                    pad + 46 * s,
                    y + 6 * s,
                    Some(w / 2),
                    &name,
                    font(19.0),
                    name_color,
                    name_face,
                );
                {
                    // A Windows program: a small WINDOWS tag after the name,
                    // the same signal as the badge on the dock icon.
                    let tag = if app.wine { "WINDOWS" } else { "LINUX" };
                    let (tw, th) = self.text.measure(tag, font(10.0), None, Face::Mono);
                    let tx = w - pad - tw - 14 * s;
                    let ty = y + 12 * s;
                    canvas.stroke_rounded_rect(tx - 5 * s, ty - 3 * s, tw + 10 * s, th + 5 * s, 3 * s, ALL_CORNERS, alpha(accent, 0.55));
                    self.text.draw(&mut canvas, tx, ty, None, tag, font(10.0), alpha(accent, 0.95), Face::Mono);
                }
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
        } else if self.busy {
            entries.push((LineKind::Busy, "Thinking".into()));
        }
        if let Some((_, desc)) = &self.pending {
            entries.push((LineKind::Info, format!("Mind wants to: {desc}   —   Y allow · N deny")));
        }
        let max_w = w - 2 * pad - 16 * s;
        let px_of = |kind: LineKind| match kind {
            LineKind::Thinking | LineKind::Busy => font(15.0),
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
            let (_, th) = match rich_runs(kind, &text) {
                Some(owned) => self.text.measure_rich(&runs(&owned), px_of(kind), Some(max_w)),
                None => self.text.measure(&text, px_of(kind), Some(max_w), face_of(kind)),
            };
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
                LineKind::Busy => FG_DIM,
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
            if kind == LineKind::Busy {
                // Three dots chasing each other, then the word.
                let pulse = self.pulse();
                let t = self.busy_since.map(|b| b.elapsed().as_secs_f32()).unwrap_or(0.0);
                let cy = y + th / 2;
                for i in 0..3 {
                    let a = 0.5 + 0.5 * (t * 6.0 - i as f32 * 1.1).sin();
                    canvas.fill_circle(text_x + 4 * s + i * 11 * s, cy, 3 * s, alpha(MIND, 0.25 + 0.75 * a));
                }
                let label_x = text_x + 40 * s;
                self.text
                    .draw(&mut canvas, label_x, y, Some(max_w - 40 * s), &text, px_of(kind), alpha(color, 0.6 + 0.4 * pulse), face_of(kind));
                continue;
            }
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
            match rich_runs(kind, &text) {
                Some(owned) => {
                    self.text.draw_rich(
                        &mut canvas,
                        text_x,
                        y,
                        Some(max_w),
                        &runs(&owned),
                        px_of(kind),
                        color,
                        Some(alpha(accent, 0.92)),
                    );
                }
                None => {
                    self.text
                        .draw(&mut canvas, text_x, y, Some(max_w), &text, px_of(kind), color, face_of(kind));
                }
            }
        }
        canvas
    }

    /// The bar's render element for one output, or `None` while closed.
    pub fn render_element<R>(
        &mut self,
        renderer: &mut R,
        output: &str,
        output_size: Size<i32, Logical>,
        scale: f64,
    ) -> Option<MemoryRenderBufferRenderElement<R>>
    where
        R: Renderer + ImportMem,
        R::TextureId: Clone + Send + 'static,
    {
        if !self.open || self.output.as_deref() != Some(output) || output_size.w < 160 || output_size.h < 160 {
            return None;
        }
        // A request in flight animates: a new frame every 60 ms.
        if self.busy {
            let since = *self.busy_since.get_or_insert_with(Instant::now);
            let phase = (since.elapsed().as_millis() / 60) as u32;
            if phase != self.phase {
                self.phase = phase;
                self.dirty = true;
            }
        } else if self.busy_since.take().is_some() {
            self.dirty = true;
        }
        let int_scale = scale.ceil().max(1.0) as i32;
        let width = (output_size.w - 80).min(980);
        let size = if !self.dirty && self.layout_output_size == Some(output_size) && self.panel.as_ref().is_some_and(|c| c.size.w == width && c.scale == int_scale) {
            self.panel.as_ref().unwrap().size
        } else {
            let body = self.body_height(width, output_size.h);
            Size::from((width, panel_height(body).min(output_size.h - 40)))
        };
        self.layout_output_size = Some(output_size);

        let stale = match &self.panel {
            Some(c) => c.size != size || c.scale != int_scale,
            None => true,
        };
        if self.dirty || stale {
            let panel = self.draw_panel(size, int_scale);
            let canvas = self.with_shadow(panel, size, int_scale);
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
            (output_size.w - size.w) / 2 - SHADOW,
            (output_size.h as f64 * 0.10) as i32 - SHADOW,
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

    /// One element for a canvas already uploaded into `buffer`, placed with its
    /// top-left corner at `loc` (logical coordinates on the output).
    fn buffer_element<R>(
        renderer: &mut R,
        buffer: &MemoryRenderBuffer,
        loc: Point<i32, Logical>,
        scale: f64,
    ) -> Option<MemoryRenderBufferRenderElement<R>>
    where
        R: Renderer + ImportMem,
        R::TextureId: Clone + Send + 'static,
    {
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            loc.to_f64().to_physical(scale),
            buffer,
            None,
            None,
            None,
            Kind::Unspecified,
        )
        .ok()
    }

    /// A memory buffer for `canvas` at buffer scale `s`.
    fn buffer_of(canvas: &Canvas, s: i32) -> MemoryRenderBuffer {
        MemoryRenderBuffer::from_slice(
            &canvas.data,
            Fourcc::Argb8888,
            (canvas.width, canvas.height),
            s,
            Transform::Normal,
            None,
        )
    }

    /// The startup card: the wordmark, the progress track and the caption,
    /// laid out in the splash's own units (`c` turns one into canvas pixels).
    fn draw_startup_card(&mut self, c: &dyn Fn(f32) -> i32, title_px: f32, tracking: i32) -> (Canvas, i32, i32) {
        let accent = self.accent;
        let pad = c(20.0);
        let title_w = self.text.measure_spaced(TITLE, title_px, Face::Display, tracking);
        let (_, title_h) = self.text.measure(TITLE, title_px, None, Face::Display);
        let bar_w = c(360.0);
        let bar_h = c(3.0).max(2);
        let cap_px = c(15.0) as f32;
        let cap_track = c(4.0);
        let cap_w = self.text.measure_spaced(CAPTION, cap_px, Face::Mono, cap_track);
        let cap_h = self.text.line_height(cap_px, Face::Mono);

        let width = title_w.max(bar_w).max(cap_w) + 2 * pad;
        let bar_y = pad + title_h + c(34.0);
        let cap_y = bar_y + bar_h + c(24.0);
        let height = cap_y + cap_h + pad;
        let mut canvas = Canvas::new(width, height);
        let cx = width / 2;

        let fg = self.foreground;
        self.text.draw_spaced_glow(
            &mut canvas,
            cx - title_w / 2,
            pad,
            TITLE,
            title_px,
            alpha(fg, 0.96),
            alpha(accent, 0.14),
            c(9.0).max(2),
            Face::Display,
            tracking,
        );
        // The track the sweep runs along: the splash's own #2a3442.
        canvas.fill_rect(cx - bar_w / 2, bar_y, bar_w, bar_h, HAIRLINE);
        canvas.fill_rect(cx - bar_w / 2, bar_y, bar_w, bar_h, alpha(accent, 0.10));
        self.text
            .draw_spaced(&mut canvas, cx - cap_w / 2, cap_y, CAPTION, cap_px, FG_DIM, Face::Mono, cap_track);
        (canvas, pad + title_h / 2, bar_y)
    }

    /// The key hints, shown only when the desktop keeps the screen waiting:
    /// enough to work with the compositor alone if the shell never comes up.
    fn draw_startup_hints(&mut self, c: &dyn Fn(f32) -> i32) -> Canvas {
        let accent = self.accent;
        let key_px = c(14.0) as f32;
        let act_px = c(15.0) as f32;
        let gap = c(12.0);
        let mut keyw = 0;
        let mut actw = 0;
        for (key, action) in STARTUP_HINTS {
            keyw = keyw.max(self.text.measure(key, key_px, None, Face::Mono).0);
            actw = actw.max(self.text.measure(action, act_px, None, Face::Label).0);
        }
        let row = self.text.line_height(act_px, Face::Label) + c(7.0);
        let mut canvas = Canvas::new(keyw + 2 * gap + actw, row * STARTUP_HINTS.len() as i32);
        let mut y = 0;
        for (key, action) in STARTUP_HINTS {
            let kw = self.text.measure(key, key_px, None, Face::Mono).0;
            self.text
                .draw(&mut canvas, keyw - kw, y + c(1.0), None, key, key_px, alpha(accent, 0.55), Face::Mono);
            self.text
                .draw(&mut canvas, keyw + 2 * gap, y, None, action, act_px, alpha(FG_DIM, 0.6), Face::Label);
            y += row;
        }
        canvas
    }

    /// One HUD corner bracket: `dx`/`dy` say which way the two arms point.
    fn draw_corner(c: &dyn Fn(f32) -> i32, accent: Rgba, dx: i32, dy: i32) -> Canvas {
        let side = c(32.0).max(6);
        let t = c(2.0).max(1);
        let mut canvas = Canvas::new(side, side);
        let x0 = if dx > 0 { 0 } else { side - t };
        let y0 = if dy > 0 { 0 } else { side - t };
        canvas.fill_rect(0, y0, side, t, alpha(FG_FAINT, 0.78));
        canvas.fill_rect(x0, 0, t, side, alpha(FG_FAINT, 0.78));
        let n = c(5.0).max(2);
        canvas.fill_rect(
            if dx > 0 { 0 } else { side - n },
            if dy > 0 { 0 } else { side - n },
            n,
            n,
            alpha(accent, 0.9),
        );
        canvas
    }

    /// The startup screen: the boot splash, continued by the compositor.
    ///
    /// Plymouth gives up the display the moment the session starts, and the
    /// shell needs a few seconds more to put its desktop up. What is drawn
    /// here is the picture the splash was already showing -- the wordmark, a
    /// progress line, the hairline sweep and the HUD corners -- so the
    /// handover does not read as a different screen, and the movement says the
    /// machine is still working rather than stuck. The key hints join it only
    /// if the wait runs long (and are what is left if the shell never comes
    /// up). All of it goes the moment the shell maps its desktop (a background
    /// layer surface), not when a window opens.
    pub fn backdrop_elements<R>(
        &mut self,
        renderer: &mut R,
        output_size: Size<i32, Logical>,
        scale: f64,
        desktop_up: bool,
        enabled: bool,
    ) -> Vec<MemoryRenderBufferRenderElement<R>>
    where
        R: Renderer + ImportMem,
        R::TextureId: Clone + Send + 'static,
    {
        if !enabled || desktop_up || output_size.w < 200 || output_size.h < 120 {
            self.backdrop_since = None;
            return Vec::new();
        }
        let elapsed = self.backdrop_since.get_or_insert_with(Instant::now).elapsed().as_secs_f32();
        let s = scale.ceil().max(1.0) as i32;
        // The splash lays everything out in 1080p units and scales from there
        // (mindos.script does the same); `c` turns one unit into a canvas
        // pixel, `u` into a logical pixel on the output.
        let ls = (output_size.h as f32 / 1080.0).clamp(0.6, 2.0);
        let cscale = ls * s as f32;
        let c = move |v: f32| (v * cscale).round() as i32;
        let u = |v: f32| (v * ls).round() as i32;
        let accent = self.accent;
        let mut out = Vec::new();

        // ---- the card: wordmark, track, caption (redrawn only when the output changes)
        let mut title_px = 112.0 * cscale;
        let tracking = |px: f32| (px * 0.18).round() as i32;
        let room = (output_size.w - u(80.0)) * s;
        while title_px > 24.0 * cscale
            && self.text.measure_spaced(TITLE, title_px, Face::Display, tracking(title_px)) > room
        {
            title_px *= 0.85;
        }
        let stale = match &self.wordmark {
            Some(card) => card.scale != s || card.size.w != output_size.w || card.size.h != output_size.h,
            None => true,
        };
        if stale {
            let (canvas, title_mid, bar_y) = self.draw_startup_card(&c, title_px, tracking(title_px));
            self.wordmark = Some(Cached {
                buffer: Self::buffer_of(&canvas, s),
                size: output_size,
                scale: s,
            });
            self.card_geometry = (canvas.width, canvas.height, title_mid, bar_y);
        }
        let (card_w, card_h, title_mid, bar_y) = self.card_geometry;
        // The wordmark sits where the splash had it: 30 units above centre.
        let card_x = (output_size.w - card_w / s) / 2;
        let card_y = output_size.h / 2 - u(30.0) - title_mid / s;
        if let Some(card) = &self.wordmark {
            out.extend(Self::buffer_element(renderer, &card.buffer, (card_x, card_y).into(), scale));
        }

        // ---- the sweep along the track: the splash's progress head, without a
        // number to report, so it runs on a 1.9 s loop while the shell starts.
        let bar_w = c(360.0);
        let strip_h = c(16.0).max(4);
        let mut strip = Canvas::new(bar_w, strip_h);
        let seg = c(120.0).max(8);
        let phase = (elapsed / 1.9).fract();
        let head = (-seg as f32 + (bar_w + seg) as f32 * phase) as i32;
        let mid = strip_h / 2;
        let th = c(3.0).max(2);
        for x in head.max(0)..(head + seg).min(bar_w) {
            let t = (x - head) as f32 / seg as f32;
            let a = (t * t * t).clamp(0.0, 1.0);
            strip.fill_rect(x, mid - th / 2, 1, th, alpha(accent, a));
        }
        let hx = head + seg;
        if hx > 0 && hx < bar_w {
            strip.fill_circle(hx, mid, c(6.0).max(3), alpha(accent, 0.22));
            strip.fill_circle(hx, mid, c(3.0).max(2), alpha(WHITE, 0.9));
        }
        out.extend(Self::buffer_element(
            renderer,
            &Self::buffer_of(&strip, s),
            (card_x + (card_w - bar_w) / 2 / s, card_y + (bar_y - strip_h / 2) / s).into(),
            scale,
        ));

        // ---- the hairline low on the screen with its streak, exactly where
        // the splash drew it (0.84 of the height, 0.56 of the width).
        let line_w = ((output_size.w as f32 * 0.56) as i32 * s).max(2);
        let line_h = c(10.0).max(3);
        let mut scan = Canvas::new(line_w, line_h);
        let ly = line_h / 2;
        scan.fill_rect(0, ly, line_w, c(1.0).max(1), alpha(HAIRLINE, 0.9));
        let streak = c(260.0).max(16);
        let travel = line_w + streak;
        let sx = (-streak as f32 + travel as f32 * (elapsed / 2.6).fract()) as i32;
        for x in sx.max(0)..(sx + streak).min(line_w) {
            let t = ((x - sx) as f32 / streak as f32 * 2.0 - 1.0).abs();
            let a = (1.0 - t).max(0.0).powf(1.3) * 0.9;
            scan.fill_rect(x, ly - c(1.0).max(1), 1, c(3.0).max(2), alpha(accent, a));
        }
        out.extend(Self::buffer_element(
            renderer,
            &Self::buffer_of(&scan, s),
            (
                (output_size.w - line_w / s) / 2,
                (output_size.h as f32 * 0.84) as i32 - line_h / (2 * s),
            )
                .into(),
            scale,
        ));

        // ---- the HUD corners
        let side = c(32.0).max(6);
        if self.corners.as_ref().map(|(sc, sd, _)| *sc != s || *sd != side).unwrap_or(true) {
            let corners = [(1, 1), (-1, 1), (1, -1), (-1, -1)]
                .iter()
                .map(|(dx, dy)| Self::buffer_of(&Self::draw_corner(&c, accent, *dx, *dy), s))
                .collect();
            self.corners = Some((s, side, corners));
        }
        if let Some((_, _, corners)) = &self.corners {
            let inset = u(32.0);
            let far_x = output_size.w - inset - side / s;
            let far_y = output_size.h - inset - side / s;
            for (buffer, loc) in corners.iter().zip([
                (inset, inset),
                (far_x, inset),
                (inset, far_y),
                (far_x, far_y),
            ]) {
                out.extend(Self::buffer_element(renderer, buffer, loc.into(), scale));
            }
        }

        // ---- the key hints, once the wait is long enough to want them
        if elapsed >= HINTS_AFTER {
            let stale = match &self.hints {
                Some(h) => h.scale != s,
                None => true,
            };
            if stale {
                let canvas = self.draw_startup_hints(&c);
                self.hints = Some(Cached {
                    buffer: Self::buffer_of(&canvas, s),
                    size: Size::from((canvas.width, canvas.height)),
                    scale: s,
                });
            }
            if let Some(hints) = &self.hints {
                let loc = (
                    (output_size.w - hints.size.w / s) / 2,
                    card_y + card_h / s + u(26.0),
                );
                out.extend(Self::buffer_element(renderer, &hints.buffer, loc.into(), scale));
            }
        }
        out
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

    #[test]
    fn single_output_cache_clicks_and_wake_recovery() {
        use smithay::backend::renderer::{element::Element, pixman::PixmanRenderer};
        let mut renderer = PixmanRenderer::new().unwrap();
        let mut bar = MindBar::new(TextRenderer::new(), fake_apps(), crate::config::FOREGROUND, crate::config::ACCENT);
        bar.output = Some("DP-1".into());
        bar.open();
        assert!(!bar.results.is_empty(), "quick launches need no typing");
        let started = Instant::now();
        let first = bar.render_element(&mut renderer, "DP-1", (1920, 1080).into(), 2.0).unwrap();
        eprintln!("HiDPI prompt first render: {:?}", started.elapsed());
        for _ in 0..30 {
            assert!(bar.render_element(&mut renderer, "HDMI-1", (1280, 720).into(), 1.0).is_none());
            let cached = bar.render_element(&mut renderer, "DP-1", (1920, 1080).into(), 2.0).unwrap();
            assert_eq!(first.id(), cached.id(), "other displays and idle frames must not rebuild the card");
        }
        bar.invalidate_graphics();
        let restored = bar.render_element(&mut renderer, "DP-1", (1920, 1080).into(), 2.0).unwrap();
        assert_ne!(first.id(), restored.id(), "wake uploads a fresh copy");
        let action = bar.click("DP-1", (510.0, (108 + PAD + HEADER_H + INPUT_H + GAP + 12) as f64).into(), (1920, 1080).into());
        assert!(matches!(action, BarAction::Launch { ref exec, .. } if exec == "steam"));
        assert!(!bar.open);
        bar.swallow_button(0x110);
        assert!(bar.release_button(0x110));
        assert!(!bar.release_button(0x110));
        bar.open();
        assert!(matches!(bar.handle_key(Keysym::Return, "", ModifiersState::default()), BarAction::Launch { .. }));
    }

    fn fake_apps() -> Vec<AppEntry> {
        ["Steam", "Firefox", "Discord", "Lutris", "Kitty", "Files", "Notepad++"]
            .iter()
            .map(|n| AppEntry {
                id: format!("{}.desktop", n.to_lowercase()),
                name: n.to_string(),
                exec: if *n == "Notepad++" { "env WINEPREFIX=/home/u/.wine wine notepad++.exe".into() } else { n.to_lowercase() },
                terminal: false,
                desktop_file: Default::default(),
                icon: String::new(),
                icon_pixels: None,
                wine: *n == "Notepad++",
                haystack: n.to_lowercase(),
            })
            .collect()
    }

    /// Lay a premultiplied BGRA canvas over another one, for the previews.
    fn paste(onto: &mut Canvas, canvas: &Canvas, ox: i32, oy: i32) {
        for y in 0..canvas.height {
            for x in 0..canvas.width {
                let i = ((y * canvas.width + x) * 4) as usize;
                let px = &canvas.data[i..i + 4];
                if px[3] == 0 {
                    continue;
                }
                let a = px[3] as f32 / 255.0;
                // un-premultiply for blend()
                onto.blend(
                    ox + x,
                    oy + y,
                    [px[2] as f32 / 255.0 / a, px[1] as f32 / 255.0 / a, px[0] as f32 / 255.0 / a, a],
                );
            }
        }
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
        bar.input.clear();
        for c in "note".chars() {
            bar.handle_key(Keysym::a, &c.to_string(), ModifiersState::default());
        }
        write_ppm(&dir.join("bar-launcher-wine.ppm"), &bar.draw_panel(size, 1));
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
        // the startup screen, composited onto a full 1920x1080 desktop the way
        // `backdrop_elements` places it (1080p units, so `c` is the identity)
        let c = |v: f32| v.round() as i32;
        let mut desktop = Canvas::new(1920, 1080);
        let (card, title_mid, _bar_y) = bar.draw_startup_card(&c, 112.0, 20);
        paste(&mut desktop, &card, (1920 - card.width) / 2, 1080 / 2 - 30 - title_mid);
        let hints = bar.draw_startup_hints(&c);
        paste(&mut desktop, &hints, (1920 - hints.width) / 2, 1080 - 120 - hints.height);
        write_ppm(&dir.join("desktop.ppm"), &desktop);
    }
}
