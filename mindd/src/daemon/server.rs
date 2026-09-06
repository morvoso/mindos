//! Unix socket server: one task per client connection.

use super::agent::{self, Session};
use super::Daemon;
use crate::proto::{ClientTool, Event, Request};
use anyhow::Result;
use serde_json::Value;
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

pub struct Conn {
    pub tx: mpsc::UnboundedSender<Event>,
    pub rx: mpsc::UnboundedReceiver<Request>,
    pub client: String,
    pub client_tools: Vec<ClientTool>,
    pub uid: u32,
}

impl Conn {
    pub fn send(&self, ev: Event) {
        let _ = self.tx.send(ev);
    }

    /// Wait for a Confirm for `id`. Any other request arriving meanwhile is
    /// handled minimally (Cancel → false).
    pub async fn wait_confirm(&mut self, id: &str, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            match tokio::time::timeout_at(deadline, self.rx.recv()).await {
                Ok(Some(Request::Confirm { id: cid, approve })) if cid == id => return approve,
                Ok(Some(Request::Cancel)) | Ok(None) | Err(_) => return false,
                Ok(Some(_)) => continue,
            }
        }
    }

    pub async fn wait_tool_result(&mut self, id: &str, timeout: Duration) -> Option<(bool, Value)> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            match tokio::time::timeout_at(deadline, self.rx.recv()).await {
                Ok(Some(Request::ToolResult { id: rid, ok, result })) if rid == id => return Some((ok, result)),
                Ok(Some(Request::Cancel)) | Ok(None) | Err(_) => return None,
                Ok(Some(_)) => continue,
            }
        }
    }
}

pub async fn serve(d: Arc<Daemon>) -> Result<()> {
    let path = d.config.daemon.socket.clone();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    // root + the mindos group may talk to the daemon
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o660))?;
    if let Some(gid) = group_id(&d.config.daemon.group) {
        let c = std::ffi::CString::new(path.to_string_lossy().as_bytes()).unwrap();
        unsafe {
            libc::chown(c.as_ptr(), 0u32.wrapping_sub(1), gid);
        }
    }
    eprintln!("mindd: listening on {}", path.display());
    loop {
        let (stream, _) = listener.accept().await?;
        let d = d.clone();
        tokio::spawn(async move {
            if let Err(e) = handle(d, stream).await {
                eprintln!("mindd: connection error: {:#}", e);
            }
        });
    }
}

fn group_id(name: &str) -> Option<u32> {
    let s = std::fs::read_to_string("/etc/group").ok()?;
    for line in s.lines() {
        let mut it = line.split(':');
        if it.next() == Some(name) {
            it.next();
            return it.next()?.parse().ok();
        }
    }
    None
}

async fn handle(d: Arc<Daemon>, stream: UnixStream) -> Result<()> {
    let uid = stream.peer_cred().map(|c| c.uid()).unwrap_or(u32::MAX);
    let (r, mut w) = stream.into_split();
    let (ev_tx, mut ev_rx) = mpsc::unbounded_channel::<Event>();
    let (req_tx, req_rx) = mpsc::unbounded_channel::<Request>();

    // writer task
    let writer = tokio::spawn(async move {
        while let Some(ev) = ev_rx.recv().await {
            let mut s = match serde_json::to_string(&ev) {
                Ok(s) => s,
                Err(_) => continue,
            };
            s.push('\n');
            if w.write_all(s.as_bytes()).await.is_err() {
                break;
            }
        }
    });
    // reader task
    let reader = tokio::spawn(async move {
        let mut lines = BufReader::new(r).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let t = line.trim();
            if t.is_empty() {
                continue;
            }
            match serde_json::from_str::<Request>(t) {
                Ok(req) => {
                    if req_tx.send(req).is_err() {
                        break;
                    }
                }
                Err(e) => eprintln!("mindd: bad request from client: {} ({})", e, t.chars().take(200).collect::<String>()),
            }
        }
    });

    let mut conn = Conn { tx: ev_tx, rx: req_rx, client: "unknown".into(), client_tools: vec![], uid };
    conn.send(Event::Welcome { version: crate::VERSION.into(), model: d.model_name(), ready: d.is_ready() });

    let mut current_session: Option<String> = None;
    while let Some(req) = conn.rx.recv().await {
        match req {
            Request::Hello { client, tools } => {
                conn.client = client;
                conn.client_tools = tools;
            }
            Request::Status => {
                let backend = if d.config.model.external_url.is_some() { "external".into() } else { "llama-server".into() };
                conn.send(Event::Status {
                    version: crate::VERSION.into(),
                    model: d.model_name(),
                    ready: d.is_ready(),
                    sessions: d.sessions.lock().await.len(),
                    uptime_secs: d.started.elapsed().as_secs(),
                    backend,
                });
            }
            Request::History { limit } => {
                conn.send(Event::History { entries: d.audit.tail(limit.min(1000)) });
            }
            Request::Cancel | Request::Confirm { .. } | Request::ToolResult { .. } => {}
            Request::Chat { session, text, autopilot } => {
                if !d.is_ready() {
                    conn.send(Event::Error { message: "the model is still loading, try again in a moment".into() });
                    continue;
                }
                let sid = session.or_else(|| current_session.clone()).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                current_session = Some(sid.clone());
                let mut sess = {
                    let mut all = d.sessions.lock().await;
                    all.remove(&sid).unwrap_or_else(|| Session { id: sid.clone(), messages: vec![crate::daemon::llm::Message::system(d.system_prompt.clone())], created: std::time::Instant::now() })
                };
                let res = agent::run_chat(&d, &mut conn, &mut sess, text, autopilot).await;
                // keep the context bounded: drop the oldest turns beyond a budget
                trim_session(&mut sess, 60);
                d.sessions.lock().await.insert(sid.clone(), sess);
                match res {
                    Ok(text) => conn.send(Event::Done { session: sid, text }),
                    Err(e) => conn.send(Event::Error { message: format!("{:#}", e) }),
                }
                // expire old sessions
                let mut all = d.sessions.lock().await;
                all.retain(|_, s| s.created.elapsed() < Duration::from_secs(6 * 3600));
            }
        }
    }
    drop(conn);
    reader.abort();
    let _ = writer.await;
    Ok(())
}

fn trim_session(s: &mut Session, max_messages: usize) {
    if s.messages.len() <= max_messages {
        return;
    }
    let system = s.messages[0].clone();
    let keep = s.messages.len() - max_messages;
    let mut rest: Vec<_> = s.messages.drain(1..).collect();
    rest.drain(..keep);
    // never start with a tool result whose call was dropped
    while rest.first().map(|m| m.role == "tool").unwrap_or(false) {
        rest.remove(0);
    }
    s.messages = vec![system];
    s.messages.extend(rest);
}
