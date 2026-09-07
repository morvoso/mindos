//! The login screen's system side (`mindshell --app greeter`).
//!
//! greetd owns authentication: the greeter only relays the conversation over
//! greetd's socket (`$GREETD_SOCK`, JSON messages with a native-endian u32
//! length prefix). This module lists the accounts and sessions to offer,
//! remembers who logged in last and drives one login at a time. It runs as
//! the unprivileged `greeter` user; nothing here needs more.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// Where the greeter keeps its state; `mindos-session` creates it for the
/// `greeter` user through systemd-tmpfiles.
const STATE_DIR: &str = "/var/lib/mindos/greeter";
const SESSIONS_DIR: &str = "/usr/share/wayland-sessions";
const AVATAR_DIR: &str = "/var/lib/AccountsService/icons";

#[derive(Debug, Clone)]
pub struct User {
    pub name: String,
    pub display: String,
    pub avatar: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub exec: String,
}

/// Login accounts: a login shell and a uid in the regular range.
pub fn users() -> Vec<User> {
    let Ok(text) = std::fs::read_to_string("/etc/passwd") else { return Vec::new() };
    let mut list: Vec<User> = text
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            if f.len() < 7 {
                return None;
            }
            let uid: u32 = f[2].parse().ok()?;
            if !(1000..60000).contains(&uid) {
                return None;
            }
            let shell = f[6].trim();
            if shell.is_empty() || shell.ends_with("nologin") || shell.ends_with("/false") {
                return None;
            }
            let name = f[0].to_string();
            let gecos = f[4].split(',').next().unwrap_or("").trim();
            let display = if gecos.is_empty() { name.clone() } else { gecos.to_string() };
            let avatar = Path::new(AVATAR_DIR).join(&name);
            let avatar = std::fs::File::open(&avatar).is_ok().then_some(avatar);
            Some(User { name, display, avatar })
        })
        .collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    list
}

/// The Wayland sessions installed, MindOS first.
pub fn sessions() -> Vec<Session> {
    let mut list = Vec::new();
    if let Ok(dir) = std::fs::read_dir(SESSIONS_DIR) {
        for entry in dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let mut name = String::new();
            let mut exec = String::new();
            let mut hidden = false;
            for line in text.lines() {
                let line = line.trim();
                if let Some(v) = line.strip_prefix("Name=") {
                    if name.is_empty() {
                        name = v.trim().to_string();
                    }
                } else if let Some(v) = line.strip_prefix("Exec=") {
                    exec = v.trim().to_string();
                } else if line == "Hidden=true" || line == "NoDisplay=true" {
                    hidden = true;
                }
            }
            if hidden || exec.is_empty() {
                continue;
            }
            let id = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            if name.is_empty() {
                name = id.clone();
            }
            list.push(Session { id, name, exec });
        }
    }
    list.sort_by(|a, b| (a.id != "mindos", &a.name).cmp(&(b.id != "mindos", &b.name)));
    list
}

