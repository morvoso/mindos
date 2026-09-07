//! Shell IPC: newline-delimited JSON over a Unix socket.
//!
//! This is the contract in `docs/SHELL.md` ("The compositor IPC"). mindwm
//! listens on `$XDG_RUNTIME_DIR/mindwm-<wayland socket>.sock` and exports the
//! path as `MINDWM_SOCKET` to everything it spawns. A client sends one JSON
//! object per line; each request may carry an `id` that the reply echoes:
//!
//! ```text
//! {"id":1,"type":"subscribe"}
//! {"id":1,"ok":true,"result":{}}
//! {"event":"windows","windows":[...],"focused":3}
//! ```
//!
//! [`IpcServer`] owns the socket and the connected clients and knows nothing
//! about the compositor; the calloop wiring and the request handlers live in
//! the `impl AnvilState` block at the bottom of this file, and the compositor
//! calls [`AnvilState::ipc_refresh`] once per event-loop turn to push
//! `windows`/`outputs`/`mindbar` events whenever something changed.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use smithay::{
    reexports::calloop::{generic::Generic, Interest, Mode, PostAction, RegistrationToken},
    utils::IsAlive,
};
use tracing::{debug, info, warn};

use crate::{
    layout::LayoutMode,
    mindbar::BarAction,
    prefs::Prefs,
    state::{AnvilState, Backend},
};

/// A request line longer than this is a bug or an attack; the client is dropped.
pub const MAX_LINE: usize = 64 * 1024;
/// Unsent bytes a stalled client may accumulate before it is dropped.
pub const MAX_PENDING: usize = 4 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

/// One entry of the `windows` event / `get_windows` result.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WindowInfo {
    pub id: u64,
    pub title: String,
    pub app_id: String,
    pub focused: bool,
    pub fullscreen: bool,
    pub maximized: bool,
    pub minimized: bool,
    pub x11: bool,
    /// The window belongs to a Wine (or Proton) process: a Windows program.
    pub wine: bool,
    pub output: Option<String>,
}

/// One entry of the `tray` event: an XEmbed tray icon hosted by the compositor.
#[cfg(feature = "xwayland")]
pub type TrayItemInfo = crate::xtray::TrayItem;
#[cfg(not(feature = "xwayland"))]
pub type TrayItemInfo = serde_json::Value;

/// Payload of the `windows` event.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct WindowsSnapshot {
    pub windows: Vec<WindowInfo>,
    pub focused: Option<u64>,
}

/// One entry of the `outputs` event / `get_outputs` result. Geometry is in
/// logical pixels; `refresh` is in Hz.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OutputInfo {
    pub name: String,
    pub make: String,
    pub model: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub scale: f64,
    pub refresh: f64,
    /// `normal`, `90`, `180`, `270`, `flipped`, `flipped-90`, `flipped-180`, `flipped-270`.
    pub transform: String,
    pub modes: Vec<ModeInfo>,
    pub enabled: bool,
    pub vrr: bool,
    pub vrr_supported: bool,
    /// The output the shell puts its main panels on.
    pub primary: bool,
    /// Physical size in millimetres (0 when unknown).
    pub mm_width: i32,
    pub mm_height: i32,
}

/// A video mode of an output; `refresh` is in millihertz.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ModeInfo {
    pub width: i32,
    pub height: i32,
    pub refresh: i32,
    pub preferred: bool,
    pub current: bool,
}

/// Body of the `set_output` request: every field is optional and only the
/// given ones change.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(default)]
pub struct OutputChange {
    pub mode: Option<ModeRequest>,
    pub scale: Option<f64>,
    pub position: Option<[i32; 2]>,
    pub transform: Option<String>,
    pub enabled: Option<bool>,
    pub vrr: Option<bool>,
    pub primary: Option<bool>,
}

/// `mode` of `set_output`; `refresh` is in millihertz.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct ModeRequest {
    pub width: i32,
    pub height: i32,
    pub refresh: i32,
}

/// `action` of the `mindbar` and `overview` requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelAction {
    Toggle,
    Open,
    Close,
}

