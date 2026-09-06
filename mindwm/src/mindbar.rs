//! The Mind bar: MindOS's launcher and conversation overlay (Super+Space).
//!
//! Typing filters installed applications; Enter launches the selection.
//! Anything that is not an application (or a query prefixed with `?`, or
//! Shift+Enter) is sent to Mind, whose streamed answer, tool calls and
//! confirmation prompts are rendered inline. `!cmd` runs a shell command.
//!
//! Everything is drawn on the CPU into a memory buffer, so the bar renders on
//! any backend and costs nothing while it is closed.

use serde_json::Value;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::{ImportMem, Renderer};
use smithay::input::keyboard::{Keysym, ModifiersState};
use smithay::utils::{Logical, Point, Size, Transform};

use crate::launcher::{self, AppEntry};
use crate::mind::{Event, MindEvent};
use crate::text::{Canvas, Rgba, TextRenderer};

const PANEL_BG: Rgba = [0.07, 0.015, 0.015, 0.94];
const PANEL_BORDER: Rgba = [1.0, 1.0, 1.0, 0.22];
const INPUT_BG: Rgba = [1.0, 1.0, 1.0, 0.08];
const DIM: Rgba = [1.0, 1.0, 1.0, 0.55];
const FAINT: Rgba = [1.0, 1.0, 1.0, 0.35];
const USER: Rgba = [1.0, 0.72, 0.72, 1.0];
const TOOL: Rgba = [1.0, 0.85, 0.5, 1.0];
const ERROR: Rgba = [1.0, 0.45, 0.45, 1.0];
const SELECTED_BG: Rgba = [1.0, 1.0, 1.0, 1.0];
const SELECTED_FG: Rgba = [0.549, 0.0627, 0.0627, 1.0];
const CONFIRM_BG: Rgba = [1.0, 0.85, 0.5, 0.18];

const PAD: i32 = 14;
const RESULT_ROWS: usize = 8;

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
    model: String,
    status: String,
    dirty: bool,
    panel: Option<Cached>,
    wordmark: Option<Cached>,
    text: TextRenderer,
    foreground: Rgba,
}

impl MindBar {
    pub fn new(text: TextRenderer, apps: Vec<AppEntry>, foreground: Rgba) -> Self {
        tracing::info!(apps = apps.len(), "application index loaded");
        MindBar {
            open: false,
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
            model: String::new(),
            status: "connecting to Mind…".into(),
            dirty: true,
            panel: None,
            wordmark: None,
            text,
            foreground,
        }
    }

    pub fn apps(&self) -> &[AppEntry] {
        &self.apps
    }

