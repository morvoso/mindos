//! Session-local forwarding: system applications open in the desktop panel.
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;
use serde_json::Value;
use crate::HostEvent;

fn path() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let display = std::env::var("WAYLAND_DISPLAY").ok()?;
    Some(PathBuf::from(runtime).join(format!("mindshell-workspace-{}.sock", display.replace('/', "_"))))
}
pub fn forward(value: &Value) -> bool {
    let Some(path) = path() else { return false };
    let Ok(mut stream) = UnixStream::connect(path) else { return false };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    if writeln!(stream, "{value}").is_err() { return false; }
    let mut ack = [0; 2];
    stream.read_exact(&mut ack).is_ok() && ack == *b"ok"
}
pub fn start(events: async_channel::Sender<HostEvent>) {
    let Some(path) = path() else { return };
    if UnixStream::connect(&path).is_ok() { return; }
    let _ = std::fs::remove_file(&path);
    let Ok(listener) = UnixListener::bind(&path) else { return };
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut line = String::new();
            if BufReader::new((&stream).take(16384)).read_line(&mut line).is_err() { continue; }
            let Ok(value) = serde_json::from_str::<Value>(&line) else { continue };
            if !matches!(value["name"].as_str(), Some("settings" | "gaming" | "library")) { continue; }
            if events.send_blocking(HostEvent::DesktopOpen(value)).is_ok() { let _ = stream.write_all(b"ok"); }
        }
    });
}
