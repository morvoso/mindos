//! StatusNotifierItem / DBusMenu host, through the `system-tray` crate on its
//! own tokio thread. The main loop receives item snapshots as JSON and sends
//! commands (activate, scroll, menu) through a channel.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use system_tray::client::{ActivateRequest, Client, Event};
use system_tray::item::{IconPixmap, Status, StatusNotifierItem};
use system_tray::menu::{MenuItem, MenuType, ToggleState, ToggleType, TrayMenu};

use crate::icons;
use crate::HostEvent;

pub type Reply<T> = async_channel::Sender<Result<T, String>>;

pub enum TrayCommand {
    Activate { id: String, x: i32, y: i32, secondary: bool, reply: Reply<()> },
    Scroll { id: String, delta: i32, orientation: String, reply: Reply<()> },
    Menu { id: String, reply: Reply<Value> },
    MenuClick { id: String, item: i32, reply: Reply<()> },
}

/// Encoded PNG pixmaps by item id, with a version that busts the view cache.
type Pixmaps = Arc<Mutex<HashMap<String, (u64, Vec<u8>)>>>;

#[derive(Clone)]
pub struct TrayHandle {
    cmd: async_channel::Sender<TrayCommand>,
    pixmaps: Pixmaps,
}

impl TrayHandle {
    pub fn start(events: async_channel::Sender<HostEvent>, icon_theme: String) -> TrayHandle {
        let (cmd_tx, cmd_rx) = async_channel::unbounded::<TrayCommand>();
        let pixmaps: Pixmaps = Arc::new(Mutex::new(HashMap::new()));
        let worker_pixmaps = pixmaps.clone();
        std::thread::Builder::new()
            .name("mindshell-tray".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    Ok(rt) => rt,
                    Err(e) => {
                        tracing::error!(%e, "cannot start the tray runtime");
                        return;
                    }
                };
                rt.block_on(run(events, cmd_rx, worker_pixmaps, icon_theme));
            })
            .expect("spawn tray thread");
        TrayHandle { cmd: cmd_tx, pixmaps }
    }

    pub fn pixmap(&self, id: &str) -> Option<Vec<u8>> {
        self.pixmaps.lock().ok()?.get(id).map(|(_, png)| png.clone())
    }

    pub async fn activate(&self, id: String, x: i32, y: i32, secondary: bool) -> Result<(), String> {
        let (reply, rx) = async_channel::bounded(1);
        self.send(TrayCommand::Activate { id, x, y, secondary, reply })?;
        rx.recv().await.map_err(|_| "tray thread gone".to_string())?
    }

    pub async fn scroll(&self, id: String, delta: i32, orientation: String) -> Result<(), String> {
        let (reply, rx) = async_channel::bounded(1);
        self.send(TrayCommand::Scroll { id, delta, orientation, reply })?;
        rx.recv().await.map_err(|_| "tray thread gone".to_string())?
    }

    pub async fn menu(&self, id: String) -> Result<Value, String> {
        let (reply, rx) = async_channel::bounded(1);
        self.send(TrayCommand::Menu { id, reply })?;
        rx.recv().await.map_err(|_| "tray thread gone".to_string())?
    }

    pub async fn menu_click(&self, id: String, item: i32) -> Result<(), String> {
        let (reply, rx) = async_channel::bounded(1);
        self.send(TrayCommand::MenuClick { id, item, reply })?;
        rx.recv().await.map_err(|_| "tray thread gone".to_string())?
    }

    fn send(&self, cmd: TrayCommand) -> Result<(), String> {
        self.cmd.try_send(cmd).map_err(|_| "tray unavailable".to_string())
    }
}

