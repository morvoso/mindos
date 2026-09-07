//! The polkit authentication agent. Without one, everything that asks
//! polkit for authorisation (pkexec, GNOME's disk and printer dialogs,
//! systemd unit management, GameMode's helpers) fails with "no
//! authentication agent". The shell registers itself for the session, shows
//! the password dialog (`HostEvent::Polkit` → the `auth` popup) and lets
//! `polkit-agent-helper-1` do the PAM conversation, which answers polkitd
//! itself.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value as ZValue};

use crate::HostEvent;

/// The helper that runs the PAM conversation and tells polkitd about it.
/// polkit 127 dropped the setuid bit and socket-activates it instead: connect,
/// write the user name and the cookie, and the same line protocol follows.
/// Older polkit only has the setuid binary, which takes the user as its
/// argument and the cookie on stdin.
const HELPER: &str = "/usr/lib/polkit-1/polkit-agent-helper-1";
const HELPER_SOCKET: &str = "/run/polkit/agent-helper.socket";
const AGENT_PATH: &str = "/org/mindos/PolkitAgent";
/// Password attempts per authorisation, as the GNOME agent does it.
const MAX_TRIES: u32 = 3;

type Replies = Arc<Mutex<HashMap<u64, async_channel::Sender<Option<String>>>>>;

#[derive(Clone)]
pub struct PolkitHandle {
    replies: Replies,
}

impl PolkitHandle {
    pub fn start(events: async_channel::Sender<HostEvent>) -> PolkitHandle {
        let replies: Replies = Arc::new(Mutex::new(HashMap::new()));
        let worker = replies.clone();
        std::thread::Builder::new()
            .name("mindshell-polkit".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    Ok(rt) => rt,
                    Err(e) => {
                        tracing::error!(%e, "cannot start the polkit agent runtime");
                        return;
                    }
                };
                rt.block_on(run(events, worker));
            })
            .expect("spawn polkit thread");
        PolkitHandle { replies }
    }

    /// The dialog answered: a password, or `None` to cancel.
    pub fn respond(&self, id: u64, password: Option<String>) {
        let tx = self.replies.lock().ok().and_then(|r| r.get(&id).cloned());
        if let Some(tx) = tx {
            let _ = tx.try_send(password);
        }
    }
}

struct Agent {
    events: async_channel::Sender<HostEvent>,
    replies: Replies,
    /// Cookie → request id, so `CancelAuthentication` finds the dialog.
    pending: Arc<Mutex<HashMap<String, u64>>>,
    next: AtomicU64,
    /// One dialog at a time; a second request waits its turn.
    gate: tokio::sync::Mutex<()>,
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.PolicyKit1.Error")]
enum PolkitError {
    #[zbus(error)]
    ZBus(zbus::Error),
    Failed(String),
    Cancelled(String),
}

#[zbus::interface(name = "org.freedesktop.PolicyKit1.AuthenticationAgent")]
impl Agent {
    #[allow(clippy::too_many_arguments)]
    async fn begin_authentication(
        &self,
        action_id: String,
        message: String,
        icon_name: String,
        details: HashMap<String, String>,
        cookie: String,
        identities: Vec<(String, HashMap<String, OwnedValue>)>,
    ) -> Result<(), PolkitError> {
        let _gate = self.gate.lock().await;
        let users = identity_users(&identities);
        let Some(user) = pick_user(&users) else {
            return Err(PolkitError::Failed("no account can authorise this".into()));
        };
        tracing::info!(action = %action_id, %user, "authorisation requested");

        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = async_channel::bounded::<Option<String>>(1);
        self.replies.lock().unwrap().insert(id, tx);
        self.pending.lock().unwrap().insert(cookie.clone(), id);

        let command = details.get("command_line").or_else(|| details.get("program")).cloned().unwrap_or_default();
        let mut error = String::new();
        let mut result = Err(PolkitError::Cancelled("cancelled".into()));
        for attempt in 1..=MAX_TRIES {
            let ask = json!({
                "type": "ask",
                "id": id,
                "action": action_id,
                "message": message,
                "icon": icon_name,
                "user": user,
                "users": users,
                "command": command,
                "error": error,
                "attempt": attempt,
                "tries": MAX_TRIES,
            });
            if self.events.send(HostEvent::Polkit(ask)).await.is_err() {
                break;
            }
            let Ok(Some(password)) = rx.recv().await else { break };
            let _ = self.events.send(HostEvent::Polkit(json!({ "type": "busy", "id": id }))).await;

            let (user2, cookie2) = (user.clone(), cookie.clone());
            let (done_tx, done_rx) = async_channel::bounded(1);
            std::thread::Builder::new()
                .name("mindshell-polkit-pam".into())
                .spawn(move || {
                    let _ = done_tx.send_blocking(authenticate(&user2, &cookie2, &password));
                })
                .map_err(|e| PolkitError::Failed(format!("cannot run the authentication helper: {e}")))?;
            match done_rx.recv().await {
                Ok(Ok(())) => {
                    tracing::info!(action = %action_id, "authorisation granted");
                    result = Ok(());
                    break;
                }
                Ok(Err(e)) => error = e,
                Err(_) => error = "the authentication helper stopped".into(),
            }
            if attempt == MAX_TRIES {
                tracing::info!(action = %action_id, "authorisation refused");
                result = Err(PolkitError::Failed(error.clone()));
            }
        }

        self.replies.lock().unwrap().remove(&id);
        self.pending.lock().unwrap().remove(&cookie);
        let _ = self.events.send(HostEvent::Polkit(json!({ "type": "done", "id": id }))).await;
        result
    }