    pub fn session(&self) -> Option<String> {
        self.session.clone()
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
                self.busy = false;
                self.status = "Mind offline (mindd stopped)".into();
            }
            MindEvent::Unavailable(err) => {
                self.connected = false;
                self.busy = false;
                self.status = format!("Mind offline ({err})");
            }
            MindEvent::Event(ev) => match ev {
                Event::Welcome { model, ready, .. } => {
                    self.model = model;
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

    fn body_height(&mut self, width: i32, output_h: i32) -> i32 {
        if !self.results.is_empty() {
            return self.results.len() as i32 * 36 + PAD;
        }
        if self.lines.is_empty() && self.streaming.is_empty() && self.thinking.is_empty() && self.pending.is_none() {
            return 0;
        }
        let max = (output_h as f32 * 0.55) as i32;
        let text_w = Some(width - 2 * PAD - 12);
        let mut needed = PAD;
        if !self.streaming.trim().is_empty() {
            needed += self.text.measure(&self.streaming, 17.0, text_w, false).1 + 8;
        } else if !self.thinking.trim().is_empty() || self.busy {
            needed += self.text.measure("…", 15.0, text_w, false).1 + 8;
        }
        if let Some((_, desc)) = &self.pending {
            needed += self.text.measure(desc, 17.0, text_w, false).1 + 16;
        }
        for line in self.lines.iter().rev().take(40) {
            needed += self.text.measure(&line.text, 17.0, text_w, false).1 + 8;
            if needed >= max {
                break;
            }
        }
        needed += 12;
        needed.clamp(80, max)
    }

    fn draw_panel(&mut self, size: Size<i32, Logical>, scale: i32) -> Canvas {
        let s = scale.max(1);
        let w = size.w * s;
        let h = size.h * s;
        let pad = PAD * s;
        let mut canvas = Canvas::new(w, h);
        canvas.fill_rounded_rect(0, 0, w, h, 12 * s, PANEL_BG);
        // 1px border
        canvas.fill_rect(0, 0, w, s, PANEL_BORDER);
        canvas.fill_rect(0, h - s, w, s, PANEL_BORDER);
        canvas.fill_rect(0, 0, s, h, PANEL_BORDER);
        canvas.fill_rect(w - s, 0, s, h, PANEL_BORDER);

        let fg = self.foreground;
        let font = |px: f32| px * s as f32;

        // status row
        let status = if self.connected {
            if self.model.is_empty() {
                format!("MIND · {}", self.status)
            } else {
                format!("MIND · {} · {}", self.model, self.status)
            }
        } else {
            format!("MIND · {}", self.status)
        };
        self.text.draw(&mut canvas, pad, pad, Some(w - 2 * pad), &status, font(13.0), DIM, true);
        let hint = if self.pending.is_some() {
            "Y allow · N deny"
        } else if self.busy {
            "Esc cancel"
        } else if !self.results.is_empty() {
            "Enter launch · Shift+Enter ask Mind · ↑↓ select · Esc close"
        } else {
            "Enter ask Mind · !cmd run · Esc close"
        };
        let (hw, _) = self.text.measure(hint, font(13.0), None, false);
        self.text
            .draw(&mut canvas, w - pad - hw, pad, None, hint, font(13.0), FAINT, false);

        // input row
        let input_y = pad + 24 * s;
        let input_h = 44 * s;
        canvas.fill_rounded_rect(pad, input_y, w - 2 * pad, input_h, 8 * s, INPUT_BG);
        let prompt_x = pad + 12 * s;
        let text_y = input_y + 10 * s;
        self.text
            .draw(&mut canvas, prompt_x, text_y, None, "›", font(22.0), fg, true);
        if self.input.is_empty() {
            self.text.draw(
                &mut canvas,
                prompt_x + 22 * s,
                text_y + 2 * s,
                Some(w - 2 * pad - 40 * s),
                "Type an app name, or ask Mind anything…",
                font(19.0),
                FAINT,
                false,
            );
        } else {
            let (tw, _) = self.text.measure(&self.input, font(21.0), None, false);
            self.text.draw(
                &mut canvas,
                prompt_x + 22 * s,
                text_y,
                None,
                &self.input,
                font(21.0),
                fg,
                false,
            );
            canvas.fill_rect(prompt_x + 24 * s + tw, text_y + 2 * s, 2 * s, 24 * s, fg);
        }

        let body_y = input_y + input_h + 8 * s;
        if !self.results.is_empty() {
            let row_h = 36 * s;
            for (i, &idx) in self.results.iter().enumerate() {
                let y = body_y + i as i32 * row_h;
                let selected = i == self.selected;
                let (name_color, exec_color) = if selected {
                    canvas.fill_rounded_rect(pad, y, w - 2 * pad, row_h - 2 * s, 6 * s, SELECTED_BG);
                    (SELECTED_FG, [SELECTED_FG[0], SELECTED_FG[1], SELECTED_FG[2], 0.7])
                } else {
                    (fg, DIM)
                };
                let app = &self.apps[idx];
                let name = app.name.clone();
                let exec = app.exec.clone();
                self.text.draw(
                    &mut canvas,
                    pad + 12 * s,
                    y + 8 * s,
                    Some(w / 2),
                    &name,
                    font(18.0),
                    name_color,
                    selected,
                );
                let (ew, _) = self.text.measure(&exec, font(13.0), None, false);
                let ex = (w - pad - 12 * s - ew).max(w / 2 + pad);
                self.text
                    .draw(&mut canvas, ex, y + 11 * s, Some(w / 2 - 2 * pad), &exec, font(13.0), exec_color, false);
            }
            return canvas;
        }

        // conversation
        let body_bottom = h - pad;
        let mut entries: Vec<(LineKind, String)> = self
            .lines
            .iter()
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
            entries.push((LineKind::Info, format!("Mind wants to: {desc}   —   press Y to allow, N to deny")));
        }
        // Lay out from the bottom up so the newest entries are always visible.
        let max_w = w - 2 * pad - 12 * s;
        let mut placed: Vec<(LineKind, String, i32, i32)> = Vec::new();
        let mut y = body_bottom;
        for (kind, text) in entries.into_iter().rev() {
            let px = if kind == LineKind::Thinking { font(15.0) } else { font(17.0) };
            let (_, th) = self.text.measure(&text, px, Some(max_w), false);
            let block = th + 8 * s;
            if y - block < body_y {
                break;
            }
            y -= block;
            placed.push((kind, text, y, th));
        }
        for (kind, text, y, th) in placed.into_iter().rev() {
            let (color, px, x) = match kind {
                LineKind::User => (USER, font(17.0), pad + 6 * s),
                LineKind::Mind => (fg, font(17.0), pad + 6 * s),
                LineKind::Thinking => (FAINT, font(15.0), pad + 6 * s),
                LineKind::Tool => (TOOL, font(17.0), pad + 6 * s),
                LineKind::Info => (DIM, font(17.0), pad + 6 * s),
                LineKind::Error => (ERROR, font(17.0), pad + 6 * s),
            };
            if kind == LineKind::Info && self.pending.is_some() && text.starts_with("Mind wants to") {
                canvas.fill_rounded_rect(pad, y - 4 * s, w - 2 * pad, th + 8 * s, 6 * s, CONFIRM_BG);
            }
            if kind == LineKind::User {
                canvas.fill_rect(pad, y, 3 * s, th, USER);
            }
            self.text.draw(&mut canvas, x + 6 * s, y, Some(max_w), &text, px, color, false);
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
        let height = PAD + 24 + 44 + 8 + body + PAD;
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

    /// The "MindOS" wordmark with key hints, drawn behind windows when the
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
        let size = Size::from((output_size.w.min(1200), 260));
        let stale = match &self.wordmark {
            Some(c) => c.size != size || c.scale != int_scale,
            None => true,
        };
        if stale {
            let s = int_scale;
            let mut canvas = Canvas::new(size.w * s, size.h * s);
            let fg = self.foreground;
            let title = "MindOS";
            let (tw, _) = self.text.measure(title, 96.0 * s as f32, None, true);
            let x = (canvas.width - tw) / 2;
            self.text
                .draw(&mut canvas, x, 20 * s, None, title, 96.0 * s as f32, [fg[0], fg[1], fg[2], 0.28], true);
            let hints = [
                "Super+Space   ask Mind or launch an app",
                "Super+Enter   terminal        Super+Q   close window",
                "Super+F   fullscreen        Super+Tab   next window",
            ];
            let mut y = 150 * s;
            for h in hints {
                let (hw, hh) = self.text.measure(h, 17.0 * s as f32, None, false);
                let hx = (canvas.width - hw) / 2;
                self.text.draw(
                    &mut canvas,
                    hx,
                    y,
                    None,
                    h,
                    17.0 * s as f32,
                    [fg[0], fg[1], fg[2], 0.45],
                    false,
                );
                y += hh + 6 * s;
            }
            let buffer = MemoryRenderBuffer::from_slice(
                &canvas.data,
                Fourcc::Argb8888,
                (canvas.width, canvas.height),
                s,
                Transform::Normal,
                None,
            );
            self.wordmark = Some(Cached {
                buffer,
                size,
                scale: s,
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
        // composite over MindOS red so the preview shows what the user sees
        let bg = [140u8, 16, 16];
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
        let mut bar = MindBar::new(TextRenderer::new(), fake_apps(), [1.0, 1.0, 1.0, 1.0]);
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
        let size = Size::from((900, PAD + 24 + 44 + 8 + bar.body_height(900, 1080) + PAD));
        write_ppm(&dir.join("bar-launcher.ppm"), &bar.draw_panel(size, 1));
        // conversation view
        bar.input.clear();
        bar.refresh_results();
        bar.push(LineKind::User, "why is my GPU running hot?");
        bar.push(LineKind::Tool, "⚙ run: nvidia-smi");
        bar.push(LineKind::Tool, "✓ run_command: 72C, 98% utilisation, 380W");
        bar.streaming = "Your RTX 4090 is at 72°C under full load, which is normal for this card. Fan curve looks fine; if you want it quieter I can cap the power limit to 320 W.".into();
        bar.pending = Some(("t1".into(), "run: nvidia-smi -pl 320".into()));
        let size = Size::from((900, PAD + 24 + 44 + 8 + bar.body_height(900, 1080) + PAD));
        write_ppm(&dir.join("bar-chat.ppm"), &bar.draw_panel(size, 1));
        // wordmark
        let mut canvas = Canvas::new(1200, 260);
        let title = "MindOS";
        let (tw, _) = bar.text.measure(title, 96.0, None, true);
        let x = (canvas.width - tw) / 2;
        bar.text.draw(&mut canvas, x, 20, None, title, 96.0, [1.0, 1.0, 1.0, 0.28], true);
        let (hw, _) = bar.text.measure("Super+Space   ask Mind or launch an app", 17.0, None, false);
        let hx = (canvas.width - hw) / 2;
        bar.text.draw(&mut canvas, hx, 150, None, "Super+Space   ask Mind or launch an app", 17.0, [1.0, 1.0, 1.0, 0.45], false);
        write_ppm(&dir.join("wordmark.ppm"), &canvas);
    }
}
