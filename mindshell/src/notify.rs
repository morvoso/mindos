//! The freedesktop notification server (`org.freedesktop.Notifications`) on
//! its own zbus/tokio thread. Applications' notifications reach the main
//! loop as JSON (`HostEvent::Notify`); the UI closes them and invokes their
//! actions through `NotifyHandle`, which emits the D-Bus signals.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Structure, Value as ZValue};

use crate::HostEvent;

/// `image-data` pixmaps by notification id, encoded as PNG.
type Pixmaps = Arc<Mutex<HashMap<u32, Vec<u8>>>>;

#[derive(Clone)]
pub struct NotifyHandle {
    cmd: async_channel::Sender<NotifyCommand>,
    pixmaps: Pixmaps,
}

pub enum NotifyCommand {
    /// Tell the application its notification is gone (reason: 1 expired, 2 dismissed, 3 closed by call).
    Closed { id: u32, reason: u32 },
    Action { id: u32, key: String },
}

impl NotifyHandle {
    pub fn start(events: async_channel::Sender<HostEvent>) -> NotifyHandle {
        let (cmd_tx, cmd_rx) = async_channel::unbounded::<NotifyCommand>();
        let pixmaps: Pixmaps = Arc::new(Mutex::new(HashMap::new()));
        let worker = pixmaps.clone();
        std::thread::Builder::new()
            .name("mindshell-notify".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    Ok(rt) => rt,
                    Err(e) => {
                        tracing::error!(%e, "cannot start the notification runtime");
                        return;
                    }
                };
                rt.block_on(run(events, cmd_rx, worker));
            })
            .expect("spawn notify thread");
        NotifyHandle { cmd: cmd_tx, pixmaps }
    }

    pub fn pixmap(&self, id: u32) -> Option<Vec<u8>> {
        self.pixmaps.lock().ok()?.get(&id).cloned()
    }

    pub fn closed(&self, id: u32, reason: u32) {
        if let Ok(mut p) = self.pixmaps.lock() {
            p.remove(&id);
        }
        let _ = self.cmd.send_blocking(NotifyCommand::Closed { id, reason });
    }

    pub fn action(&self, id: u32, key: String) {
        let _ = self.cmd.send_blocking(NotifyCommand::Action { id, key });
    }
}

struct Server {
    events: async_channel::Sender<HostEvent>,
    pixmaps: Pixmaps,
    next_id: AtomicU32,
}

fn hint_str(hints: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    let v = hints.get(key)?;
    match &**v {
        ZValue::Str(s) => Some(s.to_string()),
        _ => None,
    }
}

fn hint_u8(hints: &HashMap<String, OwnedValue>, key: &str) -> Option<u8> {
    let v = hints.get(key)?;
    match &**v {
        ZValue::U8(n) => Some(*n),
        ZValue::I32(n) => Some((*n).clamp(0, 2) as u8),
        _ => None,
    }
}