/// Every request type from the table in `docs/SHELL.md`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Subscribe,
    GetWindows,
    GetOutputs,
    Focus { window: u64 },
    Close { window: u64 },
    Minimize { window: u64 },
    Unminimize { window: u64 },
    ToggleMinimize { window: u64 },
    ToggleFullscreen { window: u64 },
    ToggleMaximize { window: u64 },
    Mindbar { action: PanelAction },
    Overview { action: PanelAction },
    Launch {
        exec: String,
        #[serde(default)]
        terminal: bool,
    },
    Terminal,
    Quit,
    GetPrefs,
    /// Change any of the keys of the `prefs` event (`layout_mode`,
    /// `mind_show_tools`, `primary_output`).
    SetPrefs { prefs: Value },
    GetLayoutMode,
    SetLayoutMode { mode: String },
    CycleLayoutMode,
    /// Change an output (Displays settings); replies with the new `outputs`.
    SetOutput {
        name: String,
        #[serde(flatten)]
        change: OutputChange,
    },
    /// Replay a click on an XEmbed tray icon (`icon` is the `id` from the `tray` event;
    /// `button` 1 left, 2 middle, 3 right, 4/5 wheel up/down, 6/7 left/right).
    TrayClick { icon: u32, button: u32 },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    Ok(Value),
    Err(String),
}

/// Split a request line into its `id` (echoed back whatever it is) and the
/// parsed request.
pub fn parse_request(line: &str) -> (Option<Value>, Result<Request, String>) {
    let value: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(err) => return (None, Err(format!("invalid JSON: {err}"))),
    };
    let id = value.get("id").filter(|id| !id.is_null()).cloned();
    let request = serde_json::from_value::<Request>(value).map_err(|err| format!("invalid request: {err}"));
    (id, request)
}

/// Serialise a reply line (without the trailing newline).
pub fn reply_line(id: Option<&Value>, reply: &Reply) -> String {
    let mut object = serde_json::Map::new();
    if let Some(id) = id {
        object.insert("id".into(), id.clone());
    }
    match reply {
        Reply::Ok(result) => {
            object.insert("ok".into(), Value::Bool(true));
            object.insert("result".into(), result.clone());
        }
        Reply::Err(error) => {
            object.insert("ok".into(), Value::Bool(false));
            object.insert("error".into(), Value::String(error.clone()));
        }
    }
    Value::Object(object).to_string()
}

pub fn windows_event(snapshot: &WindowsSnapshot) -> String {
    json!({"event": "windows", "windows": snapshot.windows, "focused": snapshot.focused}).to_string()
}

pub fn outputs_event(outputs: &[OutputInfo]) -> String {
    json!({"event": "outputs", "outputs": outputs}).to_string()
}

pub fn tray_event(items: &[TrayItemInfo]) -> String {
    json!({"event": "tray", "items": items}).to_string()
}

pub fn shortcut_event(name: &str) -> String {
    json!({"event": "shortcut", "name": name}).to_string()
}

pub fn mindbar_event(open: bool) -> String {
    json!({"event": "mindbar", "open": open}).to_string()
}

pub fn layout_mode_event(mode: LayoutMode) -> String {
    let modes: Vec<Value> = LayoutMode::ALL
        .iter()
        .map(|m| json!({"name": m.name(), "label": m.label()}))
        .collect();
    json!({"event": "layout_mode", "mode": mode.name(), "label": mode.label(), "modes": modes}).to_string()
}

pub fn prefs_event(prefs: &Prefs) -> String {
    json!({"event": "prefs", "prefs": prefs.to_json()}).to_string()
}

// ---------------------------------------------------------------------------
// Framing
// ---------------------------------------------------------------------------

/// Accumulates bytes and hands out complete lines.
#[derive(Debug, Default)]
pub struct LineBuffer {
    buf: Vec<u8>,
}

impl LineBuffer {
    /// Append `bytes` and return every complete line (without the newline,
    /// blank lines skipped). Fails when a single line exceeds [`MAX_LINE`].
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>, String> {
        self.buf.extend_from_slice(bytes);
        let mut lines = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            let text = String::from_utf8_lossy(&line[..line.len() - 1]);
            let text = text.trim();
            if !text.is_empty() {
                lines.push(text.to_string());
            }
        }
        if self.buf.len() > MAX_LINE {
            return Err(format!("request line longer than {MAX_LINE} bytes"));
        }
        Ok(lines)
    }
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Client {
    stream: UnixStream,
    lines: LineBuffer,
    pending: Vec<u8>,
    subscribed: bool,
    token: Option<RegistrationToken>,
}

