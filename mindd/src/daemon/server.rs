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
                    sleeping: d.is_sleeping(),
                    notices: d.notices.count(),
                });
            }
            Request::Subscribe => {
                d.notices.subscribe(conn.tx.clone());
                conn.send(Event::Notices { notices: d.notices.list() });
                conn.send(Event::Updates(d.updates.lock().unwrap().clone()));
                let (checked_at, findings) = d.last_health.lock().unwrap().clone();
                if checked_at > 0 {
                    conn.send(Event::Health { checked_at, findings });
                }
                conn.send(Event::Sleep { sleeping: d.is_sleeping() });
            }
            Request::Notices => conn.send(Event::Notices { notices: d.notices.list() }),
            Request::DismissNotice { id } => {
                let gone = d.notices.dismiss(&id);
                d.audit.record("notice_dismissed", "", conn.uid, serde_json::json!({"ids": gone}));
                conn.send(Event::Notices { notices: d.notices.list() });
            }
            Request::Updates { check } => {
                if check {
                    let d2 = d.clone();
                    let tx = conn.tx.clone();
                    tokio::spawn(async move {
                        let s = super::updates::check(&d2, true).await;
                        let _ = tx.send(Event::Updates(s));
                    });
                    // the caller sees progress (checking/assessing) through Updates pushes
                    conn.send(Event::Updates(d.updates.lock().unwrap().clone()));
                } else {
                    conn.send(Event::Updates(d.updates.lock().unwrap().clone()));
                }
            }
            Request::ApplyUpdates => {
                let d2 = d.clone();
                let tx = conn.tx.clone();
                let uid = conn.uid;
                d.audit.record("update_requested", "", uid, serde_json::json!({"client": conn.client}));
                tokio::spawn(async move {
                    match super::updates::apply(&d2, uid).await {
                        Ok(s) => { let _ = tx.send(Event::Updates(s)); }
                        Err(e) => { let _ = tx.send(Event::Error { message: format!("{:#}", e) }); }
                    }
                });
            }
            Request::SetAutoUpdate { enabled } => match d.set_auto_update(enabled) {
                Ok(()) => {
                    d.audit.record("auto_update", "", conn.uid, serde_json::json!({"enabled": enabled}));
                    conn.send(Event::Updates(d.updates.lock().unwrap().clone()));
                }
                Err(e) => conn.send(Event::Error { message: format!("{:#}", e) }),
            },
            Request::Health => {
                let findings = super::health::run_and_notify(&d).await;
                conn.send(Event::Health { checked_at: super::notices::now(), findings });
            }
            Request::SetSleep { sleeping } => {
                d.audit.record("sleep", "", conn.uid, serde_json::json!({"sleeping": sleeping, "client": conn.client}));
                d.set_sleep(sleeping);
                conn.send(Event::Sleep { sleeping: d.is_sleeping() });
            }
            Request::Power { action } => {
                let verb = match action.as_str() {
                    "reboot" => "reboot",
                    "poweroff" | "shutdown" => "poweroff",
                    _ => {
                        conn.send(Event::Error { message: format!("unknown power action {}", action) });
                        continue;
                    }
                };
                d.audit.record("power", "", conn.uid, serde_json::json!({"action": verb, "client": conn.client}));
                let verb = verb.to_string();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    let _ = tokio::process::Command::new("systemctl").arg(&verb).status().await;
                });
                conn.send(Event::Notices { notices: d.notices.list() });
            }
            Request::Rollback { snapshot } => {
                d.audit.record("rollback", "", conn.uid, serde_json::json!({"snapshot": snapshot, "client": conn.client}));
                let out = tokio::process::Command::new("mindos-boot").arg("restore").arg(snapshot.to_string()).output().await;
                match out {
                    Ok(o) if o.status.success() => {
                        d.notices.dismiss("updates:problems");
                        d.notices.post(crate::proto::Notice { id: "updates:rolled-back".into(), level: "ok".into(), title: format!("Rolled back to snapshot {}", snapshot), body: "Rebooting into the restored system.".into(), source: "updates".into(), time: 0, actions: vec![] });
                        conn.send(Event::Notices { notices: d.notices.list() });
                        tokio::spawn(async move {
                            tokio::time::sleep(Duration::from_secs(3)).await;
                            let _ = tokio::process::Command::new("systemctl").arg("reboot").status().await;
                        });
                    }
                    Ok(o) => conn.send(Event::Error { message: format!("mindos-boot restore failed: {}", String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("").to_string()) }),
                    Err(e) => conn.send(Event::Error { message: format!("mindos-boot: {}", e) }),
                }
            }
            Request::History { limit } => {
                conn.send(Event::History { entries: d.audit.tail(limit.min(1000)) });
            }
            Request::Cancel | Request::Confirm { .. } | Request::ToolResult { .. } => {}
            Request::Models => conn.send(d.models_event()),
            Request::SetModel { path } => match d.set_model(&path) {
                Ok(()) => {
                    d.audit.record("model", "", conn.uid, serde_json::json!({"path": path, "client": conn.client}));
                    conn.send(d.models_event());
                }
                Err(e) => conn.send(Event::Error { message: format!("{:#}", e) }),
            },
            Request::SetThinking { enabled } => match d.set_thinking(enabled) {
                Ok(()) => conn.send(d.models_event()),
                Err(e) => conn.send(Event::Error { message: format!("{:#}", e) }),
            },
            Request::DownloadModel { url, file, size, use_after } => {
                match d.start_download(url.clone(), file.clone(), size, use_after, conn.tx.clone()) {
                    Ok(()) => d.audit.record("download", "", conn.uid, serde_json::json!({"url": url, "file": file, "client": conn.client})),
                    Err(e) => conn.send(Event::Error { message: format!("{:#}", e) }),
                }
            }
            Request::CancelDownload => d.cancel_download(),
            Request::Chat { session, text, autopilot } => {
                if d.is_sleeping() {
                    conn.send(Event::Delta { text: String::new(), kind: "status".into() });
                    if !d.wake_and_wait(120).await {
                        conn.send(Event::Error { message: "the Mind is waking up (a game asked it to sleep); try again in a moment".into() });
                        continue;
                    }
                }
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
                agent::refresh_status(&d, &mut sess);
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
    // The daemon's own lines (the status block, the clock lines) are not
    // conversation: they do not count, and once the cap is hit they all go.
    // The next turn posts a fresh status before its question; the prompt
    // cache is refilled at that point anyway, the front of the conversation
    // having moved.
    let conversation = s.messages.iter().filter(|m| !agent::is_status(m)).count();
    if conversation <= max_messages {
        return;
    }
    let head: Vec<_> = s.messages.iter().take_while(|m| m.role == "system").cloned().collect();
    let keep = conversation - max_messages;
    let mut rest: Vec<_> = s.messages.drain(head.len()..).filter(|m| !agent::is_status(m)).collect();
    rest.drain(..keep.min(rest.len()));
    // never start with a tool result whose call was dropped
    while rest.first().map(|m| m.role == "tool").unwrap_or(false) {
        rest.remove(0);
    }
    s.messages = head;
    s.messages.extend(rest);
}