    async fn cancel_authentication(&self, cookie: String) {
        let id = self.pending.lock().unwrap().get(&cookie).copied();
        if let Some(id) = id {
            let tx = self.replies.lock().unwrap().get(&id).cloned();
            if let Some(tx) = tx {
                let _ = tx.try_send(None);
            }
        }
    }
}

/// One PAM conversation through the helper: it asks for the secret, we
/// answer, and on success it tells polkitd (as root) that the identity
/// authenticated. Blocking, so it runs on its own thread.
fn authenticate(user: &str, cookie: &str, password: &str) -> Result<(), String> {
    if let Ok(sock) = UnixStream::connect(HELPER_SOCKET) {
        let reader = BufReader::new(sock.try_clone().map_err(|e| e.to_string())?);
        let mut writer = sock;
        writeln!(writer, "{user}\n{cookie}").and_then(|_| writer.flush()).map_err(|e| e.to_string())?;
        return converse(reader, writer, password);
    }
    let mut child = Command::new(HELPER)
        .arg(user)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{HELPER}: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("no pipe to the authentication helper")?;
    let stdout = BufReader::new(child.stdout.take().ok_or("no pipe from the authentication helper")?);
    writeln!(stdin, "{cookie}").and_then(|_| stdin.flush()).map_err(|e| e.to_string())?;
    let result = converse(stdout, stdin, password);
    let _ = child.wait();
    result
}

/// The helper's line protocol, the same over the socket and over the pipes:
/// it asks, we answer with the password, and it ends in SUCCESS or FAILURE.
fn converse<R: BufRead, W: Write>(stdout: R, mut stdin: W, password: &str) -> Result<(), String> {
    let mut ok = false;
    let mut message = String::new();
    for line in stdout.lines() {
        let Ok(line) = line else { break };
        let (tag, rest) = line.split_once(' ').unwrap_or((line.as_str(), ""));
        match tag {
            "PAM_PROMPT_ECHO_OFF" | "PAM_PROMPT_ECHO_ON" => {
                if writeln!(stdin, "{password}").and_then(|_| stdin.flush()).is_err() {
                    break;
                }
            }
            "PAM_ERROR_MSG" => message = rest.trim().to_string(),
            "PAM_TEXT_INFO" => {
                if message.is_empty() {
                    message = rest.trim().to_string();
                }
            }
            "SUCCESS" => ok = true,
            "FAILURE" => ok = false,
            _ => {}
        }
    }
    drop(stdin);
    if ok {
        Ok(())
    } else if message.is_empty() {
        Err("That password did not work.".into())
    } else {
        Err(message)
    }
}

fn as_u32(v: &OwnedValue) -> Option<u32> {
    match &**v {
        ZValue::U32(n) => Some(*n),
        ZValue::I32(n) => Some(*n as u32),
        ZValue::U64(n) => Some(*n as u32),
        _ => None,
    }
}

/// The accounts polkit will accept for this action: `unix-user` identities
/// as they are, `unix-group` ones expanded to their members.
fn identity_users(identities: &[(String, HashMap<String, OwnedValue>)]) -> Vec<String> {
    let mut users: Vec<String> = vec![];
    let mut push = |name: String| {
        if !name.is_empty() && !users.contains(&name) {
            users.push(name);
        }
    };
    for (kind, details) in identities {
        match kind.as_str() {
            "unix-user" => {
                if let Some(name) = details.get("uid").and_then(as_u32).and_then(user_by_uid) {
                    push(name);
                }
            }
            "unix-group" => {
                if let Some(gid) = details.get("gid").and_then(as_u32) {
                    for name in group_members(gid) {
                        push(name);
                    }
                }
            }
            _ => {}
        }
    }
    users
}

/// Who the dialog offers: the user at the keyboard when they qualify (they
/// are in `wheel` on a MindOS install), else the first account listed.
fn pick_user(users: &[String]) -> Option<String> {
    let me = crate::system::user_name();
    if users.iter().any(|u| *u == me) {
        return Some(me);
    }
    users.iter().find(|u| *u != "root").or_else(|| users.first()).cloned()
}

fn passwd_lines() -> Vec<Vec<String>> {
    std::fs::read_to_string("/etc/passwd")
        .unwrap_or_default()
        .lines()
        .map(|l| l.split(':').map(str::to_string).collect())
        .collect()
}

fn user_by_uid(uid: u32) -> Option<String> {
    passwd_lines()
        .into_iter()
        .find(|f| f.len() > 2 && f[2].parse::<u32>().ok() == Some(uid))
        .map(|f| f[0].clone())
}

/// Members of a group: the ones listed in /etc/group plus everyone whose
/// primary group it is.
fn group_members(gid: u32) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for line in std::fs::read_to_string("/etc/group").unwrap_or_default().lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() > 3 && f[2].parse::<u32>().ok() == Some(gid) {
            out.extend(f[3].split(',').filter(|s| !s.is_empty()).map(str::to_string));
        }
    }
    for f in passwd_lines() {
        if f.len() > 3 && f[3].parse::<u32>().ok() == Some(gid) && !out.contains(&f[0]) {
            out.push(f[0].clone());
        }
    }
    out
}