/// `image-data` / `icon_data`: (width, height, rowstride, has_alpha, bits, channels, bytes) → PNG.
fn hint_image(hints: &HashMap<String, OwnedValue>) -> Option<Vec<u8>> {
    let v = hints.get("image-data").or_else(|| hints.get("image_data")).or_else(|| hints.get("icon_data"))?;
    let s: &Structure = match &**v {
        ZValue::Structure(s) => s,
        _ => return None,
    };
    let f = s.fields();
    if f.len() < 7 {
        return None;
    }
    let as_i32 = |v: &ZValue| -> Option<i32> {
        match v {
            ZValue::I32(n) => Some(*n),
            ZValue::U32(n) => Some(*n as i32),
            _ => None,
        }
    };
    let width = as_i32(&f[0])?;
    let height = as_i32(&f[1])?;
    let stride = as_i32(&f[2])?;
    let alpha = matches!(&f[3], ZValue::Bool(true));
    let channels = as_i32(&f[5])?;
    let data: Vec<u8> = match &f[6] {
        ZValue::Array(a) => a.iter().filter_map(|b| if let ZValue::U8(x) = b { Some(*x) } else { None }).collect(),
        _ => return None,
    };
    if width <= 0 || height <= 0 || stride <= 0 || channels < 3 {
        return None;
    }
    let (w, h, stride, ch) = (width as usize, height as usize, stride as usize, channels as usize);
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let i = y * stride + x * ch;
            if i + ch > data.len() {
                return None;
            }
            rgba.extend_from_slice(&[data[i], data[i + 1], data[i + 2], if alpha && ch >= 4 { data[i + 3] } else { 255 }]);
        }
    }
    crate::tray::encode_rgba_png(w as u32, h as u32, &rgba)
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Server {
    fn get_capabilities(&self) -> Vec<String> {
        ["body", "body-markup", "actions", "action-icons", "icon-static", "persistence"].iter().map(|s| s.to_string()).collect()
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        ("mindshell".into(), "MindOS".into(), env!("CARGO_PKG_VERSION").into(), "1.2".into())
    }

    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        let id = if replaces_id > 0 { replaces_id } else { self.next_id.fetch_add(1, Ordering::Relaxed) };
        let acts: Vec<Value> = actions.chunks(2).map(|c| json!({ "key": c[0], "label": c.get(1).cloned().unwrap_or_else(|| c[0].clone()) })).collect();
        let urgency = hint_u8(&hints, "urgency").unwrap_or(1);
        let image = hint_image(&hints);
        let has_image = image.is_some();
        if let Some(png) = image {
            if let Ok(mut p) = self.pixmaps.lock() {
                p.insert(id, png);
            }
        }
        let image_path = hint_str(&hints, "image-path").or_else(|| hint_str(&hints, "image_path"));
        let icon = if has_image {
            format!("{}/notify/{}", crate::scheme::ORIGIN, id)
        } else if let Some(p) = image_path.filter(|p| !p.is_empty()) {
            icon_ref(&p)
        } else if !app_icon.is_empty() {
            icon_ref(&app_icon)
        } else {
            String::new()
        };
        let desktop = hint_str(&hints, "desktop-entry").unwrap_or_default();
        let value = json!({
            "id": id,
            "app": app_name,
            "desktop": desktop,
            "icon": icon,
            "summary": summary,
            "body": body,
            "actions": acts,
            "urgency": urgency,
            "resident": matches!(hints.get("resident"), Some(v) if matches!(&**v, ZValue::Bool(true))),
            "transient": matches!(hints.get("transient"), Some(v) if matches!(&**v, ZValue::Bool(true))),
            "category": hint_str(&hints, "category").unwrap_or_default(),
            "timeout": expire_timeout,
            "time": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
            "replaced": replaces_id > 0,
        });
        tracing::debug!(id, app = %value["app"], summary = %value["summary"], "notification");
        let _ = self.events.send_blocking(HostEvent::Notify(value));
        id
    }

    fn close_notification(&self, id: u32) {
        let _ = self.events.send_blocking(HostEvent::NotifyClosed(id, 3));
    }

    #[zbus(signal)]
    async fn notification_closed(emitter: &SignalEmitter<'_>, id: u32, reason: u32) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_invoked(emitter: &SignalEmitter<'_>, id: u32, action_key: String) -> zbus::Result<()>;
}

/// An icon name or a file path/URI → what the UI can load.
fn icon_ref(s: &str) -> String {
    let s = s.strip_prefix("file://").unwrap_or(s);
    if s.starts_with('/') {
        format!("{}/file/{}", crate::scheme::ORIGIN, crate::icons::percent_encode(s))
    } else {
        crate::app::icon_url(s, 48)
    }
}

async fn run(events: async_channel::Sender<HostEvent>, cmd_rx: async_channel::Receiver<NotifyCommand>, pixmaps: Pixmaps) {
    let server = Server { events: events.clone(), pixmaps, next_id: AtomicU32::new(1) };
    let conn = match zbus::connection::Builder::session().and_then(|b| b.name("org.freedesktop.Notifications")).and_then(|b| b.serve_at("/org/freedesktop/Notifications", server)) {
        Ok(b) => match b.build().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(%e, "notification server not started (another one owns the name?)");
                return;
            }
        },
        Err(e) => {
            tracing::warn!(%e, "notification server not started");
            return;
        }
    };
    tracing::info!("notification server up (org.freedesktop.Notifications)");
    let iface = match conn.object_server().interface::<_, Server>("/org/freedesktop/Notifications").await {
        Ok(i) => i,
        Err(e) => {
            tracing::error!(%e, "notification interface missing");
            return;
        }
    };
    while let Ok(cmd) = cmd_rx.recv().await {
        let emitter = iface.signal_emitter();
        let r = match cmd {
            NotifyCommand::Closed { id, reason } => Server::notification_closed(emitter, id, reason).await,
            NotifyCommand::Action { id, key } => Server::action_invoked(emitter, id, key).await,
        };
        if let Err(e) = r {
            tracing::debug!(%e, "notification signal failed");
        }
    }
}