async fn run(
    events: async_channel::Sender<HostEvent>,
    cmd_rx: async_channel::Receiver<TrayCommand>,
    pixmaps: Pixmaps,
    icon_theme: String,
) {
    let mut versions: HashMap<String, u64> = HashMap::new();
    let mut last_snapshot: Option<String> = None;
    loop {
        let client = match Client::new().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(%e, "system tray unavailable (no session bus or watcher); retrying");
                // Answer commands while we have no client so callers do not hang.
                let wait = tokio::time::sleep(Duration::from_secs(5));
                tokio::pin!(wait);
                loop {
                    tokio::select! {
                        _ = &mut wait => break,
                        cmd = cmd_rx.recv() => match cmd {
                            Ok(cmd) => fail(cmd, "system tray unavailable"),
                            Err(_) => return,
                        },
                    }
                }
                continue;
            }
        };
        tracing::info!("system tray host running");
        let mut rx = client.subscribe();
        let bus = zbus::Connection::session().await.ok();
        publish(&client, &pixmaps, &mut versions, &icon_theme, &events, &mut last_snapshot);
        loop {
            tokio::select! {
                ev = rx.recv() => match ev {
                    Ok(ev) => {
                        if let Event::Remove(id) = &ev {
                            if let Ok(mut p) = pixmaps.lock() {
                                p.remove(id);
                            }
                        }
                        publish(&client, &pixmaps, &mut versions, &icon_theme, &events, &mut last_snapshot);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        publish(&client, &pixmaps, &mut versions, &icon_theme, &events, &mut last_snapshot);
                    }
                    Err(_) => break,
                },
                cmd = cmd_rx.recv() => match cmd {
                    Ok(cmd) => handle(&client, bus.as_ref(), cmd).await,
                    Err(_) => return,
                },
            }
        }
        tracing::warn!("system tray client stopped; restarting");
        if let Ok(mut p) = pixmaps.lock() {
            p.clear();
        }
        let _ = events.send(HostEvent::Tray(Value::Array(Vec::new()))).await;
        last_snapshot = None;
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

fn fail(cmd: TrayCommand, why: &str) {
    match cmd {
        TrayCommand::Activate { reply, .. } | TrayCommand::Scroll { reply, .. } | TrayCommand::MenuClick { reply, .. } => {
            let _ = reply.try_send(Err(why.into()));
        }
        TrayCommand::Menu { reply, .. } => {
            let _ = reply.try_send(Err(why.into()));
        }
    }
}

async fn handle(client: &Client, bus: Option<&zbus::Connection>, cmd: TrayCommand) {
    match cmd {
        TrayCommand::Activate { id, x, y, secondary, reply } => {
            let req = if secondary {
                ActivateRequest::Secondary { address: id, x, y }
            } else {
                ActivateRequest::Default { address: id, x, y }
            };
            let r = client.activate(req).await.map_err(|e| e.to_string());
            let _ = reply.try_send(r);
        }
        TrayCommand::Scroll { id, delta, orientation, reply } => {
            let r = match bus {
                Some(bus) => {
                    let orientation = if orientation.starts_with('h') { "horizontal" } else { "vertical" };
                    bus.call_method(
                        Some(id.as_str()),
                        "/StatusNotifierItem",
                        Some("org.kde.StatusNotifierItem"),
                        "Scroll",
                        &(delta, orientation),
                    )
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string())
                }
                None => Err("no session bus".into()),
            };
            let _ = reply.try_send(r);
        }
        TrayCommand::Menu { id, reply } => {
            let menu_path = item_menu_path(client, &id);
            let r = match menu_path {
                Some(path) => {
                    // Let the application refresh the menu before we read it.
                    let _ = tokio::time::timeout(
                        Duration::from_millis(800),
                        client.about_to_show_menuitem(id.clone(), path, 0),
                    )
                    .await;
                    Ok(menu_json(client, &id))
                }
                None => Ok(Value::Array(Vec::new())),
            };
            let _ = reply.try_send(r);
        }
        TrayCommand::MenuClick { id, item, reply } => {
            let r = match item_menu_path(client, &id) {
                Some(menu_path) => client
                    .activate(ActivateRequest::MenuItem { address: id, menu_path, submenu_id: item })
                    .await
                    .map_err(|e| e.to_string()),
                None => Err("item has no menu".into()),
            };
            let _ = reply.try_send(r);
        }
    }
}

fn item_menu_path(client: &Client, id: &str) -> Option<String> {
    let items = client.items();
    let map = items.lock().ok()?;
    map.get(id).and_then(|(item, _)| item.menu.clone())
}