/// The logind session this shell runs in; polkit registers agents per session.
async fn session_id(conn: &zbus::Connection) -> Option<String> {
    if let Ok(s) = std::env::var("XDG_SESSION_ID") {
        if !s.is_empty() {
            return Some(s);
        }
    }
    let reply = conn
        .call_method(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1",
            Some("org.freedesktop.login1.Manager"),
            "GetSessionByPID",
            &(std::process::id(),),
        )
        .await
        .ok()?;
    let path: OwnedObjectPath = reply.body().deserialize().ok()?;
    let reply = conn
        .call_method(
            Some("org.freedesktop.login1"),
            &path,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.freedesktop.login1.Session", "Id"),
        )
        .await
        .ok()?;
    let value: OwnedValue = reply.body().deserialize().ok()?;
    match &*value {
        ZValue::Str(s) => Some(s.to_string()),
        _ => None,
    }
}

async fn register(conn: &zbus::Connection, session: &str) -> zbus::Result<()> {
    let mut subject: HashMap<&str, ZValue> = HashMap::new();
    subject.insert("session-id", ZValue::from(session));
    let locale = std::env::var("LANG").unwrap_or_else(|_| "en_US.UTF-8".into());
    // The object path goes over the wire as a plain string here: polkit's
    // signature is ((sa{sv})ss), not ((sa{sv})so).
    conn.call_method(
        Some("org.freedesktop.PolicyKit1"),
        "/org/freedesktop/PolicyKit1/Authority",
        Some("org.freedesktop.PolicyKit1.Authority"),
        "RegisterAuthenticationAgent",
        &(("unix-session", subject), locale, AGENT_PATH),
    )
    .await?;
    Ok(())
}

async fn run(events: async_channel::Sender<HostEvent>, replies: Replies) {
    if !std::path::Path::new(HELPER).exists() && !std::path::Path::new(HELPER_SOCKET).exists() {
        tracing::warn!(HELPER, "polkit agent not started (the helper is missing; is polkit installed?)");
        return;
    }
    let agent = Agent {
        events,
        replies,
        pending: Arc::new(Mutex::new(HashMap::new())),
        next: AtomicU64::new(1),
        gate: tokio::sync::Mutex::new(()),
    };
    let conn = match zbus::connection::Builder::system().and_then(|b| b.serve_at(AGENT_PATH, agent)) {
        Ok(b) => match b.build().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(%e, "polkit agent not started (no system bus)");
                return;
            }
        },
        Err(e) => {
            tracing::warn!(%e, "polkit agent not started");
            return;
        }
    };
    let Some(session) = session_id(&conn).await else {
        tracing::warn!("polkit agent not started (no logind session)");
        return;
    };
    // polkitd is D-Bus activated and the session may still be settling.
    for attempt in 1..=12 {
        match register(&conn, &session).await {
            Ok(()) => {
                tracing::info!(session, "polkit authentication agent registered");
                std::future::pending::<()>().await;
                return;
            }
            Err(e) if attempt == 12 => {
                tracing::warn!(%e, "polkit agent could not register");
                return;
            }
            Err(e) => tracing::debug!(%e, attempt, "polkit agent registration failed, retrying"),
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

/// Everything the dialog needs, as the UI sees it (`shell.state.polkit`).
pub fn request_json(v: &Value) -> Value {
    json!({
        "id": v.get("id").cloned().unwrap_or(Value::Null),
        "action": v.get("action").cloned().unwrap_or(Value::Null),
        "message": v.get("message").cloned().unwrap_or(Value::Null),
        "icon": v.get("icon").cloned().unwrap_or(Value::Null),
        "user": v.get("user").cloned().unwrap_or(Value::Null),
        "users": v.get("users").cloned().unwrap_or(json!([])),
        "command": v.get("command").cloned().unwrap_or(Value::Null),
        "error": v.get("error").cloned().unwrap_or(Value::Null),
        "attempt": v.get("attempt").cloned().unwrap_or(json!(1)),
        "tries": v.get("tries").cloned().unwrap_or(json!(MAX_TRIES)),
        "busy": false,
    })
}