impl Client {
    /// Push as much of `pending` as the socket takes; `false` means the
    /// client is dead or hopelessly behind and should be dropped.
    fn flush(&mut self) -> bool {
        while !self.pending.is_empty() {
            match self.stream.write(&self.pending) {
                Ok(0) => return false,
                Ok(n) => {
                    self.pending.drain(..n);
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                    return self.pending.len() <= MAX_PENDING;
                }
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return false,
            }
        }
        true
    }
}

/// The listening socket, its clients and the last snapshots that were sent.
#[derive(Debug, Default)]
pub struct IpcServer {
    path: Option<PathBuf>,
    clients: HashMap<u64, Client>,
    next_client: u64,
    pub last_windows: Option<WindowsSnapshot>,
    pub last_outputs: Option<Vec<OutputInfo>>,
    pub last_mindbar_open: bool,
    pub last_tray: Option<Vec<TrayItemInfo>>,
}

impl IpcServer {
    /// Bind the socket (replacing a stale file from a crashed compositor).
    /// The listener is returned for the event loop; the server remembers the
    /// path so it can be exported and removed on exit.
    pub fn bind(&mut self, path: &Path) -> io::Result<UnixListener> {
        if path.exists() {
            let _ = std::fs::remove_file(path);
        }
        let listener = UnixListener::bind(path)?;
        listener.set_nonblocking(true)?;
        self.path = Some(path.to_path_buf());
        Ok(listener)
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn has_client(&self, id: u64) -> bool {
        self.clients.contains_key(&id)
    }

    pub fn has_subscribers(&self) -> bool {
        self.clients.values().any(|c| c.subscribed)
    }

    pub fn is_subscribed(&self, id: u64) -> bool {
        self.clients.get(&id).map(|c| c.subscribed).unwrap_or(false)
    }

    pub fn subscribe(&mut self, id: u64) {
        if let Some(client) = self.clients.get_mut(&id) {
            client.subscribed = true;
        }
    }

    /// Register a freshly accepted connection and return its id.
    pub fn add_client(&mut self, stream: UnixStream) -> u64 {
        let _ = stream.set_nonblocking(true);
        self.next_client += 1;
        let id = self.next_client;
        self.clients.insert(
            id,
            Client {
                stream,
                lines: LineBuffer::default(),
                pending: Vec::new(),
                subscribed: false,
                token: None,
            },
        );
        id
    }

    pub fn set_token(&mut self, id: u64, token: RegistrationToken) {
        if let Some(client) = self.clients.get_mut(&id) {
            client.token = Some(token);
        }
    }

    /// Forget a client; the caller removes the returned event source.
    pub fn remove_client(&mut self, id: u64) -> Option<RegistrationToken> {
        self.clients.remove(&id).and_then(|c| c.token)
    }

    /// Read everything the client has sent and return the complete lines.
    /// An error means the connection is gone (or misbehaving) and the client
    /// should be dropped.
    pub fn read_client(&mut self, id: u64) -> io::Result<Vec<String>> {
        let client = self
            .clients
            .get_mut(&id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown ipc client"))?;
        let mut chunk = [0u8; 4096];
        let mut lines = Vec::new();
        loop {
            match client.stream.read(&mut chunk) {
                Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "client closed")),
                Ok(n) => {
                    let new = client
                        .lines
                        .push(&chunk[..n])
                        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                    lines.extend(new);
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err),
            }
        }
        // Retry whatever an earlier write could not push out.
        if !client.flush() {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "client stalled"));
        }
        Ok(lines)
    }

    /// Queue a line for one client. `false` means the client should be dropped.
    pub fn send(&mut self, id: u64, line: &str) -> bool {
        let Some(client) = self.clients.get_mut(&id) else {
            return false;
        };
        client.pending.extend_from_slice(line.as_bytes());
        client.pending.push(b'\n');
        client.flush()
    }

    /// Send a line to every subscriber; dead clients are removed and their
    /// event-loop tokens returned so the caller can unregister them.
    pub fn broadcast(&mut self, line: &str) -> Vec<RegistrationToken> {
        let ids: Vec<u64> = self
            .clients
            .iter()
            .filter(|(_, c)| c.subscribed)
            .map(|(id, _)| *id)
            .collect();
        let mut dead = Vec::new();
        for id in ids {
            if !self.send(id, line) {
                dead.extend(self.remove_client(id));
            }
        }
        dead
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            let _ = std::fs::remove_file(path);
        }
    }
}