/// Who logged in last, and with which session.
pub fn load_state() -> Value {
    std::fs::read_to_string(Path::new(STATE_DIR).join("state.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}))
}

pub fn save_state(user: &str, session: &str) {
    let path = Path::new(STATE_DIR).join("state.json");
    let text = json!({ "user": user, "session": session }).to_string();
    if let Err(e) = std::fs::write(&path, text) {
        tracing::warn!(path = %path.display(), %e, "cannot remember the last login");
    }
}

/// What the conversation with greetd came to.
#[derive(Debug, Clone)]
pub enum Outcome {
    /// Authenticated and the session is queued: greetd starts it as soon as
    /// the greeter exits.
    Started,
    /// PAM wants something more (a one-time code, a new password, ...):
    /// show `message`, collect an answer, then `Login::respond`.
    Prompt { secret: bool, message: String, notes: Vec<String> },
    /// Authentication failed; the greetd session is cancelled.
    Failed { message: String, notes: Vec<String> },
}

/// One login conversation with greetd.
pub struct Login {
    pub user: String,
    stream: UnixStream,
    /// The password, handed over at the first secret prompt.
    password: Option<String>,
    cmd: Vec<String>,
    env: Vec<String>,
    /// Informational PAM messages collected on the way.
    notes: Vec<String>,
}

impl Login {
    /// Begin: create the greetd session for `user` and answer its prompts
    /// with `password`.
    pub fn start(user: &str, password: &str, cmd: Vec<String>, env: Vec<String>) -> Result<(Login, Outcome), String> {
        let sock = std::env::var("GREETD_SOCK").map_err(|_| "GREETD_SOCK is not set: not started by greetd".to_string())?;
        let stream = UnixStream::connect(&sock).map_err(|e| format!("cannot reach greetd at {sock}: {e}"))?;
        let mut login = Login {
            user: user.to_string(),
            stream,
            password: Some(password.to_string()),
            cmd,
            env,
            notes: Vec::new(),
        };
        let reply = login.send(json!({ "type": "create_session", "username": user }))?;
        let outcome = login.drive(reply)?;
        Ok((login, outcome))
    }

    /// Answer the prompt the last outcome asked for (`None` = no answer).
    pub fn respond(&mut self, response: Option<String>) -> Result<Outcome, String> {
        let reply = self.send(json!({ "type": "post_auth_message_response", "response": response }))?;
        self.drive(reply)
    }

    /// Abandon the conversation.
    pub fn cancel(&mut self) {
        let _ = self.send(json!({ "type": "cancel_session" }));
    }

    fn drive(&mut self, mut reply: Value) -> Result<Outcome, String> {
        loop {
            let kind = reply.get("type").and_then(Value::as_str).unwrap_or("");
            match kind {
                "success" => {
                    if self.cmd.is_empty() {
                        return Ok(Outcome::Started);
                    }
                    let cmd = std::mem::take(&mut self.cmd);
                    let env = std::mem::take(&mut self.env);
                    reply = self.send(json!({ "type": "start_session", "cmd": cmd, "env": env }))?;
                }
                "error" => {
                    let description = reply.get("description").and_then(Value::as_str).unwrap_or("").to_string();
                    let error_type = reply.get("error_type").and_then(Value::as_str).unwrap_or("error");
                    self.cancel();
                    if error_type == "auth_error" {
                        return Ok(Outcome::Failed {
                            message: description,
                            notes: std::mem::take(&mut self.notes),
                        });
                    }
                    return Err(if description.is_empty() { "greetd refused the login".into() } else { description });
                }
                "auth_message" => {
                    let message = reply.get("auth_message").and_then(Value::as_str).unwrap_or("").trim().to_string();
                    match reply.get("auth_message_type").and_then(Value::as_str).unwrap_or("") {
                        "secret" => {
                            if let Some(password) = self.password.take() {
                                reply = self.send(json!({ "type": "post_auth_message_response", "response": password }))?;
                            } else {
                                return Ok(Outcome::Prompt { secret: true, message, notes: std::mem::take(&mut self.notes) });
                            }
                        }
                        "visible" => {
                            return Ok(Outcome::Prompt { secret: false, message, notes: std::mem::take(&mut self.notes) });
                        }
                        _ => {
                            // info / error: remember it, nothing to answer.
                            if !message.is_empty() {
                                self.notes.push(message);
                            }
                            reply = self.send(json!({ "type": "post_auth_message_response", "response": null }))?;
                        }
                    }
                }
                other => return Err(format!("unexpected reply from greetd: {other}")),
            }
        }
    }

    fn send(&mut self, request: Value) -> Result<Value, String> {
        let payload = request.to_string();
        let len = (payload.len() as u32).to_ne_bytes();
        self.stream.write_all(&len).and_then(|_| self.stream.write_all(payload.as_bytes())).map_err(|e| format!("greetd: write: {e}"))?;
        let mut len = [0u8; 4];
        self.stream.read_exact(&mut len).map_err(|e| format!("greetd: read: {e}"))?;
        let len = u32::from_ne_bytes(len) as usize;
        if len > 1 << 20 {
            return Err("greetd: reply too large".into());
        }
        let mut buf = vec![0u8; len];
        self.stream.read_exact(&mut buf).map_err(|e| format!("greetd: read: {e}"))?;
        serde_json::from_slice(&buf).map_err(|e| format!("greetd: bad reply: {e}"))
    }
}

/// The JSON the UI gets from `greeter.info`.
pub fn info_json() -> Value {
    let last = load_state();
    json!({
        "users": users().iter().map(|u| json!({
            "name": u.name,
            "display": u.display,
            "avatar": u.avatar.as_ref().map(|p| crate::scheme::file_url(p)),
        })).collect::<Vec<_>>(),
        "sessions": sessions().iter().map(|s| json!({ "id": s.id, "name": s.name, "exec": s.exec })).collect::<Vec<_>>(),
        "last": last,
        "host": crate::system::host_name(),
    })
}

pub fn outcome_json(outcome: &Outcome) -> Value {
    match outcome {
        Outcome::Started => json!({ "status": "started" }),
        Outcome::Prompt { secret, message, notes } => json!({ "status": "prompt", "secret": secret, "message": message, "notes": notes }),
        Outcome::Failed { message, notes } => json!({ "status": "failed", "message": message, "notes": notes }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_put_mindos_first() {
        let mut list = vec![
            Session { id: "sway".into(), name: "Sway".into(), exec: "sway".into() },
            Session { id: "mindos".into(), name: "MindOS".into(), exec: "mindos-session".into() },
        ];
        list.sort_by(|a, b| (a.id != "mindos", &a.name).cmp(&(b.id != "mindos", &b.name)));
        assert_eq!(list[0].id, "mindos");
    }
}
