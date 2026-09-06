//! Client for mindd, the MindOS LLM daemon, over its Unix socket.
//!
//! The wire format is newline-delimited JSON (see `mindd/src/proto.rs`). A
//! background thread owns the connection and forwards daemon events into the
//! compositor's calloop event loop through a channel; requests are queued on
//! a std mpsc channel and flushed as soon as the daemon is reachable.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};
use smithay::reexports::calloop::channel::Sender as LoopSender;

fn default_kind() -> String {
    "text".into()
}

/// Events the daemon streams back (mirror of `mindd::proto::Event`).
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Welcome {
        #[serde(default)]
        version: String,
        #[serde(default)]
        model: String,
        #[serde(default)]
        ready: bool,
    },
    Delta {
        text: String,
        #[serde(default = "default_kind")]
        kind: String,
    },
    ToolCall {
        id: String,
        name: String,
        #[serde(default)]
        args: Value,
        #[serde(default)]
        policy: String,
        #[serde(default)]
        needs_confirmation: bool,
    },
    ToolResult {
        id: String,
        name: String,
        ok: bool,
        #[serde(default)]
        summary: String,
    },
    ClientTool {
        id: String,
        name: String,
        #[serde(default)]
        args: Value,
    },
    Done {
        #[serde(default)]
        session: String,
        #[serde(default)]
        text: String,
    },
    Status {
        #[serde(default)]
        version: String,
        #[serde(default)]
        model: String,
        #[serde(default)]
        ready: bool,
        #[serde(default)]
        backend: String,
    },
    History {
        #[serde(default)]
        entries: Vec<Value>,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone)]
pub enum MindEvent {
    Connected,
    Disconnected,
    Unavailable(String),
    Event(Event),
}

#[derive(Debug)]
pub struct MindClient {
    tx: mpsc::Sender<String>,
}

impl MindClient {
    pub fn start(socket: PathBuf, events: LoopSender<MindEvent>) -> MindClient {
        let (tx, rx) = mpsc::channel::<String>();
        std::thread::Builder::new()
            .name("mind-client".into())
            .spawn(move || manager(socket, rx, events))
            .expect("spawn mind client thread");
        MindClient { tx }
    }

    fn send(&self, request: Value) {
        let _ = self.tx.send(request.to_string());
    }

    pub fn chat(&self, session: Option<&str>, text: &str, autopilot: bool) {
        self.send(json!({
            "type": "chat",
            "session": session,
            "text": text,
            "autopilot": autopilot,
        }));
    }

    pub fn confirm(&self, id: &str, approve: bool) {
        self.send(json!({"type": "confirm", "id": id, "approve": approve}));
    }

    pub fn tool_result(&self, id: &str, ok: bool, result: Value) {
        self.send(json!({"type": "tool_result", "id": id, "ok": ok, "result": result}));
    }

    pub fn cancel(&self) {
        self.send(json!({"type": "cancel"}));
    }
}

/// Tools the compositor executes on Mind's behalf inside the user's session.
pub fn client_tools() -> Value {
    json!([
        {
            "name": "launch_app",
            "description": "Launch a desktop application in the user's session by name (e.g. \"Steam\", \"Firefox\", \"Discord\").",
            "parameters": {"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]}
        },
        {
            "name": "open_terminal",
            "description": "Open a terminal window for the user.",
            "parameters": {"type": "object", "properties": {}}
        },
        {
            "name": "run_in_terminal",
            "description": "Run a shell command in a new terminal window the user can watch (use for long or interactive commands like game launchers, builds, or dev servers).",
            "parameters": {"type": "object", "properties": {"command": {"type": "string"}}, "required": ["command"]}
        }
    ])
}

fn hello() -> String {
    json!({"type": "hello", "client": "mindwm", "tools": client_tools()}).to_string()
}

fn manager(socket: PathBuf, rx: mpsc::Receiver<String>, events: LoopSender<MindEvent>) {
    let mut reported_unavailable = false;
    loop {
        let stream = match UnixStream::connect(&socket) {
            Ok(s) => s,
            Err(err) => {
                if !reported_unavailable {
                    let _ = events.send(MindEvent::Unavailable(err.to_string()));
                    reported_unavailable = true;
                }
                // Drain nothing: queued requests wait for the daemon.
                std::thread::sleep(Duration::from_secs(3));
                continue;
            }
        };
        reported_unavailable = false;
        let alive = Arc::new(AtomicBool::new(true));
        let reader = match stream.try_clone() {
            Ok(r) => r,
            Err(_) => continue,
        };
        {
            let events = events.clone();
            let alive = alive.clone();
            std::thread::Builder::new()
                .name("mind-reader".into())
                .spawn(move || {
                    let reader = BufReader::new(reader);
                    for line in reader.lines() {
                        let Ok(line) = line else { break };
                        if line.trim().is_empty() {
                            continue;
                        }
                        match serde_json::from_str::<Event>(&line) {
                            Ok(ev) => {
                                if events.send(MindEvent::Event(ev)).is_err() {
                                    break;
                                }
                            }
                            Err(err) => tracing::debug!(%err, line, "ignoring unknown mind event"),
                        }
                    }
                    alive.store(false, Ordering::SeqCst);
                    let _ = events.send(MindEvent::Disconnected);
                })
                .expect("spawn mind reader thread");
        }
        let mut writer = stream;
        if writeln!(writer, "{}", hello()).is_err() {
            continue;
        }
        let _ = events.send(MindEvent::Connected);
        loop {
            match rx.recv_timeout(Duration::from_secs(1)) {
                Ok(line) => {
                    if writeln!(writer, "{line}").and_then(|_| writer.flush()).is_err() {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if !alive.load(Ordering::SeqCst) {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
        let _ = writer.shutdown(std::net::Shutdown::Both);
        std::thread::sleep(Duration::from_secs(2));
    }
}