fn menu_json(client: &Client, id: &str) -> Value {
    let items = client.items();
    let Ok(map) = items.lock() else { return Value::Array(Vec::new()) };
    match map.get(id) {
        Some((_, Some(menu))) => menu_items_json(menu),
        _ => Value::Array(Vec::new()),
    }
}

fn menu_items_json(menu: &TrayMenu) -> Value {
    Value::Array(menu.submenus.iter().filter(|m| m.visible).map(menu_item_json).collect())
}

fn menu_item_json(item: &MenuItem) -> Value {
    let is_submenu = item.children_display.as_deref() == Some("submenu") || !item.submenu.is_empty();
    let kind = match item.menu_type {
        MenuType::Separator => "separator",
        MenuType::Standard if is_submenu => "submenu",
        MenuType::Standard => "item",
    };
    let mut v = json!({
        "id": item.id,
        "label": strip_mnemonic(item.label.as_deref().unwrap_or("")),
        "enabled": item.enabled,
        "type": kind,
    });
    match item.toggle_type {
        ToggleType::Checkmark => {
            v["toggle"] = json!("checkmark");
            v["checked"] = json!(item.toggle_state == ToggleState::On);
        }
        ToggleType::Radio => {
            v["toggle"] = json!("radio");
            v["checked"] = json!(item.toggle_state == ToggleState::On);
        }
        ToggleType::CannotBeToggled => {}
    }
    if let Some(name) = item.icon_name.as_deref().filter(|n| !n.is_empty()) {
        v["icon"] = json!(icon_url(name, 16));
    } else if let Some(data) = item.icon_data.as_ref().filter(|d| !d.is_empty()) {
        v["icon"] = json!(format!("data:image/png;base64,{}", gtk4::glib::base64_encode(data)));
    }
    if is_submenu {
        v["children"] = Value::Array(item.submenu.iter().filter(|m| m.visible).map(menu_item_json).collect());
    }
    v
}