// ---------------------------------------------------------------------------
// Compositor side
// ---------------------------------------------------------------------------

impl<BackendData: Backend + 'static> AnvilState<BackendData> {
    /// Create the socket next to the Wayland one and start accepting clients.
    pub fn start_ipc(&mut self) {
        let Some(socket) = self.socket_name.clone() else {
            return;
        };
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let path = dir.join(format!("mindwm-{socket}.sock"));
        let listener = match self.ipc.bind(&path) {
            Ok(listener) => listener,
            Err(err) => {
                warn!(path = %path.display(), %err, "shell IPC unavailable");
                return;
            }
        };
        let source = Generic::new(listener, Interest::READ, Mode::Level);
        let result = self.handle.insert_source(source, |_, listener, state| {
            loop {
                match listener.accept() {
                    Ok((stream, _)) => state.ipc_accept(stream),
                    Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                    Err(err) => {
                        warn!(%err, "shell IPC accept failed");
                        break;
                    }
                }
            }
            Ok(PostAction::Continue)
        });
        match result {
            Ok(_) => info!(path = %path.display(), "shell IPC listening"),
            Err(err) => warn!(%err, "failed to add the shell IPC socket to the event loop"),
        }
    }

    fn ipc_accept(&mut self, stream: UnixStream) {
        let reader = match stream.try_clone() {
            Ok(reader) => reader,
            Err(err) => {
                warn!(%err, "shell IPC: cannot clone client socket");
                return;
            }
        };
        let id = self.ipc.add_client(stream);
        let source = Generic::new(reader, Interest::READ, Mode::Level);
        match self.handle.insert_source(source, move |_, _, state| {
            state.ipc_client_ready(id);
            Ok(PostAction::Continue)
        }) {
            Ok(token) => {
                self.ipc.set_token(id, token);
                debug!(client = id, "shell IPC client connected");
            }
            Err(err) => {
                warn!(%err, "shell IPC: cannot watch client socket");
                self.ipc.remove_client(id);
            }
        }
    }

    fn ipc_drop_client(&mut self, id: u64) {
        if let Some(token) = self.ipc.remove_client(id) {
            self.handle.remove(token);
        }
        debug!(client = id, "shell IPC client disconnected");
    }

    fn ipc_client_ready(&mut self, id: u64) {
        let lines = match self.ipc.read_client(id) {
            Ok(lines) => lines,
            Err(err) => {
                if err.kind() != io::ErrorKind::UnexpectedEof {
                    debug!(client = id, %err, "shell IPC read failed");
                }
                self.ipc_drop_client(id);
                return;
            }
        };
        for line in lines {
            let (request_id, request) = parse_request(&line);
            let subscribing = matches!(request, Ok(Request::Subscribe));
            let reply = match request {
                Ok(request) => self.ipc_request(id, request),
                Err(err) => Reply::Err(err),
            };
            if !self.ipc.send(id, &reply_line(request_id.as_ref(), &reply)) {
                self.ipc_drop_client(id);
                return;
            }
            if subscribing && self.ipc.is_subscribed(id) {
                self.ipc_send_snapshots(id);
                if !self.ipc.has_client(id) {
                    return;
                }
            }
        }
    }

    /// The initial `windows`, `outputs` and `mindbar` events after `subscribe`.
    fn ipc_send_snapshots(&mut self, id: u64) {
        let windows = self.windows_snapshot();
        let outputs = self.outputs_snapshot();
        let open = self.mindbar.open;
        let layout_mode = layout_mode_event(self.layout.mode);
        let prefs = prefs_event(&self.prefs);
        let tray = self.tray_snapshot();
        let ok = self.ipc.send(id, &windows_event(&windows))
            && self.ipc.send(id, &outputs_event(&outputs))
            && self.ipc.send(id, &mindbar_event(open))
            && self.ipc.send(id, &layout_mode)
            && self.ipc.send(id, &prefs)
            && self.ipc.send(id, &tray_event(&tray));
        self.ipc.last_windows = Some(windows);
        self.ipc.last_outputs = Some(outputs);
        self.ipc.last_mindbar_open = open;
        self.ipc.last_tray = Some(tray);
        if !ok {
            self.ipc_drop_client(id);
        }
    }

    /// Push a line to every subscriber and unregister the ones that went away.
    pub fn ipc_broadcast(&mut self, line: &str) {
        for token in self.ipc.broadcast(line) {
            self.handle.remove(token);
        }
    }

    /// Emit a `shortcut` event (`launcher`, `overview`). Returns `false` when
    /// nobody is listening so the caller can fall back to a built-in.
    pub fn ipc_shortcut(&mut self, name: &str) -> bool {
        if !self.ipc.has_subscribers() {
            return false;
        }
        self.ipc_broadcast(&shortcut_event(name));
        true
    }

    /// Called once per event-loop turn: prune dead minimised windows and send
    /// `windows`/`outputs`/`mindbar` events when the last snapshot changed.
    pub fn ipc_refresh(&mut self) {
        self.minimized.retain(|m| m.window.alive());
        if !self.ipc.has_subscribers() {
            return;
        }
        let windows = self.windows_snapshot();
        if self.ipc.last_windows.as_ref() != Some(&windows) {
            self.ipc_broadcast(&windows_event(&windows));
            self.ipc.last_windows = Some(windows);
        }
        let outputs = self.outputs_snapshot();
        if self.ipc.last_outputs.as_ref() != Some(&outputs) {
            self.ipc_broadcast(&outputs_event(&outputs));
            self.ipc.last_outputs = Some(outputs);
        }
        let open = self.mindbar.open;
        if self.ipc.last_mindbar_open != open {
            self.ipc.last_mindbar_open = open;
            self.ipc_broadcast(&mindbar_event(open));
        }
        if self.tray_changed() {
            let tray = self.tray_snapshot();
            if self.ipc.last_tray.as_ref() != Some(&tray) {
                self.ipc_broadcast(&tray_event(&tray));
                self.ipc.last_tray = Some(tray);
            }
        }
    }

    /// The XEmbed tray icons the compositor hosts (none without XWayland).
    fn tray_snapshot(&self) -> Vec<TrayItemInfo> {
        #[cfg(feature = "xwayland")]
        {
            self.xtray.as_ref().map(|t| t.snapshot()).unwrap_or_default()
        }
        #[cfg(not(feature = "xwayland"))]
        {
            Vec::new()
        }
    }

    fn tray_changed(&mut self) -> bool {
        #[cfg(feature = "xwayland")]
        {
            self.xtray.as_mut().is_some_and(|t| t.take_dirty())
        }
        #[cfg(not(feature = "xwayland"))]
        {
            false
        }
    }

    fn ipc_request(&mut self, client: u64, request: Request) -> Reply {
        let ok = || Reply::Ok(json!({}));
        match request {
            Request::Subscribe => {
                self.ipc.subscribe(client);
                ok()
            }
            Request::GetWindows => {
                let snapshot = self.windows_snapshot();
                Reply::Ok(json!({"windows": snapshot.windows, "focused": snapshot.focused}))
            }
            Request::GetOutputs => Reply::Ok(json!({"outputs": self.outputs_snapshot()})),
            Request::Focus { window } => self.with_window(window, |state, window| {
                state.unminimize_window(&window);
                state.activate_window(&window);
            }),
            Request::Close { window } => self.with_window(window, |_, window| window.close()),
            Request::Minimize { window } => self.with_window(window, |state, window| state.minimize_window(&window)),
            Request::Unminimize { window } => {
                self.with_window(window, |state, window| state.unminimize_window(&window))
            }
            Request::ToggleMinimize { window } => self.with_window(window, |state, window| {
                if state.is_minimized(&window) {
                    state.unminimize_window(&window);
                } else {
                    state.minimize_window(&window);
                }
            }),
            Request::ToggleFullscreen { window } => self.with_window(window, |state, window| {
                state.unminimize_window(&window);
                state.toggle_fullscreen_window(&window);
            }),
            Request::ToggleMaximize { window } => self.with_window(window, |state, window| {
                state.unminimize_window(&window);
                state.toggle_maximize_window(&window);
            }),
            Request::Mindbar { action } => {
                match action {
                    PanelAction::Toggle => self.mindbar.toggle(),
                    PanelAction::Open => self.mindbar.open(),
                    PanelAction::Close => self.mindbar.close(),
                }
                ok()
            }
            Request::Overview { action } => {
                self.show_window_preview = match action {
                    PanelAction::Toggle => !self.show_window_preview,
                    PanelAction::Open => true,
                    PanelAction::Close => false,
                };
                ok()
            }
            Request::Launch { exec, terminal } => {
                if exec.trim().is_empty() {
                    return Reply::Err("exec is required".into());
                }
                self.handle_bar_action(BarAction::Launch { exec, terminal });
                ok()
            }
            Request::Terminal => {
                self.spawn_terminal();
                ok()
            }
            Request::Quit => {
                info!("shell asked the session to end");
                self.running.store(false, Ordering::SeqCst);
                ok()
            }
            Request::GetPrefs => Reply::Ok(json!({"prefs": self.prefs.to_json()})),
            Request::SetPrefs { prefs } => match self.apply_prefs(prefs) {
                Ok(()) => Reply::Ok(json!({"prefs": self.prefs.to_json()})),
                Err(err) => Reply::Err(err),
            },
            Request::GetLayoutMode => {
                Reply::Ok(json!({"mode": self.layout.mode.name(), "label": self.layout.mode.label()}))
            }
            Request::SetLayoutMode { mode } => match LayoutMode::parse(&mode) {
                Some(mode) => {
                    self.set_layout_mode(mode);
                    ok()
                }
                None => Reply::Err(format!("unknown layout mode: {mode}")),
            },
            Request::CycleLayoutMode => {
                self.cycle_layout_mode();
                ok()
            }
            Request::SetOutput { name, change } => match self.set_output_config(&name, &change) {
                Ok(()) => Reply::Ok(json!({"outputs": self.outputs_snapshot()})),
                Err(err) => Reply::Err(err),
            },
            Request::TrayClick { icon, button } => self.tray_click(icon, button),
        }
    }

    /// A click on a hosted tray icon lands where the pointer is.
    fn tray_click(&mut self, id: u32, button: u32) -> Reply {
        #[cfg(feature = "xwayland")]
        {
            let location = self.pointer.current_location();
            let pointer = (location.x.round() as i32, location.y.round() as i32);
            let Some(tray) = self.xtray.as_mut() else {
                return Reply::Err("no tray host".into());
            };
            let button = u8::try_from(button).unwrap_or(0);
            match tray.click(id, button, pointer) {
                Ok(()) => Reply::Ok(json!({})),
                Err(err) => Reply::Err(err),
            }
        }
        #[cfg(not(feature = "xwayland"))]
        {
            let _ = (id, button);
            Reply::Err("no tray host".into())
        }
    }

    fn with_window(
        &mut self,
        id: u64,
        f: impl FnOnce(&mut Self, crate::shell::WindowElement),
    ) -> Reply {
        match self.window_by_id(id) {
            Some(window) => {
                f(self, window);
                Reply::Ok(json!({}))
            }
            None => Reply::Err(format!("no such window: {id}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_requests_and_echoes_ids() {
        let (id, req) = parse_request(r#"{"id":7,"type":"focus","window":3}"#);
        assert_eq!(id, Some(json!(7)));
        assert_eq!(req, Ok(Request::Focus { window: 3 }));

        let (id, req) = parse_request(r#"{"type":"launch","exec":"steam"}"#);
        assert_eq!(id, None);
        assert_eq!(
            req,
            Ok(Request::Launch {
                exec: "steam".into(),
                terminal: false
            })
        );

        let (id, req) = parse_request(r#"{"id":"a","type":"mindbar","action":"open"}"#);
        assert_eq!(id, Some(json!("a")));
        assert_eq!(
            req,
            Ok(Request::Mindbar {
                action: PanelAction::Open
            })
        );

        let (_, req) = parse_request(r#"{"type":"teleport"}"#);
        assert!(req.unwrap_err().starts_with("invalid request"));
        let (_, req) = parse_request("nope");
        assert!(req.unwrap_err().starts_with("invalid JSON"));
    }

    #[test]
    fn formats_replies() {
        assert_eq!(
            reply_line(Some(&json!(1)), &Reply::Ok(json!({}))),
            r#"{"id":1,"ok":true,"result":{}}"#
        );
        assert_eq!(
            reply_line(None, &Reply::Err("no such window: 9".into())),
            r#"{"error":"no such window: 9","ok":false}"#
        );
        let windows = WindowsSnapshot {
            windows: vec![WindowInfo {
                id: 1,
                title: "Steam".into(),
                app_id: "steam".into(),
                focused: true,
                fullscreen: false,
                maximized: true,
                minimized: false,
                x11: true,
                wine: false,
                output: Some("DP-1".into()),
            }],
            focused: Some(1),
        };
        let line: Value = serde_json::from_str(&windows_event(&windows)).unwrap();
        assert_eq!(line["event"], "windows");
        assert_eq!(line["focused"], 1);
        assert_eq!(line["windows"][0]["app_id"], "steam");
        assert_eq!(line["windows"][0]["output"], "DP-1");
        assert_eq!(line["windows"][0]["wine"], false);
        let line: Value = serde_json::from_str(&shortcut_event("launcher")).unwrap();
        assert_eq!(line["name"], "launcher");
    }

    #[test]
    fn frames_lines() {
        let mut buf = LineBuffer::default();
        assert_eq!(buf.push(b"{\"type\":\"sub").unwrap(), Vec::<String>::new());
        assert_eq!(
            buf.push(b"scribe\"}\n\n{\"type\":\"quit\"}\n{").unwrap(),
            vec![r#"{"type":"subscribe"}"#.to_string(), r#"{"type":"quit"}"#.to_string()]
        );
        assert_eq!(buf.push(b"}\n").unwrap(), vec!["{}".to_string()]);
        let long = vec![b'x'; MAX_LINE + 1];
        assert!(buf.push(&long).is_err());
    }

    #[test]
    fn server_round_trip() {
        let dir = std::env::temp_dir().join(format!("mindwm-ipc-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("s.sock");
        let mut server = IpcServer::default();
        let listener = server.bind(&path).unwrap();
        assert_eq!(server.path(), Some(path.as_path()));

        let mut client = UnixStream::connect(&path).unwrap();
        let (accepted, _) = listener.accept().unwrap();
        let id = server.add_client(accepted);
        assert!(!server.has_subscribers());

        client.write_all(b"{\"id\":1,\"type\":\"subscribe\"}\n").unwrap();
        // The listener is non-blocking; the accepted stream was made non-blocking too.
        let mut lines = Vec::new();
        for _ in 0..50 {
            lines = server.read_client(id).unwrap();
            if !lines.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(lines, vec![r#"{"id":1,"type":"subscribe"}"#.to_string()]);
        server.subscribe(id);
        assert!(server.has_subscribers());
        assert!(server.send(id, "{\"ok\":true}"));
        assert!(server.broadcast("{\"event\":\"mindbar\",\"open\":false}").is_empty());

        let mut got = String::new();
        let mut reader = std::io::BufReader::new(&client);
        std::io::BufRead::read_line(&mut reader, &mut got).unwrap();
        assert_eq!(got, "{\"ok\":true}\n");
        got.clear();
        std::io::BufRead::read_line(&mut reader, &mut got).unwrap();
        assert_eq!(got, "{\"event\":\"mindbar\",\"open\":false}\n");

        drop(client);
        // A closed peer is reported as an error so the caller drops the client.
        let mut closed = false;
        for _ in 0..50 {
            if server.read_client(id).is_err() {
                closed = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(closed);
        assert!(server.remove_client(id).is_none());
        drop(server);
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
