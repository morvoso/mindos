//! Client for the compositor's IPC socket (see docs/SHELL.md, "The compositor
//! IPC"). A background thread owns the connection; events are forwarded to
//! the GTK main loop through an async channel, requests get their reply
//! through a oneshot channel keyed by id.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use crate::HostEvent;

pub type Reply = Result<Value, String>;

#[derive(Debug, Clone)]
pub enum IpcEvent {
    Connected,
    Disconnected,
    /// Any `{"event": ...}` message from the compositor.
    Event(String, Value),
}

struct Shared {
    stream: Option<UnixStream>,
    pending: HashMap<u64, async_channel::Sender<Reply>>,
    next_id: u64,
    connected: bool,
}

#[derive(Clone)]
pub struct IpcClient {
    shared: Arc<Mutex<Shared>>,
    path: PathBuf,
}

pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("MINDWM_SOCKET") {
        return PathBuf::from(p);
    }
    let runtime = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    let display = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".into());
    PathBuf::from(runtime).join(format!("mindwm-{display}.sock"))
}

impl IpcClient {
    pub fn start(events: async_channel::Sender<HostEvent>) -> IpcClient {
        let client = IpcClient {
            shared: Arc::new(Mutex::new(Shared {
                stream: None,
                pending: HashMap::new(),
                next_id: 1,
                connected: false,
            })),
            path: socket_path(),
        };
        let worker = client.clone();
        std::thread::Builder::new()
            .name("mindwm-ipc".into())
            .spawn(move || worker.run(events))
            .expect("spawn ipc thread");
        client
    }

    pub fn is_connected(&self) -> bool {
        self.shared.lock().map(|s| s.connected).unwrap_or(false)
    }

    /// Send a request without waiting for the reply.
    pub fn send(&self, mut request: Value) -> Result<(), String> {
        let mut shared = self.shared.lock().map_err(|_| "ipc state poisoned".to_string())?;
        let id = shared.next_id;
        shared.next_id += 1;
        if let Value::Object(map) = &mut request {
            map.insert("id".into(), json!(id));
        }
        let Some(stream) = shared.stream.as_mut() else {
            return Err("compositor IPC not connected".into());
        };
        let mut line = request.to_string();
        line.push('\n');
        stream.write_all(line.as_bytes()).map_err(|e| format!("ipc write: {e}"))
    }

    /// Send a request and await its reply (with a timeout).
    pub async fn request(&self, mut request: Value) -> Reply {
        let (tx, rx) = async_channel::bounded::<Reply>(1);
        {
            let mut shared = self.shared.lock().map_err(|_| "ipc state poisoned".to_string())?;
            if shared.stream.is_none() {
                return Err("compositor IPC not connected".into());
            }
            let id = shared.next_id;
            shared.next_id += 1;
            if let Value::Object(map) = &mut request {
                map.insert("id".into(), json!(id));
            }
            shared.pending.insert(id, tx);
            let mut line = request.to_string();
            line.push('\n');
            let write = shared.stream.as_mut().unwrap().write_all(line.as_bytes());
            if let Err(e) = write {
                shared.pending.remove(&id);
                return Err(format!("ipc write: {e}"));
            }
        }
        let timeout = gtk4::glib::timeout_future(Duration::from_secs(5));
        futures_select(rx.recv(), timeout).await
    }

    fn run(self, events: async_channel::Sender<HostEvent>) {
        let mut backoff = Duration::from_millis(300);
        loop {
            match UnixStream::connect(&self.path) {
                Ok(stream) => {
                    tracing::info!(path = %self.path.display(), "connected to the compositor");
                    backoff = Duration::from_millis(300);
                    let reader = match stream.try_clone() {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::warn!(%e, "ipc clone failed");
                            std::thread::sleep(backoff);
                            continue;
                        }
                    };
                    {
                        let mut shared = self.shared.lock().unwrap();
                        shared.stream = Some(stream);
                        shared.connected = true;
                    }
                    let _ = self.send(json!({"type": "subscribe"}));
                    let _ = events.send_blocking(HostEvent::Ipc(IpcEvent::Connected));
                    let mut lines = BufReader::new(reader).lines();
                    while let Some(Ok(line)) = lines.next() {
                        let line = line.trim();
                        if line.is_empty() {
                            continue;
                        }
                        let Ok(value) = serde_json::from_str::<Value>(line) else {
                            tracing::warn!(line, "compositor sent unparsable JSON");
                            continue;
                        };
                        if let Some(id) = value.get("id").and_then(Value::as_u64) {
                            let tx = self.shared.lock().ok().and_then(|mut s| s.pending.remove(&id));
                            if let Some(tx) = tx {
                                let reply = if value.get("ok").and_then(Value::as_bool).unwrap_or(false) {
                                    Ok(value.get("result").cloned().unwrap_or(Value::Null))
                                } else {
                                    Err(value
                                        .get("error")
                                        .and_then(Value::as_str)
                                        .unwrap_or("compositor error")
                                        .to_string())
                                };
                                let _ = tx.send_blocking(reply);
                            }
                            continue;
                        }
                        if let Some(event) = value.get("event").and_then(Value::as_str) {
                            let _ = events.send_blocking(HostEvent::Ipc(IpcEvent::Event(event.to_string(), value.clone())));
                        }
                    }
                    tracing::warn!("compositor IPC connection closed");
                    {
                        let mut shared = self.shared.lock().unwrap();
                        shared.stream = None;
                        shared.connected = false;
                        for (_, tx) in shared.pending.drain() {
                            let _ = tx.send_blocking(Err("compositor disconnected".into()));
                        }
                    }
                    let _ = events.send_blocking(HostEvent::Ipc(IpcEvent::Disconnected));
                }
                Err(err) => {
                    tracing::debug!(path = %self.path.display(), %err, "compositor IPC unavailable, retrying");
                }
            }
            std::thread::sleep(backoff);
            backoff = (backoff * 2).min(Duration::from_secs(5));
        }
    }
}

/// Await `recv` unless the timeout fires first.
async fn futures_select(
    recv: async_channel::Recv<'_, Reply>,
    timeout: impl std::future::Future<Output = ()>,
) -> Reply {
    use std::future::Future;
    use std::pin::pin;
    use std::task::Poll;
    let mut recv = pin!(recv);
    let mut timeout = pin!(timeout);
    std::future::poll_fn(move |cx| {
        if let Poll::Ready(r) = recv.as_mut().poll(cx) {
            return Poll::Ready(match r {
                Ok(reply) => reply,
                Err(_) => Err("compositor disconnected".into()),
            });
        }
        if timeout.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err("compositor request timed out".into()));
        }
        Poll::Pending
    })
    .await
}