/// dbusmenu labels mark mnemonics with a single underscore.
pub fn strip_mnemonic(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '_' {
            if chars.peek() == Some(&'_') {
                chars.next();
                out.push('_');
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn icon_url(name: &str, size: u16) -> String {
    format!("mindos://shell/icon/{}?size={}", icons::percent_encode(name), size)
}

fn publish(
    client: &Client,
    pixmaps: &Pixmaps,
    versions: &mut HashMap<String, u64>,
    icon_theme: &str,
    events: &async_channel::Sender<HostEvent>,
    last: &mut Option<String>,
) {
    let items = client.items();
    let snapshot: Vec<Value> = {
        let Ok(map) = items.lock() else { return };
        let mut list: Vec<(&String, &(StatusNotifierItem, Option<TrayMenu>))> = map.iter().collect();
        list.sort_by(|a, b| a.0.cmp(b.0));
        list.into_iter()
            .map(|(id, (item, menu))| item_json(id, item, menu.is_some(), pixmaps, versions, icon_theme))
            .collect()
    };
    let value = Value::Array(snapshot);
    let text = value.to_string();
    if last.as_deref() == Some(text.as_str()) {
        return;
    }
    *last = Some(text);
    let _ = events.send_blocking(HostEvent::Tray(value));
}

fn item_json(
    id: &str,
    item: &StatusNotifierItem,
    has_menu: bool,
    pixmaps: &Pixmaps,
    versions: &mut HashMap<String, u64>,
    icon_theme: &str,
) -> Value {
    let attention = item.status == Status::NeedsAttention;
    let icon_name = if attention {
        item.attention_icon_name.clone().filter(|n| !n.is_empty()).or_else(|| item.icon_name.clone())
    } else {
        item.icon_name.clone()
    }
    .filter(|n| !n.is_empty());
    let pixmap = if attention {
        item.attention_icon_pixmap.as_ref().or(item.icon_pixmap.as_ref())
    } else {
        item.icon_pixmap.as_ref()
    }
    .and_then(|p| best_pixmap(p, 32));

    // Prefer a themed name when it resolves (crisp SVGs); fall back to the pixmap.
    let mut icon = None;
    if let Some(name) = icon_name.as_deref() {
        if let Some(dir) = item.icon_theme_path.as_deref().filter(|d| !d.is_empty()) {
            if let Some(p) = icons::resolve_in_dir(Path::new(dir), name, 32) {
                icon = Some(format!("mindos://shell/icon/{}", icons::percent_encode(&p.to_string_lossy())));
            }
        }
        if icon.is_none() && icons::resolve(name, 32, icon_theme).is_some() {
            icon = Some(icon_url(name, 32));
        }
    }
    if icon.is_none() {
        if let Some(pix) = pixmap {
            if let Some(png) = encode_png(pix) {
                let version = versions.entry(id.to_string()).or_insert(0);
                let mut changed = true;
                if let Ok(map) = pixmaps.lock() {
                    if let Some((_, old)) = map.get(id) {
                        changed = *old != png;
                    }
                }
                if changed {
                    *version += 1;
                    if let Ok(mut map) = pixmaps.lock() {
                        map.insert(id.to_string(), (*version, png));
                    }
                }
                icon = Some(format!("mindos://shell/tray/{}?v={}", icons::percent_encode(id), version));
            }
        }
    }
    if icon.is_none() {
        if let Some(name) = icon_name.as_deref() {
            icon = Some(icon_url(name, 32));
        }
    }
    let tooltip = item.tool_tip.as_ref().map(|t| {
        if t.description.is_empty() {
            t.title.clone()
        } else if t.title.is_empty() {
            t.description.clone()
        } else {
            format!("{}\n{}", t.title, t.description)
        }
    });
    let title = item
        .title
        .clone()
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| item.id.clone());
    json!({
        "id": id,
        "title": title,
        "tooltip": tooltip.unwrap_or_else(|| title.clone()),
        "icon": icon,
        "status": match item.status {
            Status::Passive => "passive",
            Status::Active => "active",
            Status::NeedsAttention => "attention",
            Status::Unknown => "unknown",
        },
        "hasMenu": has_menu || item.menu.is_some(),
        "isMenu": item.item_is_menu,
        "app": item.id,
    })
}

fn best_pixmap(pixmaps: &[IconPixmap], want: i32) -> Option<&IconPixmap> {
    pixmaps
        .iter()
        .filter(|p| p.width > 0 && p.height > 0 && p.pixels.len() >= (p.width * p.height * 4) as usize)
        .min_by_key(|p| {
            let d = p.width - want;
            // prefer the smallest one that is at least `want`, then the largest below
            if d >= 0 { d } else { 1000 - d }
        })
}

/// SNI pixmaps are ARGB32 in network byte order; PNG wants RGBA.
pub fn encode_png(p: &IconPixmap) -> Option<Vec<u8>> {
    let (w, h) = (p.width as u32, p.height as u32);
    let n = (w * h) as usize;
    let mut rgba = Vec::with_capacity(n * 4);
    for px in p.pixels.chunks_exact(4).take(n) {
        rgba.extend_from_slice(&[px[1], px[2], px[3], px[0]]);
    }
    encode_rgba_png(w, h, &rgba)
}

/// Straight-alpha RGBA (what the compositor sends for XEmbed icons) to PNG.
pub fn encode_rgba_png(w: u32, h: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    if w == 0 || h == 0 || rgba.len() < (w * h * 4) as usize {
        return None;
    }
    let rgba = &rgba[..(w * h * 4) as usize];
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().ok()?;
        writer.write_image_data(&rgba).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mnemonics() {
        assert_eq!(strip_mnemonic("_Quit"), "Quit");
        assert_eq!(strip_mnemonic("Save __as_"), "Save _as");
    }

    #[test]
    fn png_roundtrip() {
        let pix = IconPixmap { width: 2, height: 1, pixels: vec![255, 1, 2, 3, 128, 4, 5, 6] };
        let png = encode_png(&pix).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        let decoder = png::Decoder::new(std::io::Cursor::new(png));
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height), (2, 1));
        assert_eq!(&buf[..8], &[1, 2, 3, 255, 4, 5, 6, 128]);
    }
}
